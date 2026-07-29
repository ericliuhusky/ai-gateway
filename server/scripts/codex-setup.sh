#!/bin/sh
set -eu

umask 077

fail() {
  printf 'ai-gateway: %s\n' "$*" >&2
  exit 1
}

gateway_base_url=${1:-}
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

[ -n "${HOME:-}" ] || fail "缺少 HOME 环境变量"

codex_dir=${CODEX_HOME:-"$HOME/.codex"}
data_dir="$HOME/.ai-gateway"
config_path="$codex_dir/config.toml"
backup_path="$data_dir/codex-config.before-ai-gateway.toml"
absent_marker="$data_dir/codex-config.before-ai-gateway.absent"
lock_dir="$data_dir/codex-config-script.lock"
temp_path=

mkdir -p "$codex_dir" "$data_dir"
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

if [ ! -f "$backup_path" ] && [ ! -f "$absent_marker" ]; then
  if [ -f "$config_path" ]; then
    cp "$config_path" "$backup_path"
  else
    : > "$absent_marker"
  fi
fi

temp_path="$codex_dir/.config.toml.ai-gateway.$$"
source_path=$config_path
if [ ! -f "$source_path" ]; then
  source_path=/dev/null
fi

awk '
BEGIN {
  in_root = 1
  skipping_gateway = 0
  inserted_provider = 0
}

function insert_provider() {
  if (!inserted_provider) {
    print "model_provider = \"ai-gateway\""
    inserted_provider = 1
  }
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

  if (in_root && line ~ /^[[:space:]]*\[/) {
    insert_provider()
    in_root = 0
  }

  if (in_root && line ~ /^[[:space:]]*model_provider[[:space:]]*=/) {
    next
  }

  print line
}

END {
  insert_provider()
}
' "$source_path" > "$temp_path"

cat >> "$temp_path" <<EOF

[model_providers.ai-gateway]
name = "ai-gateway"
base_url = "$gateway_base_url"
wire_api = "responses"
EOF

chmod 600 "$temp_path"
mv "$temp_path" "$config_path"
temp_path=

printf '%s\n' \
  "AI Gateway 已写入 $config_path" \
  "Gateway: $gateway_base_url" \
  "请重新启动 Codex 或新建任务使配置生效。"
