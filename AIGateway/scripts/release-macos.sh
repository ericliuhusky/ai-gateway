#!/bin/sh

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${PROJECT_ROOT}/.." && pwd)"
PROJECT_PATH="${PROJECT_ROOT}/AIGateway.xcodeproj"
SCHEME="AIGateway"
CONFIGURATION="Release"
REPO="ericliuhusky/ai-gateway"
APP_NAME="AIGateway.app"
ZIP_NAME="AIGateway.app.zip"
BUILD_ROOT="$(mktemp -d /tmp/ai-gateway-release.XXXXXX)"
ARCHIVE_ROOT="${BUILD_ROOT}/archive"
EXPORT_ROOT="${BUILD_ROOT}/export"
ASSET_PATH="${EXPORT_ROOT}/${ZIP_NAME}"

usage() {
  cat <<EOF
Usage: $(basename "$0") [notes-file]

Examples:
  $(basename "$0")
  $(basename "$0") ./release-notes.md

This script will:
  0. read MARKETING_VERSION and CURRENT_PROJECT_VERSION from Xcode build settings
  1. build the macOS app in Release mode
  2. zip it as ${ZIP_NAME}
  3. create a GitHub release tag from MARKETING_VERSION, for example v1.0.1
  4. upload the zip asset to ${REPO}
EOF
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

NOTES_FILE="${1:-}"

require_command xcodebuild
require_command ditto
require_command gh
require_command mktemp

cleanup() {
  if [ -n "${BUILD_ROOT:-}" ] && [ -d "${BUILD_ROOT}" ]; then
    rm -rf "${BUILD_ROOT}"
  fi
}

trap cleanup EXIT INT TERM

if ! gh auth status >/dev/null 2>&1; then
  echo "error: gh is not authenticated. Run: gh auth login" >&2
  exit 1
fi

if [ -n "${NOTES_FILE}" ] && [ ! -f "${NOTES_FILE}" ]; then
  echo "error: notes file not found: ${NOTES_FILE}" >&2
  exit 1
fi

BUILD_SETTINGS="$(xcodebuild -project "${PROJECT_PATH}" -scheme "${SCHEME}" -configuration "${CONFIGURATION}" -showBuildSettings)"

MARKETING_VERSION="$(printf '%s\n' "${BUILD_SETTINGS}" | sed -n 's/^[[:space:]]*MARKETING_VERSION = //p' | head -n 1)"
CURRENT_PROJECT_VERSION="$(printf '%s\n' "${BUILD_SETTINGS}" | sed -n 's/^[[:space:]]*CURRENT_PROJECT_VERSION = //p' | head -n 1)"

if [ -z "${MARKETING_VERSION}" ]; then
  echo "error: failed to read MARKETING_VERSION from Xcode build settings" >&2
  exit 1
fi

if [ -z "${CURRENT_PROJECT_VERSION}" ]; then
  echo "error: failed to read CURRENT_PROJECT_VERSION from Xcode build settings" >&2
  exit 1
fi

TAG="v${MARKETING_VERSION}"

echo "Preparing release ${TAG}"
echo "Version: ${MARKETING_VERSION} (${CURRENT_PROJECT_VERSION})"
mkdir -p "${ARCHIVE_ROOT}" "${EXPORT_ROOT}"

echo "Building ${APP_NAME}"
xcodebuild \
  -project "${PROJECT_PATH}" \
  -scheme "${SCHEME}" \
  -configuration "${CONFIGURATION}" \
  -derivedDataPath "${BUILD_ROOT}/DerivedData" \
  build

APP_PATH="$(find "${BUILD_ROOT}/DerivedData/Build/Products/${CONFIGURATION}" -maxdepth 1 -type d -name "${APP_NAME}" | head -n 1)"

if [ -z "${APP_PATH}" ] || [ ! -d "${APP_PATH}" ]; then
  echo "error: built app not found" >&2
  exit 1
fi

echo "Packaging ${ZIP_NAME}"
ditto -c -k --sequesterRsrc --keepParent "${APP_PATH}" "${ASSET_PATH}"

if gh release view "${TAG}" --repo "${REPO}" >/dev/null 2>&1; then
  echo "error: release ${TAG} already exists on ${REPO}" >&2
  exit 1
fi

echo "Creating GitHub release ${TAG}"
if [ -n "${NOTES_FILE}" ]; then
  gh release create "${TAG}" "${ASSET_PATH}" \
    --repo "${REPO}" \
    --title "${TAG}" \
    --notes-file "${NOTES_FILE}"
else
  gh release create "${TAG}" "${ASSET_PATH}" \
    --repo "${REPO}" \
    --title "${TAG}" \
    --generate-notes
fi

echo "Release created successfully"
echo "Asset: ${ASSET_PATH}"
