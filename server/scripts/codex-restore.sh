#!/bin/sh
set -eu

umask 077

fail() {
  printf 'ai-gateway: %s\n' "$*" >&2
  exit 1
}

[ -n "${HOME:-}" ] || fail "缺少 HOME 环境变量"

codex_dir="$HOME/.codex"
config_path="$codex_dir/config.toml"
auth_path="$codex_dir/auth.json"
auth_backup_path="$codex_dir/.ai-gateway-auth.before-setup.json"
auth_absent_marker="$codex_dir/.ai-gateway-auth.was-absent"
lock_dir="$codex_dir/.ai-gateway-config.lock"
temp_path=

[ -f "$config_path" ] || fail "Codex 配置不存在：$config_path"

if ! mkdir "$lock_dir" 2>/dev/null; then
  fail "另一个 Codex 配置脚本正在运行"
fi

cleanup() {
  if [ -n "$temp_path" ]; then
    rm -f "$temp_path"
  fi
  rmdir "$lock_dir" 2>/dev/null || true
}
trap cleanup 0 1 2 3 15

temp_path="$codex_dir/.config.toml.ai-gateway-restore.$$"

if ! awk '
BEGIN {
  marker_prefix = "# ai-gateway.previous-model-provider: "
  marker_seen = 0
  previous_provider_line = ""
  in_root = 1
  root_count = 0
  root_flushed = 0
  skipping_gateway = 0
}

function flush_root(    i) {
  if (root_flushed) {
    return
  }

  if (!marker_seen) {
    exit 42
  }

  if (previous_provider_line != "<absent>") {
    print previous_provider_line
  }

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
        if (!marker_seen) {
          previous_provider_line = substr(line, length(marker_prefix) + 1)
          marker_seen = 1
        }
        next
      }

      if (line ~ /^[[:space:]]*model_provider[[:space:]]*=/) {
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
' "$config_path" > "$temp_path"; then
  fail "没有找到 AI Gateway 保存的原模型供应商"
fi

chmod 600 "$temp_path"
mv "$temp_path" "$config_path"
temp_path=

if [ -f "$auth_backup_path" ]; then
  mv "$auth_backup_path" "$auth_path"
  rm -f "$auth_absent_marker"
elif [ -f "$auth_absent_marker" ]; then
  rm -f "$auth_path" "$auth_absent_marker"
fi

printf '%s\n' \
  "已清理 AI Gateway 的 Codex 配置，并恢复原模型供应商。" \
  "已移除 ai-gateway Provider 配置和切换标记。" \
  "已恢复切换前的 Codex 登录凭据。" \
  "请重新启动 Codex 或新建任务使配置生效。"
