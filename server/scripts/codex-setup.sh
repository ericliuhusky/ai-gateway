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
config_path="$codex_dir/config.toml"
lock_dir="$codex_dir/.ai-gateway-config.lock"
temp_path=

mkdir -p "$codex_dir"
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

temp_path="$codex_dir/.config.toml.ai-gateway.$$"
source_path=$config_path
if [ ! -f "$source_path" ]; then
  source_path=/dev/null
fi

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

chmod 600 "$temp_path"
mv "$temp_path" "$config_path"
temp_path=

printf '%s\n' \
  "AI Gateway 已写入 $config_path" \
  "Gateway: $gateway_base_url" \
  "原模型供应商已记录在 config.toml 注释中。" \
  "请重新启动 Codex 或新建任务使配置生效。"
