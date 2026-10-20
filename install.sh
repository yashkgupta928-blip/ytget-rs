#!/usr/bin/env bash
# ytget-rs installer/updater for Linux and macOS.
# Optional override: set YTGET_REPO=owner/repository. The default is configured for this repository.
set -Eeuo pipefail

if [[ "${1:-install}" == "update" ]]; then MODE_LABEL="Update"; else MODE_LABEL="Install"; fi

MODE="${1:-install}"
if [[ "$MODE" != "install" && "$MODE" != "update" ]]; then
  echo "Usage: $0 [install|update]" >&2
  exit 2
fi

REPO="${YTGET_REPO:-yashkgupta928-blip/ytget-rs}"
if [[ "$REPO" == *YOUR_GITHUB_USERNAME* || "$REPO" != */* ]]; then
  cat >&2 <<'MSG'
Set YTGET_REPO to your GitHub owner/repository before running this script,
or set YTGET_REPO to a different repository if using a fork.
Example: export YTGET_REPO="yourname/ytget-rs"
MSG
  exit 2
fi

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-unknown-linux-gnu"; YTDLP_ASSET="yt-dlp_linux" ;;
      aarch64|arm64) TARGET="aarch64-unknown-linux-gnu"; YTDLP_ASSET="yt-dlp_linux_aarch64" ;;
      *) echo "Unsupported Linux architecture: $ARCH (supported: x86_64, aarch64)." >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-apple-darwin" ;;
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      *) echo "Unsupported macOS architecture: $ARCH." >&2; exit 1 ;;
    esac
    YTDLP_ASSET="yt-dlp_macos"
    ;;
  *) echo "This script supports Linux and macOS only; use install.ps1 on Windows." >&2; exit 1 ;;
esac

BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR"
TMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}
need_cmd curl
need_cmd awk
need_cmd tar

fetch() {
  curl --fail --location --silent --show-error --retry 3 --retry-delay 1 "$1" --output "$2"
}

manifest_hash() {
  local manifest="$1" asset="$2"
  awk -v wanted="$asset" '{ name=$2; sub(/^\*/, "", name); if (name == wanted) { print $1; exit } }' "$manifest"
}
verify_sha256() {
  local file="$1" asset="$2" manifest="$3" expected actual
  expected="$(manifest_hash "$manifest" "$asset")"
  if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "Could not find a SHA-256 checksum for $asset; refusing to install it." >&2
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  fi
  actual="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  if [[ "$actual" != "$expected" ]]; then
    echo "SHA-256 verification failed for $asset; the file was not installed." >&2
    exit 1
  fi
}

install_ffmpeg() {
  echo "Checking FFmpeg..."
  if [[ "$OS" == "Darwin" ]]; then
    if ! command -v brew >/dev/null 2>&1; then
      if [[ -x /opt/homebrew/bin/brew ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
      elif [[ -x /usr/local/bin/brew ]]; then
        eval "$(/usr/local/bin/brew shellenv)"
      else
        echo "Homebrew is missing. Installing it from the official Homebrew source..."
        NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        if [[ -x /opt/homebrew/bin/brew ]]; then
          eval "$(/opt/homebrew/bin/brew shellenv)"
        elif [[ -x /usr/local/bin/brew ]]; then
          eval "$(/usr/local/bin/brew shellenv)"
        fi
      fi
    fi
    if ! command -v brew >/dev/null 2>&1; then
      echo "Homebrew installation did not finish. Install Xcode Command Line Tools with 'xcode-select --install', then rerun this script." >&2
      exit 1
    fi
    # Make Homebrew available in future terminal sessions as well as this script.
    local brew_exe brew_init brew_rc shell_name
    brew_exe="$(command -v brew)"
    shell_name="$(basename "${SHELL:-zsh}")"
    case "$shell_name" in
      bash) brew_rc="$HOME/.bash_profile" ;;
      fish)
        brew_rc="$HOME/.config/fish/config.fish"
        case "$brew_exe" in
          /opt/homebrew/bin/brew) brew_init='fish_add_path /opt/homebrew/bin /opt/homebrew/sbin' ;;
          /usr/local/bin/brew) brew_init='fish_add_path /usr/local/bin /usr/local/sbin' ;;
          *) brew_init="" ;;
        esac
        ;;
      *) brew_rc="$HOME/.zprofile" ;;
    esac
    if [[ "$shell_name" != "fish" ]]; then
      case "$brew_exe" in
        /opt/homebrew/bin/brew) brew_init='eval "$(/opt/homebrew/bin/brew shellenv)"' ;;
        /usr/local/bin/brew) brew_init='eval "$(/usr/local/bin/brew shellenv)"' ;;
        *) brew_init="" ;;
      esac
    fi
    if [[ -n "$brew_rc" && -n "$brew_init" ]]; then
      mkdir -p "$(dirname "$brew_rc")"
      if [[ ! -f "$brew_rc" ]] || ! grep -Fq "$brew_init" "$brew_rc"; then
        printf '\n# Homebrew environment\n%s\n' "$brew_init" >> "$brew_rc"
      fi
    fi
    brew update
    if [[ "$MODE" == "update" ]] && brew list --versions ffmpeg >/dev/null 2>&1; then
      brew upgrade ffmpeg
    else
      brew install ffmpeg
    fi
  else
    # FFmpeg is installed/updated through the Linux distribution's package repositories.
    if command -v apt-get >/dev/null 2>&1; then
      sudo apt-get update
      sudo apt-get install -y ffmpeg
    elif command -v dnf >/dev/null 2>&1; then
      # Fedora commonly names its codec-restricted build ffmpeg-free in the base repos.
      if [[ "$MODE" == "update" ]]; then
        sudo dnf upgrade -y ffmpeg || sudo dnf upgrade -y ffmpeg-free || sudo dnf install -y ffmpeg || sudo dnf install -y ffmpeg-free
      else
        sudo dnf install -y ffmpeg || sudo dnf install -y ffmpeg-free
      fi
    elif command -v yum >/dev/null 2>&1; then
      if [[ "$MODE" == "update" ]]; then sudo yum update -y ffmpeg || sudo yum install -y ffmpeg; else sudo yum install -y ffmpeg; fi
    elif command -v pacman >/dev/null 2>&1; then
      # Arch requires a full system upgrade when refreshing package databases.
      sudo pacman -Syu --needed --noconfirm ffmpeg
    elif command -v zypper >/dev/null 2>&1; then
      sudo zypper --non-interactive refresh
      if [[ "$MODE" == "update" ]]; then sudo zypper --non-interactive update ffmpeg || sudo zypper --non-interactive install ffmpeg; else sudo zypper --non-interactive install ffmpeg; fi
    elif command -v apk >/dev/null 2>&1; then
      if [[ "$MODE" == "update" ]]; then sudo apk upgrade ffmpeg || sudo apk add ffmpeg; else sudo apk add ffmpeg; fi
    else
      cat >&2 <<'MSG'
