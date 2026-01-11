#!/usr/bin/env bash
set -euo pipefail

# Muninn installer
# Usage: curl -fsSL https://raw.githubusercontent.com/OWNER/muninn/main/install.sh | bash

REPO="colliery-io/muninn"
BINARY="muninn"
INSTALL_DIR="${MUNINN_INSTALL_DIR:-$HOME/.local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

info() { echo -e "${GREEN}info${NC}: $1"; }
warn() { echo -e "${YELLOW}warn${NC}: $1"; }
error() { echo -e "${RED}error${NC}: $1" >&2; exit 1; }

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "darwin" ;;
        *)       error "Unsupported OS: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *)             error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Get latest release version
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name":' \
        | sed -E 's/.*"([^"]+)".*/\1/'
}

# Build target triple
get_target() {
    local os=$1
    local arch=$2

    case "$os" in
        linux)  echo "${arch}-unknown-linux-gnu" ;;
        darwin) echo "${arch}-apple-darwin" ;;
    esac
}

main() {
    local os=$(detect_os)
    local arch=$(detect_arch)
    local target=$(get_target "$os" "$arch")

    info "Detected platform: ${os}/${arch}"

    # Get version (from arg or latest)
    local version="${1:-$(get_latest_version)}"
    if [[ -z "$version" ]]; then
        error "Could not determine latest version"
    fi
    info "Installing ${BINARY} ${version}"

    # Download URL
    local url="https://github.com/${REPO}/releases/download/${version}/${BINARY}-${version}-${target}.tar.gz"
    info "Downloading from: ${url}"

    # Create temp directory
    local tmpdir=$(mktemp -d)
    trap "rm -rf ${tmpdir}" EXIT

    # Download and extract
    curl -fsSL "$url" | tar -xz -C "$tmpdir"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"

    info "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

    # Check if in PATH
    if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
        warn "${INSTALL_DIR} is not in your PATH"
        echo ""
        echo "Add it to your shell config:"
        echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
    fi

    info "Installation complete!"
    echo ""
    "${INSTALL_DIR}/${BINARY}" --version || true
}

main "$@"
