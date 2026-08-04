#!/bin/sh
set -eu

umask 077

fail() {
  printf 'ai-gateway: %s\n' "$*" >&2
  exit 1
}

warn() {
  printf 'ai-gateway: 警告：%s\n' "$*" >&2
}

sql_escape() {
  awk -v value="$1" 'BEGIN {
    gsub(/\047/, "\047\047", value)
    printf "%s", value
  }'
}

detect_previous_provider() {
  awk '
  BEGIN {
    marker_prefix = "# ai-gateway.previous-model-provider: "
    in_root = 1
    found = 0
  }

  function emit_provider(line,    value) {
    if (line !~ /^[[:space:]]*model_provider[[:space:]]*=[[:space:]]*"[A-Za-z0-9._-]+"/) {
      return
    }

    value = line
    sub(/^[[:space:]]*model_provider[[:space:]]*=[[:space:]]*"/, "", value)
    sub(/".*$/, "", value)
    print value
    found = 1
    exit
  }

  {
    if (!in_root) {
      next
    }

    if ($0 ~ /^[[:space:]]*\[/) {
      in_root = 0
      next
    }

    if (index($0, marker_prefix) == 1) {
      saved = substr($0, length(marker_prefix) + 1)
      if (saved == "<absent>") {
        print "openai"
        found = 1
        exit
      }
      emit_provider(saved)
    }

    emit_provider($0)
  }

  END {
    if (!found) {
      print "openai"
    }
  }
  ' "$1"
}

rewrite_rollout() {
  source_rollout=$1
  target_rollout=$2
  source_id=$3
  alias_id=$4
  source_provider=$5
  target_provider=$6
  rollout_temp="$target_rollout.tmp.$$"

  if ! awk \
    -v source_id="$source_id" \
    -v alias_id="$alias_id" \
    -v source_provider="$source_provider" \
    -v target_provider="$target_provider" '
  function replace_all(value, needle, replacement, kind,    output, position) {
    output = ""
    while ((position = index(value, needle)) > 0) {
      output = output substr(value, 1, position - 1) replacement
      value = substr(value, position + length(needle))
      if (kind == "id") {
        id_replacements++
      } else {
        provider_replacements++
      }
    }
    return output value
  }

  {
    line = replace_all($0, "\"" source_id "\"", "\"" alias_id "\"", "id")
    line = replace_all(line, "\"model_provider\":\"" source_provider "\"", "\"model_provider\":\"" target_provider "\"", "provider")
    print line
  }

  END {
    if (id_replacements == 0 || provider_replacements == 0) {
      exit 42
    }
  }
  ' "$source_rollout" > "$rollout_temp"; then
    rm -f "$rollout_temp"
    rollout_temp=
    return 1
  fi

  chmod 600 "$rollout_temp"
  mv "$rollout_temp" "$target_rollout"
  rollout_temp=
}

sync_history_aliases() {
  source_provider=$1
  target_provider=ai-gateway
  state_path="$codex_dir/state_5.sqlite"
  history_dir="$codex_dir/.ai-gateway-history"
  mapping_path="$history_dir/aliases.tsv"
  backup_path="$history_dir/state_5.before-first-sync.sqlite"
  backup_temp="$history_dir/state_5.before-first-sync.sqlite.$$"
  source_list="$history_dir/source-threads.$$"
  pending_list="$history_dir/pending-threads.$$"
  columns_list="$history_dir/thread-columns.$$"
  mapping_temp="$history_dir/aliases.tsv.$$"
  sql_path="$history_dir/insert-aliases.$$"

  if [ "$source_provider" = "$target_provider" ]; then
    return 0
  fi

  if ! command -v sqlite3 >/dev/null 2>&1; then
    warn "未找到 sqlite3，已跳过 Codex 历史同步"
    return 0
  fi

  if [ ! -f "$state_path" ]; then
    warn "未找到 ${state_path}，已跳过 Codex 历史同步"
    return 0
  fi

  case "$source_provider" in
    *[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-]*)
      warn "原模型供应商名称无法安全处理，已跳过 Codex 历史同步"
      return 0
      ;;
  esac

  mkdir -p "$history_dir"
  chmod 700 "$history_dir"
  : > "$pending_list"

  if [ -f "$mapping_path" ]; then
    cp "$mapping_path" "$mapping_temp"
  else
    printf '# source_provider\tsource_id\talias_id\talias_rollout_path\n' > "$mapping_temp"
  fi

  source_provider_sql=$(sql_escape "$source_provider")
  target_provider_sql=$(sql_escape "$target_provider")

  if [ "$(sqlite3 "$state_path" "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'threads';" 2>/dev/null || printf 0)" != 1 ]; then
    warn "Codex state 数据库缺少 threads 表，已跳过历史同步"
    return 0
  fi

  if ! sqlite3 -separator "$(printf '\t')" "$state_path" \
    "SELECT id, rollout_path, archived
     FROM threads
     WHERE model_provider = '$source_provider_sql'
     ORDER BY created_at, id;" > "$source_list"; then
    warn "无法读取 Codex 历史数据库，已跳过历史同步"
    return 0
  fi

  source_count=$(awk 'END { print NR + 0 }' "$source_list")
  if [ "$source_count" -eq 0 ]; then
    rm -f "$mapping_temp"
    printf '未发现需要同步的 %s 历史任务。\n' "$source_provider"
    return 0
  fi

  if [ ! -f "$backup_path" ]; then
    rm -f "$backup_temp"
    backup_temp_sql=$(sql_escape "$backup_temp")
    if ! sqlite3 "$state_path" "PRAGMA busy_timeout = 10000; VACUUM INTO '$backup_temp_sql';" >/dev/null; then
      warn "无法创建 Codex state 安全备份，已跳过历史同步"
      return 0
    fi
    chmod 600 "$backup_temp"
    mv "$backup_temp" "$backup_path"
  fi

  if ! sqlite3 "$state_path" \
    "SELECT name FROM pragma_table_info('threads') ORDER BY cid;" > "$columns_list"; then
    warn "无法读取 Codex threads 表结构，已跳过历史同步"
    return 0
  fi

  for required_column in id rollout_path model_provider; do
    if ! awk -v required="$required_column" '$0 == required { found = 1 } END { exit !found }' "$columns_list"; then
      warn "Codex threads 表缺少 $required_column 字段，已跳过历史同步"
      return 0
    fi
  done

  created=0
  repaired=0
  existing=0
  skipped=0
  mapping_changed=0

  while IFS="$(printf '\t')" read -r source_id source_rollout archived; do
    [ -n "$source_id" ] || continue

    case "$source_id" in
      *[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-]*)
        warn "任务 ID $source_id 包含不支持的字符，已跳过"
        skipped=$((skipped + 1))
        continue
        ;;
    esac

    mapping_row=$(awk -F '\t' \
      -v provider="$source_provider" \
      -v source_id="$source_id" \
      '$1 == provider && $2 == source_id { print $3 "\t" $4; exit }' \
      "$mapping_temp")

    alias_id=
    alias_rollout=
    if [ -n "$mapping_row" ]; then
      alias_id=${mapping_row%%"$(printf '\t')"*}
      alias_rollout=${mapping_row#*"$(printf '\t')"}
    fi

    if [ -z "$alias_id" ]; then
      if ! alias_id=$(sqlite3 ':memory:' \
        "SELECT lower(
           hex(randomblob(4)) || '-' ||
           hex(randomblob(2)) || '-4' ||
           substr(hex(randomblob(2)), 2) || '-' ||
           substr('89ab', abs(random()) % 4 + 1, 1) ||
           substr(hex(randomblob(2)), 2) || '-' ||
           hex(randomblob(6))
         );"); then
        warn "无法生成历史 alias ID，已跳过任务 $source_id"
        skipped=$((skipped + 1))
        continue
      fi

      source_file=${source_rollout##*/}
      case "$source_file" in
        rollout-*.jsonl) ;;
        *)
          warn "任务 $source_id 的 rollout 文件名无法识别，已跳过"
          skipped=$((skipped + 1))
          continue
          ;;
      esac

      alias_file=$(awk -v source_id="$source_id" -v alias_id="$alias_id" '
        {
          position = index($0, source_id)
          if (position == 0) {
            exit 42
          }
          print substr($0, 1, position - 1) alias_id substr($0, position + length(source_id))
        }
      ' <<EOF
$source_file
EOF
      ) || alias_file=

      if [ -z "$alias_file" ]; then
        warn "任务 $source_id 的 rollout 文件名与任务 ID 不匹配，已跳过"
        skipped=$((skipped + 1))
        continue
      fi

      if [ "$archived" = 1 ]; then
        alias_dir="$codex_dir/archived_sessions"
      else
        rollout_date=${source_file#rollout-}
        rollout_date=${rollout_date%%T*}
        if ! printf '%s\n' "$rollout_date" | awk '
          /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/ { found = 1 }
          END { exit !found }
        '; then
          warn "任务 $source_id 的 rollout 日期无法识别，已跳过"
          skipped=$((skipped + 1))
          continue
        fi
        year=${rollout_date%%-*}
        month_day=${rollout_date#*-}
        month=${month_day%%-*}
        day=${month_day#*-}
        alias_dir="$codex_dir/sessions/$year/$month/$day"
      fi

      alias_rollout="$alias_dir/$alias_file"
      printf '%s\t%s\t%s\t%s\n' \
        "$source_provider" "$source_id" "$alias_id" "$alias_rollout" >> "$mapping_temp"
      mapping_changed=1
    else
      case "$alias_id" in
        *[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-]*)
          warn "任务 $source_id 的历史映射已损坏，已跳过"
          skipped=$((skipped + 1))
          continue
          ;;
      esac
      case "$alias_rollout" in
        "$codex_dir"/sessions/*|"$codex_dir"/archived_sessions/*) ;;
        *)
          warn "任务 $source_id 的历史映射路径不安全，已跳过"
          skipped=$((skipped + 1))
          continue
          ;;
      esac
    fi

    alias_id_sql=$(sql_escape "$alias_id")
    if ! alias_exists=$(sqlite3 "$state_path" \
      "SELECT count(*) FROM threads
       WHERE id = '$alias_id_sql' AND model_provider = '$target_provider_sql';"); then
      warn "无法检查任务 $source_id 的历史 alias，已跳过"
      skipped=$((skipped + 1))
      continue
    fi

    if [ ! -f "$alias_rollout" ]; then
      if [ ! -f "$source_rollout" ]; then
        warn "任务 $source_id 的 rollout 文件不存在，已跳过"
        skipped=$((skipped + 1))
        continue
      fi

      mkdir -p "${alias_rollout%/*}"
      if ! rewrite_rollout \
        "$source_rollout" \
        "$alias_rollout" \
        "$source_id" \
        "$alias_id" \
        "$source_provider" \
        "$target_provider"; then
        warn "任务 $source_id 的 rollout 内容无法安全转换，已跳过"
        skipped=$((skipped + 1))
        continue
      fi

      if [ "$alias_exists" -eq 1 ]; then
        repaired=$((repaired + 1))
      fi
    fi

    if [ "$alias_exists" -eq 1 ]; then
      existing=$((existing + 1))
      continue
    fi

    printf '%s\t%s\t%s\n' "$source_id" "$alias_id" "$alias_rollout" >> "$pending_list"
    created=$((created + 1))
  done < "$source_list"

  if [ "$mapping_changed" -eq 1 ]; then
    chmod 600 "$mapping_temp"
    mv "$mapping_temp" "$mapping_path"
  else
    rm -f "$mapping_temp"
  fi

  if [ "$created" -gt 0 ]; then
    column_list=
    while IFS= read -r column; do
      case "$column" in
        *[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_]*)
          warn "Codex threads 表包含无法安全处理的字段名，已跳过数据库写入"
          return 0
          ;;
      esac
      if [ -n "$column_list" ]; then
        column_list="$column_list, "
      fi
      column_list="$column_list\"$column\""
    done < "$columns_list"

    {
      printf '.timeout 10000\n'
      printf 'BEGIN IMMEDIATE;\n'
      while IFS="$(printf '\t')" read -r source_id alias_id alias_rollout; do
        source_id_sql=$(sql_escape "$source_id")
        alias_id_sql=$(sql_escape "$alias_id")
        alias_rollout_sql=$(sql_escape "$alias_rollout")
        select_list=

        while IFS= read -r column; do
          case "$column" in
            id) expression="'$alias_id_sql'" ;;
            rollout_path) expression="'$alias_rollout_sql'" ;;
            model_provider) expression="'$target_provider_sql'" ;;
            *) expression="\"$column\"" ;;
          esac
          if [ -n "$select_list" ]; then
            select_list="$select_list, "
          fi
          select_list="$select_list$expression"
        done < "$columns_list"

        printf 'INSERT INTO threads (%s) SELECT %s FROM threads WHERE id = '\''%s'\'' AND model_provider = '\''%s'\'';\n' \
          "$column_list" "$select_list" "$source_id_sql" "$source_provider_sql"
      done < "$pending_list"
      printf 'COMMIT;\n'
    } > "$sql_path"

    if ! sqlite3 "$state_path" < "$sql_path"; then
      warn "Codex 历史数据库写入失败；映射和 rollout 已保留，可稍后重新执行接入脚本重试"
      return 0
    fi

    missing_aliases=0
    while IFS="$(printf '\t')" read -r source_id alias_id alias_rollout; do
      alias_id_sql=$(sql_escape "$alias_id")
      if [ "$(sqlite3 "$state_path" "SELECT count(*) FROM threads WHERE id = '$alias_id_sql' AND model_provider = '$target_provider_sql';" 2>/dev/null || printf 0)" != 1 ]; then
        missing_aliases=$((missing_aliases + 1))
      fi
    done < "$pending_list"
    if [ "$missing_aliases" -gt 0 ]; then
      warn "$missing_aliases 条历史 alias 未写入数据库，可稍后重新执行接入脚本重试"
    fi
  fi

  printf '%s\n' \
    "Codex 历史同步完成：来源 $source_count 条，新建 $created 条，已有 $existing 条，修复 $repaired 条，跳过 $skipped 条。"
}

gateway_base_url=${1:-}
gateway_access_token=${2:-}
[ -n "$gateway_base_url" ] || fail "缺少 Gateway Base URL"

case "$gateway_base_url" in
  http://*|https://*) ;;
  *) fail "Gateway Base URL 必须使用 http:// 或 https://" ;;
esac

case "$gateway_base_url" in
  *\"*|*\\*) fail "Gateway Base URL 包含不支持的字符" ;;
esac

newline='
'
case "$gateway_base_url" in
  *"$newline"*) fail "Gateway Base URL 包含不支持的字符" ;;
esac

while [ "${gateway_base_url%/}" != "$gateway_base_url" ]; do
  gateway_base_url=${gateway_base_url%/}
done

case "$gateway_access_token" in
  ""|agw_*) ;;
  *) fail "Gateway API Key 格式无效" ;;
esac
case "$gateway_access_token" in
  *\"*|*\\*|*"$newline"*) fail "Gateway API Key 包含不支持的字符" ;;
esac

[ -n "${HOME:-}" ] || fail "缺少 HOME 环境变量"

codex_dir="$HOME/.codex"
config_path="$codex_dir/config.toml"
auth_path="$codex_dir/auth.json"
auth_backup_path="$codex_dir/.ai-gateway-auth.before-setup.json"
auth_absent_marker="$codex_dir/.ai-gateway-auth.was-absent"
lock_dir="$codex_dir/.ai-gateway-config.lock"
temp_path=
auth_temp=
history_dir=
backup_temp=
rollout_temp=
source_list=
pending_list=
columns_list=
mapping_temp=
sql_path=

mkdir -p "$codex_dir"
if ! mkdir "$lock_dir" 2>/dev/null; then
  fail "另一个 Codex 配置脚本正在运行"
fi

cleanup() {
  if [ -n "$temp_path" ]; then
    rm -f "$temp_path"
  fi
  if [ -n "$auth_temp" ]; then
    rm -f "$auth_temp"
  fi
  for history_temp in "$backup_temp" "$rollout_temp" "$source_list" "$pending_list" "$columns_list" "$mapping_temp" "$sql_path"; do
    if [ -n "$history_temp" ]; then
      rm -f "$history_temp"
    fi
  done
  rmdir "$lock_dir" 2>/dev/null || true
}
trap cleanup 0 1 2 3 15

temp_path="$codex_dir/.config.toml.ai-gateway.$$"
source_path=$config_path
if [ ! -f "$source_path" ]; then
  source_path=/dev/null
fi
previous_provider=$(detect_previous_provider "$source_path")

awk '
BEGIN {
  marker_prefix = "# ai-gateway.previous-model-provider: "
  in_root = 1
  root_count = 0
  root_flushed = 0
  skipping_gateway = 0
  previous_provider_line = ""
  existing_marker = ""
}

function flush_root(    i) {
  if (root_flushed) {
    return
  }

  if (existing_marker != "") {
    print existing_marker
  } else if (previous_provider_line != "") {
    print marker_prefix previous_provider_line
  } else {
    print marker_prefix "<absent>"
  }

  print "model_provider = \"ai-gateway\""

  for (i = 1; i <= root_count; i++) {
    print root_lines[i]
  }

  root_flushed = 1
}

{
  line = $0

  if (skipping_gateway) {
    if (line ~ /^[[:space:]]*\[/) {
      skipping_gateway = 0
    } else {
      next
    }
  }

  if (line ~ /^[[:space:]]*\[model_providers[[:space:]]*\.[[:space:]]*ai-gateway[[:space:]]*\][[:space:]]*($|#)/) {
    skipping_gateway = 1
    next
  }

  if (in_root) {
    if (line ~ /^[[:space:]]*\[/) {
      flush_root()
      in_root = 0
    } else {
      if (index(line, marker_prefix) == 1) {
        if (existing_marker == "") {
          existing_marker = line
        }
        next
      }

      if (line ~ /^[[:space:]]*model_provider[[:space:]]*=/) {
        if (previous_provider_line == "") {
          previous_provider_line = line
        }
        next
      }

      root_lines[++root_count] = line
      next
    }
  }

  print line
}

END {
  if (in_root) {
    flush_root()
  }
}
' "$source_path" > "$temp_path"

cat >> "$temp_path" <<EOF

[model_providers.ai-gateway]
name = "ai-gateway"
base_url = "$gateway_base_url"
wire_api = "responses"
EOF

if [ -n "$gateway_access_token" ]; then
  if [ ! -f "$auth_backup_path" ] && [ ! -f "$auth_absent_marker" ]; then
    if [ -f "$auth_path" ]; then
      cp "$auth_path" "$auth_backup_path"
      chmod 600 "$auth_backup_path"
    else
      : > "$auth_absent_marker"
      chmod 600 "$auth_absent_marker"
    fi
  fi

  auth_temp="$codex_dir/.auth.json.ai-gateway.$$"
  printf '{\n  "OPENAI_API_KEY": "%s"\n}\n' "$gateway_access_token" > "$auth_temp"
  chmod 600 "$auth_temp"
fi

chmod 600 "$temp_path"
mv "$temp_path" "$config_path"
temp_path=
if [ -n "$auth_temp" ]; then
  mv "$auth_temp" "$auth_path"
  auth_temp=
fi

if ! sync_history_aliases "$previous_provider"; then
  warn "Codex 历史同步未完成，可稍后重新执行接入脚本重试"
fi

printf '%s\n' \
  "AI Gateway 已写入 $config_path" \
  "Gateway: $gateway_base_url" \
  "原模型供应商已记录在 config.toml 注释中。" \
  "请重新启动 Codex 或新建任务使配置生效。"