No supported Linux package manager was detected. Install FFmpeg using your
Linux distribution's trusted package source, then rerun this script.
Official FFmpeg downloads page: https://ffmpeg.org/download.html
Supported package managers: apt, dnf, yum, pacman, zypper, apk.
MSG
      exit 1
    fi
  fi
  if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "FFmpeg installation finished, but 'ffmpeg' is not available on PATH." >&2
    exit 1
  fi
  ffmpeg -version | head -n 1 || true
}

install_app() {
  local asset="ytget-rs-${TARGET}"
  local archive
  if [[ "$TARGET" == *windows* ]]; then archive="${asset}.zip"; else archive="${asset}.tar.gz"; fi
  echo "Downloading ytget-rs release for ${TARGET}..."
  fetch "https://github.com/${REPO}/releases/latest/download/${archive}" "$TMP_DIR/$archive"
  fetch "https://github.com/${REPO}/releases/latest/download/SHA256SUMS" "$TMP_DIR/SHA256SUMS"
  verify_sha256 "$TMP_DIR/$archive" "$archive" "$TMP_DIR/SHA256SUMS"
  mkdir -p "$TMP_DIR/app"
  tar -xzf "$TMP_DIR/$archive" -C "$TMP_DIR/app"
  if [[ ! -f "$TMP_DIR/app/ytget-rs" ]]; then
    echo "The release archive did not contain ytget-rs." >&2
    exit 1
  fi
  install -m 0755 "$TMP_DIR/app/ytget-rs" "$BIN_DIR/ytget-rs.new"
  mv -f "$BIN_DIR/ytget-rs.new" "$BIN_DIR/ytget-rs"
  echo "Installed ytget-rs to $BIN_DIR/ytget-rs"
}

install_ytdlp() {
  local checksum_url="https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS"
  local dest="$BIN_DIR/yt-dlp"
  echo "Downloading the official yt-dlp release ($YTDLP_ASSET)..."
  fetch "https://github.com/yt-dlp/yt-dlp/releases/latest/download/${YTDLP_ASSET}" "$TMP_DIR/$YTDLP_ASSET"
  fetch "$checksum_url" "$TMP_DIR/SHA2-256SUMS"
  verify_sha256 "$TMP_DIR/$YTDLP_ASSET" "$YTDLP_ASSET" "$TMP_DIR/SHA2-256SUMS"
  install -m 0755 "$TMP_DIR/$YTDLP_ASSET" "$dest.new"
  mv -f "$dest.new" "$dest"
  echo "Installed yt-dlp to $dest"
}

persist_path() {
  local marker="# ytget-rs user PATH"
  local shell_name rc_files=() rc
  shell_name="$(basename "${SHELL:-sh}")"
  case "$shell_name" in
    zsh) rc_files=("$HOME/.zprofile" "$HOME/.zshrc") ;;
    bash) rc_files=("$HOME/.profile" "$HOME/.bash_profile" "$HOME/.bashrc") ;;
    fish)
      local fish_rc="$HOME/.config/fish/config.fish"
      mkdir -p "$(dirname "$fish_rc")"
      if [[ ! -f "$fish_rc" ]] || ! grep -Fq "$marker" "$fish_rc"; then
        printf '\n%s\nfish_add_path "$HOME/.local/bin"\n' "$marker" >> "$fish_rc"
      fi
      ;;
    *) rc_files=("$HOME/.profile") ;;
  esac
  for rc in "${rc_files[@]}"; do
    if [[ ! -f "$rc" ]] || ! grep -Fq "$marker" "$rc"; then
      printf '\n%s\nexport PATH="$HOME/.local/bin:$PATH"\n' "$marker" >> "$rc"
    fi
  done
  export PATH="$BIN_DIR:$PATH"
}

if [[ "$MODE" == "update" ]]; then echo "Updating ytget-rs and its dependencies..."; else echo "Installing ytget-rs and its dependencies..."; fi
install_ffmpeg
install_ytdlp
install_app
persist_path

if ! command -v ffprobe >/dev/null 2>&1; then
  echo "Note: ffprobe was not found. FFmpeg installations normally include it." >&2
fi
cat <<MSG

${MODE_LABEL} complete.
App:    $BIN_DIR/ytget-rs
yt-dlp: $BIN_DIR/yt-dlp
FFmpeg: $(command -v ffmpeg)

Open a new terminal (or source your shell profile), then run:
  ytget-rs
MSG
