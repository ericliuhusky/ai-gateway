#!/bin/sh

set -eu

PROJECT_ROOT="${SRCROOT}"
WORKSPACE_ROOT="$(cd "${PROJECT_ROOT}/.." && pwd)"
SERVER_BIN_NAME="ai-gateway"
OUTPUT_BIN_NAME="ai-gateway-server"
CARGO_BIN="${CARGO_BIN:-}"

if [ -z "${CARGO_BIN}" ] && [ -n "${HOME:-}" ] && [ -f "${HOME}/.cargo/env" ]; then
  # shellcheck disable=SC1090
  . "${HOME}/.cargo/env"
fi

for candidate in \
  "${CARGO_BIN}" \
  "${HOME:-}/.cargo/bin/cargo" \
  "/opt/homebrew/bin/cargo" \
  "/usr/local/bin/cargo"
do
  if [ -n "${candidate}" ] && [ -x "${candidate}" ]; then
    CARGO_BIN="${candidate}"
    break
  fi
done

if [ -z "${CARGO_BIN}" ] && command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="$(command -v cargo)"
fi

if [ -z "${CARGO_BIN}" ] || [ ! -x "${CARGO_BIN}" ]; then
  echo "error: cargo is required to build ${SERVER_BIN_NAME}" >&2
  echo "hint: set CARGO_BIN to your cargo path, or make sure ~/.cargo/bin is available to Xcode build scripts" >&2
  exit 1
fi

PROFILE_DIR="debug"
if [ "${CONFIGURATION}" = "Release" ]; then
  PROFILE_DIR="release"
  echo "Building bundled Rust server in release mode"
  "${CARGO_BIN}" build --manifest-path "${WORKSPACE_ROOT}/server/Cargo.toml" --release
else
  echo "Building bundled Rust server in debug mode"
  "${CARGO_BIN}" build --manifest-path "${WORKSPACE_ROOT}/server/Cargo.toml"
fi

SOURCE_BIN="${WORKSPACE_ROOT}/target/${PROFILE_DIR}/${SERVER_BIN_NAME}"
DEST_DIR="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}/bin"
DEST_BIN="${DEST_DIR}/${OUTPUT_BIN_NAME}"

if [ ! -x "${SOURCE_BIN}" ]; then
  echo "error: expected Rust binary at ${SOURCE_BIN}" >&2
  exit 1
fi

mkdir -p "${DEST_DIR}"
install -m 755 "${SOURCE_BIN}" "${DEST_BIN}"
echo "Embedded ${OUTPUT_BIN_NAME} at ${DEST_BIN}"
