#!/bin/bash
# curl -fsSL https://raw.githubusercontent.com/drbh/jikan/main/install.sh | sh
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# Function to print error messages
error() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

# Function to print success messages
success() {
    echo -e "${GREEN}$1${NC}"
}

# Detect the operating system and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Map OS and architecture names
case $OS in
    darwin)
        OS="macos"
        ;;
    linux)
        ;;
    *)
        error "Unsupported operating system: $OS"
        ;;
esac

case $ARCH in
    x86_64)
        ;;
    arm64)
        if [ "$OS" != "macos" ]; then
            error "ARM64 is only supported on macOS"
        fi
        ;;
    *)
        error "Unsupported architecture: $ARCH"
        ;;
esac

# GitHub repository information
GITHUB_REPO="drbh/jikan"
API_URL="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"

# Fetch the latest release version
echo "Fetching the latest version..."
VERSION=$(curl -s $API_URL | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$VERSION" ]; then
    error "Failed to fetch the latest version"
fi

echo "Latest version: $VERSION"

# Construct the download URLs
DAEMON_BINARY_NAME="jikand-${OS}-${ARCH}"
CLIENT_BINARY_NAME="jk-${OS}-${ARCH}"
DAEMON_DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${DAEMON_BINARY_NAME}"
CLIENT_DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${CLIENT_BINARY_NAME}"

# Create a temporary directory
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# Download the latest release binaries
echo "Downloading Jikan binaries..."
curl -L "$DAEMON_DOWNLOAD_URL" -o "$TMP_DIR/jikand" || error "Failed to download jikand"
curl -L "$CLIENT_DOWNLOAD_URL" -o "$TMP_DIR/jk" || error "Failed to download jk"

# Make the binaries executable
chmod +x "$TMP_DIR/jikand" "$TMP_DIR/jk"

# Install Jikan binaries
install -d "$HOME/.local/bin"
install "$TMP_DIR/jikand" "$HOME/.local/bin/jikand" || error "Failed to install jikand"
install "$TMP_DIR/jk" "$HOME/.local/bin/jk" || error "Failed to install jk"

# Add to PATH if not already there
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.zshrc"
    success "Added Jikan to PATH. Please restart your shell or run 'source ~/.bashrc' (or ~/.zshrc) to use Jikan."
fi

success "Jikan version $VERSION has been successfully installed!"
echo "Run 'jikand --help' to get started with the daemon."
echo "Run 'jk --help' to get started with the client."