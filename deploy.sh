#!/usr/bin/env bash
set -euo pipefail

# Builds the server and Web console locally, then deploys only the resulting
# artifacts to the remote macOS host. Persistent server data is left untouched.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_PKG="${SERVER_PKG:-ai-gateway}"
ARTIFACT_DIR="${ARTIFACT_DIR:-}"

# Override these when deploying to a different host.
DEPLOY_HOST="${DEPLOY_HOST:-10.10.57.55}"
DEPLOY_USER="${DEPLOY_USER:-ninebot}"
SSH_PORT="${SSH_PORT:-22}"
DEPLOY_SERVICE_NAME="${DEPLOY_SERVICE_NAME:-ericliu.husky.ai-gateway}"

# The gateway itself always reads its data and Web files from
# $HOME/.ai-gateway, so the remote runtime directory must remain fixed.
REMOTE_APP_DIR=".ai-gateway"
GATEWAY_PORT="4242"
SKIP_BUILD="0"

usage() {
  cat <<'USAGE'
用法:
  ./deploy.sh [--skip-build]

说明:
  本地构建 ai-gateway 二进制和 Web 管理端，只将构建产物同步至远端
  ~/.ai-gateway/bin 和 ~/.ai-gateway/web，随后安装/更新 LaunchAgent 并重启服务。
  远端已有的 SQLite 数据库、日志和其他持久化文件不会被删除。

默认远端配置:
  主机: 10.10.57.55
  用户: ninebot
  SSH 端口: 22
  LaunchAgent Label: ericliu.husky.ai-gateway
  服务地址: 0.0.0.0:4242

可选环境变量:
  DEPLOY_HOST          远端主机
  DEPLOY_USER          远端 SSH 用户
  SSH_PORT             远端 SSH 端口
  DEPLOY_SERVICE_NAME  LaunchAgent Label
  ARTIFACT_DIR         本地构建产物目录；未设置时使用临时目录并在结束后删除
  SKIP_BUILD=1         跳过构建；必须同时指定 ARTIFACT_DIR 以复用已有产物

示例:
  ./deploy.sh
  DEPLOY_HOST=gateway.example.com DEPLOY_USER=deploy ./deploy.sh
  ARTIFACT_DIR=/path/to/artifacts ./deploy.sh --skip-build
USAGE
}

log() {
  printf '\n[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "缺少依赖命令: $1" >&2
    exit 1
  fi
}

shell_quote() {
  printf '%q' "$1"
}

for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD="1" ;;
    -h|--help|help) usage; exit 0 ;;
    *) echo "错误：未知参数 \"$arg\"" >&2; echo >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "$ARTIFACT_DIR" && "$SKIP_BUILD" == "1" ]]; then
  echo "错误：--skip-build 时必须指定 ARTIFACT_DIR。" >&2
  exit 1
fi

TEMP_ARTIFACT_DIR=""
if [[ -z "$ARTIFACT_DIR" ]]; then
  require_cmd mktemp
  TEMP_ARTIFACT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ai-gateway-release.XXXXXX")"
  ARTIFACT_DIR="$TEMP_ARTIFACT_DIR"
  trap 'rm -rf "$TEMP_ARTIFACT_DIR"' EXIT
fi

SSH_ARGS=(-p "$SSH_PORT" -o StrictHostKeyChecking=accept-new)
REMOTE_TARGET="${DEPLOY_USER}@${DEPLOY_HOST}"

build_local() {
  require_cmd cargo
  require_cmd bun

  log "安装 Web 构建依赖"
  (
    cd "$ROOT_DIR/web"
    bun install --frozen-lockfile
  )

  log "构建后端服务: $SERVER_PKG"
  (
    cd "$ROOT_DIR"
    cargo build --release -p "$SERVER_PKG"
  )

  log "构建 Web 管理端"
  (
    cd "$ROOT_DIR/web"
    bun run build
  )

  log "整理发布产物: $ARTIFACT_DIR"
  rm -rf "$ARTIFACT_DIR"
  mkdir -p "$ARTIFACT_DIR/bin" "$ARTIFACT_DIR/web"
  install -m 755 "$ROOT_DIR/target/release/$SERVER_PKG" "$ARTIFACT_DIR/bin/$SERVER_PKG"

  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete "$ROOT_DIR/web/dist/" "$ARTIFACT_DIR/web/"
  else
    cp -R "$ROOT_DIR/web/dist/." "$ARTIFACT_DIR/web/"
  fi
}

verify_artifacts() {
  if [[ ! -x "$ARTIFACT_DIR/bin/$SERVER_PKG" ]]; then
    echo "错误：未找到可执行发布产物: $ARTIFACT_DIR/bin/$SERVER_PKG" >&2
    exit 1
  fi
  if [[ ! -f "$ARTIFACT_DIR/web/index.html" ]]; then
    echo "错误：未找到 Web 发布产物: $ARTIFACT_DIR/web/index.html" >&2
    exit 1
  fi
}

