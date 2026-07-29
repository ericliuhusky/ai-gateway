#!/bin/sh
set -eu

umask 077

fail() {
  printf 'ai-gateway: %s\n' "$*" >&2
  exit 1
}

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
  fail "没有可恢复的 Codex 配置备份"
fi

if [ -f "$config_path" ]; then
  timestamp=$(date '+%Y%m%d-%H%M%S')
  safety_path="$data_dir/codex-config.before-restore.$timestamp.toml"
  cp "$config_path" "$safety_path"
  printf '当前配置已额外备份到 %s\n' "$safety_path"
fi

if [ -f "$backup_path" ]; then
  temp_path="$codex_dir/.config.toml.ai-gateway-restore.$$"
  cp "$backup_path" "$temp_path"
  chmod 600 "$temp_path"
  mv "$temp_path" "$config_path"
  temp_path=
  rm -f "$backup_path" "$absent_marker"
  printf 'Codex 配置已恢复到接入 AI Gateway 之前的状态。\n'
else
  rm -f "$config_path" "$absent_marker"
  printf '接入前不存在 Codex 配置，已移除脚本创建的配置文件。\n'
fi

printf '请重新启动 Codex 或新建任务使配置生效。\n'
