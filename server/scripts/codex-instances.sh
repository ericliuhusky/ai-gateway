#!/bin/sh
set -eu

# Manage isolated Codex desktop profiles for AI Gateway.
#
# Each profile has its own CODEX_HOME (config.toml, auth.json, sessions) and
# Electron user-data directory. The global ~/.codex directory is only used as
# an optional config template; auth.json is intentionally never copied.

umask 077

fail() {
  printf 'ai-gateway instances: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  instances.sh create <name> <gateway-base-url> [gateway-api-key]
  instances.sh start <name>
  instances.sh delete <name>
  instances.sh list
  instances.sh path <name>

Environment:
  AI_GATEWAY_CODEX_INSTANCES_DIR  Profile root
                                  (default: ~/.ai-gateway/codex-instances)
  AI_GATEWAY_CODEX_TEMPLATE_HOME  Config template home
                                  (default: ~/.codex)
  CODEX_APP_PATH                  macOS app name/path passed to `open -a`
                                  (default: ChatGPT)

`create` creates and starts an isolated Codex window. It copies only the
template config.toml, rewrites it for AI Gateway, and never copies auth.json.
Sign in separately in each new window when you need different Codex accounts.
EOF
}

require_instance_name() {
  case "$1" in
    ""|.*|*/*|*[!A-Za-z0-9_-]*)
      fail "实例名称只能包含字母、数字、_ 或 -，且不能以 . 开头：$1"
      ;;
  esac
}

require_gateway_url() {
  case "$1" in
    http://*|https://*) ;;
    *) fail "Gateway Base URL 必须使用 http:// 或 https://" ;;
  esac

  case "$1" in
    *\"*|*\\*) fail "Gateway Base URL 包含不支持的字符" ;;
  esac

  newline='
'
  case "$1" in
    *"$newline"*) fail "Gateway Base URL 包含不支持的字符" ;;
  esac
}

trim_trailing_slashes() {
  value=$1
  while [ "${value%/}" != "$value" ]; do
    value=${value%/}
  done
  printf '%s' "$value"
}

link_shared_path() {
  source_path=$1
  target_path=$2

  [ -e "$source_path" ] || [ -L "$source_path" ] || return 0
  [ ! -e "$target_path" ] && [ ! -L "$target_path" ] || return 0
  ln -s "$source_path" "$target_path"
}

configure_gateway() {
  config_path=$1
  gateway_base_url=$2
  codex_home=$3
  gateway_access_token=$4
  temp_path="${config_path}.ai-gateway.$$"
  source_path=$config_path

  if [ ! -f "$source_path" ]; then
    source_path=/dev/null
  fi

  awk -v instance_home="$codex_home" '
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

  if (line ~ /^[[:space:]]*CODEX_HOME[[:space:]]*=/) {
    match(line, /^[[:space:]]*/)
    print substr(line, 1, RLENGTH) "CODEX_HOME = \"" instance_home "\""
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
    printf 'bearer_token_env_var = "%s"\n' "$gateway_access_token" >> "$temp_path"
  fi

  chmod 600 "$temp_path"
  mv "$temp_path" "$config_path"
}

instances_root=${AI_GATEWAY_CODEX_INSTANCES_DIR:-"$HOME/.ai-gateway/codex-instances"}
template_home=${AI_GATEWAY_CODEX_TEMPLATE_HOME:-"$HOME/.codex"}

instance_dir() {
  printf '%s/%s' "$instances_root" "$1"
}