sync_remote() {
  require_cmd ssh
  require_cmd rsync

  log "创建远端运行目录: $REMOTE_TARGET:~/$REMOTE_APP_DIR"
  ssh "${SSH_ARGS[@]}" "$REMOTE_TARGET" \
    "mkdir -p $(shell_quote "$REMOTE_APP_DIR")/{bin,web,log}"

  # Upload to temporary filenames. The remote restart step atomically promotes
  # them only after both binary and static files have arrived.
  log "同步二进制"
  rsync -az -e "ssh ${SSH_ARGS[*]}" \
    "$ARTIFACT_DIR/bin/$SERVER_PKG" \
    "$REMOTE_TARGET:$REMOTE_APP_DIR/bin/$SERVER_PKG.new"

  log "同步 Web 静态资源"
  rsync -az --delete --delay-updates -e "ssh ${SSH_ARGS[*]}" \
    "$ARTIFACT_DIR/web/" \
    "$REMOTE_TARGET:$REMOTE_APP_DIR/web/"
}

restart_remote_service() {
  ssh "${SSH_ARGS[@]}" "$REMOTE_TARGET" \
    "REMOTE_APP_DIR=$(shell_quote "$REMOTE_APP_DIR") SERVER_PKG=$(shell_quote "$SERVER_PKG") DEPLOY_SERVICE_NAME=$(shell_quote "$DEPLOY_SERVICE_NAME") GATEWAY_PORT=$(shell_quote "$GATEWAY_PORT") bash -s" <<'REMOTE_SCRIPT'
set -euo pipefail

remote_log() {
  printf '\n[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"
}

APP_DIR="$HOME/$REMOTE_APP_DIR"
BIN_PATH="$APP_DIR/bin/$SERVER_PKG"
NEW_BIN_PATH="$BIN_PATH.new"
LOG_DIR="$APP_DIR/log"
PLIST_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$PLIST_DIR/$DEPLOY_SERVICE_NAME.plist"
DOMAIN="gui/$(id -u)"
TARGET="$DOMAIN/$DEPLOY_SERVICE_NAME"

if [[ ! -f "$NEW_BIN_PATH" ]]; then
  echo "缺少已上传的二进制文件: $NEW_BIN_PATH" >&2
  exit 1
fi

write_plist() {
  mkdir -p "$PLIST_DIR" "$LOG_DIR"
  cat >"$PLIST_PATH" <<EOF_PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$DEPLOY_SERVICE_NAME</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN_PATH</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$APP_DIR</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key>
    <string>$HOME</string>
    <key>RUST_LOG</key>
    <string>info</string>
  </dict>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/service.out.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/service.err.log</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
EOF_PLIST
  plutil -lint "$PLIST_PATH" >/dev/null
}

kill_listening_port() {
  command -v lsof >/dev/null 2>&1 || return 0
  local pids
  pids="$(lsof -ti "tcp:$GATEWAY_PORT" -sTCP:LISTEN 2>/dev/null || true)"
  if [[ -n "$pids" ]]; then
    remote_log "端口 $GATEWAY_PORT 被占用，停止旧进程: ${pids//$'\n'/, }"
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
    sleep 0.5
  fi
}

remote_log "安装新二进制"
mkdir -p "$APP_DIR/bin" "$APP_DIR/web" "$LOG_DIR"
chmod +x "$NEW_BIN_PATH"
mv -f "$NEW_BIN_PATH" "$BIN_PATH"
write_plist

remote_log "停止旧服务: $TARGET"
launchctl bootout "$TARGET" >/dev/null 2>&1 || launchctl bootout "$DOMAIN" "$PLIST_PATH" >/dev/null 2>&1 || true
kill_listening_port

remote_log "加载并启动服务: $TARGET"
if ! launchctl bootstrap "$DOMAIN" "$PLIST_PATH" >/dev/null 2>&1; then
  launchctl bootout "$TARGET" >/dev/null 2>&1 || true
  launchctl bootstrap "$DOMAIN" "$PLIST_PATH"
fi
launchctl enable "$TARGET" >/dev/null 2>&1 || true
launchctl kickstart -k "$TARGET"

if command -v curl >/dev/null 2>&1; then
  remote_log "等待健康检查: http://127.0.0.1:$GATEWAY_PORT/healthz"
  for _ in {1..30}; do
    if curl -fsS "http://127.0.0.1:$GATEWAY_PORT/healthz" >/dev/null 2>&1; then
      remote_log "服务已启动: http://127.0.0.1:$GATEWAY_PORT"
      exit 0
    fi
    sleep 1
  done

  echo "服务启动后健康检查失败，诊断信息如下：" >&2
  launchctl print "$TARGET" || true
  echo "--- stderr log ---" >&2
  tail -120 "$LOG_DIR/service.err.log" 2>/dev/null || true
  echo "--- stdout log ---" >&2
  tail -120 "$LOG_DIR/service.out.log" 2>/dev/null || true
  exit 1
fi
REMOTE_SCRIPT
}

if [[ "$SKIP_BUILD" != "1" ]]; then
  log "使用临时发布目录: $ARTIFACT_DIR"
  build_local
fi
verify_artifacts
sync_remote
restart_remote_service
log "服务发布完成"