start_instance() {
  name=$1
  require_instance_name "$name"
  root=$(instance_dir "$name")
  codex_home="$root/codex-home"
  electron_home="$root/electron"

  [ -d "$codex_home" ] || fail "实例不存在：$name"
  mkdir -p "$electron_home"

  case "$(uname -s)" in
    Darwin)
      app_path=${CODEX_APP_PATH:-ChatGPT}
      open -n -a "$app_path" \
        --env "CODEX_HOME=$codex_home" \
        --env "CODEX_ELECTRON_USER_DATA_PATH=$electron_home" \
        --args "--user-data-dir=$electron_home"
      ;;
    *)
      fail "当前多实例启动脚本仅支持 macOS；实例配置已保存在：$codex_home"
      ;;
  esac

  printf '%s\n' \
    "已启动 Codex 实例：$name" \
    "CODEX_HOME: $codex_home" \
    "Electron 数据目录: $electron_home"
}

create_instance() {
  name=$1
  gateway_base_url=$2
  gateway_access_token=${3:-}
  require_instance_name "$name"
  require_gateway_url "$gateway_base_url"
  case "$gateway_access_token" in
    ""|agw_*) ;;
    *) fail "Gateway API Key 格式无效" ;;
  esac
  newline='
'
  case "$gateway_access_token" in
    *\"*|*\\*|*"$newline"*) fail "Gateway API Key 包含不支持的字符" ;;
  esac
  gateway_base_url=$(trim_trailing_slashes "$gateway_base_url")

  root=$(instance_dir "$name")
  codex_home="$root/codex-home"
  config_path="$codex_home/config.toml"
  [ ! -e "$root" ] && [ ! -L "$root" ] || fail "实例已存在：$name"

  mkdir -p "$codex_home" "$root/electron"
  if [ -f "$template_home/config.toml" ]; then
    cp "$template_home/config.toml" "$config_path"
  fi

  # Reuse skills/rules without sharing auth, config, session, or Electron data.
  link_shared_path "$template_home/skills" "$codex_home/skills"
  link_shared_path "$template_home/rules" "$codex_home/rules"
  link_shared_path "$template_home/AGENTS.md" "$codex_home/AGENTS.md"

  configure_gateway "$config_path" "$gateway_base_url" "$codex_home" "$gateway_access_token"
  printf '%s\n' \
    "已创建隔离实例：$name" \
    "未复制 auth.json；请在新窗口中单独登录所需账号。"
  start_instance "$name"
}

delete_instance() {
  name=$1
  require_instance_name "$name"
  root=$(instance_dir "$name")

  [ -e "$root" ] || [ -L "$root" ] || fail "实例不存在：$name"
  rm -rf "$root"
  printf '%s\n' "已删除 Codex 实例及其本地配置：$name"
}

list_instances() {
  [ -d "$instances_root" ] || {
    printf '%s\n' "尚未创建实例。"
    return 0
  }

  found=0
  for root in "$instances_root"/*; do
    [ -d "$root" ] || continue
    name=$(basename "$root")
    config="$root/codex-home/config.toml"
    auth="$root/codex-home/auth.json"
    [ -f "$config" ] || continue
    found=1
    if [ -f "$auth" ]; then
      auth_state=已登录
    else
      auth_state=未登录
    fi
    printf '%s\t%s\t%s\n' "$name" "$auth_state" "$root/codex-home"
  done

  [ "$found" -eq 1 ] || printf '%s\n' "尚未创建实例。"
}

command=${1:-}
case "$command" in
  create)
    [ "$#" -ge 3 ] && [ "$#" -le 4 ] || {
      usage
      exit 1
    }
    create_instance "$2" "$3" "${4:-}"
    ;;
  start)
    [ "$#" -eq 2 ] || {
      usage
      exit 1
    }
    start_instance "$2"
    ;;
  delete)
    [ "$#" -eq 2 ] || {
      usage
      exit 1
    }
    delete_instance "$2"
    ;;
  list)
    [ "$#" -eq 1 ] || {
      usage
      exit 1
    }
    list_instances
    ;;
  path)
    [ "$#" -eq 2 ] || {
      usage
      exit 1
    }
    require_instance_name "$2"
    printf '%s\n' "$(instance_dir "$2")/codex-home"
    ;;
  -h|--help|help|"")
    usage
    ;;
  *)
    fail "未知命令：$command"
    ;;
esac
