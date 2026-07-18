#!/bin/sh
# Install OxiDNS Next release archives on Linux, macOS, and FreeBSD.
#
# Common overrides:
#   OXIDNS_NEXT_VERSION=v0.1.0
#   OXIDNS_NEXT_INSTALL_DIR=/opt/oxidns-next
#   OXIDNS_NEXT_BIN_DIR=/usr/local/bin
#   OXIDNS_NEXT_TARGET=x86_64-unknown-linux-musl
#   OXIDNS_NEXT_BUNDLE=standard
#   OXIDNS_NEXT_INSTALL_SERVICE=0
#   OXIDNS_NEXT_START_SERVICE=0

set -eu

REPO="${OXIDNS_NEXT_REPO:-ciallothu/oxidns-next}"
VERSION="${OXIDNS_NEXT_VERSION:-latest}"
TARGET="${OXIDNS_NEXT_TARGET:-}"
BUNDLE="${OXIDNS_NEXT_BUNDLE:-full}"
INSTALL_DIR="${OXIDNS_NEXT_INSTALL_DIR:-}"
BIN_DIR="${OXIDNS_NEXT_BIN_DIR:-}"
NO_PATH="${OXIDNS_NEXT_NO_PATH:-0}"
INSTALL_SERVICE="${OXIDNS_NEXT_INSTALL_SERVICE:-auto}"
START_SERVICE="${OXIDNS_NEXT_START_SERVICE:-1}"

log() {
    printf '%s\n' "$*"
}

warn() {
    printf 'warning: %s\n' "$*" >&2
}

err() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

is_truthy() {
    case "$1" in
        1|true|TRUE|yes|YES|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

is_falsey() {
    case "$1" in
        0|false|FALSE|no|NO|off|OFF) return 0 ;;
        *) return 1 ;;
    esac
}

is_root() {
    [ "$(id -u 2>/dev/null || printf '1')" = "0" ]
}

is_openwrt() {
    [ -f /etc/openwrt_release ] && return 0
    [ -r /etc/os-release ] && grep -qi 'openwrt' /etc/os-release 2>/dev/null
}

should_install_service() {
    case "$INSTALL_SERVICE" in
        auto|"")
            is_root && ! is_openwrt
            ;;
        *)
            is_truthy "$INSTALL_SERVICE"
            ;;
    esac
}

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os:$arch" in
        Linux:x86_64|Linux:amd64)
            printf 'x86_64-unknown-linux-musl'
            ;;
        Linux:aarch64|Linux:arm64)
            printf 'aarch64-unknown-linux-musl'
            ;;
        Linux:i386|Linux:i686)
            printf 'i686-unknown-linux-musl'
            ;;
        Linux:armv7l|Linux:armv7)
            printf 'arm-unknown-linux-musleabihf'
            ;;
        Darwin:x86_64|Darwin:amd64)
            printf 'x86_64-apple-darwin'
            ;;
        Darwin:arm64|Darwin:aarch64)
            printf 'aarch64-apple-darwin'
            ;;
        FreeBSD:x86_64|FreeBSD:amd64)
            printf 'x86_64-unknown-freebsd'
            ;;
        *)
            err "unsupported platform: $os $arch. Set OXIDNS_NEXT_TARGET to override."
            ;;
    esac
}

download() {
    url="$1"
    out="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --retry-delay 2 -o "$out" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$out" "$url"
    else
        err "curl or wget is required to download OxiDNS Next"
    fi
}

contains_path() {
    dir="$1"
    case ":${PATH:-}:" in
        *:"$dir":*) return 0 ;;
        *) return 1 ;;
    esac
}

install_link() {
    mkdir -p "$BIN_DIR"
    link_path="$BIN_DIR/oxidns-next"

    if [ "$BIN_DIR" = "$INSTALL_DIR" ]; then
        return 0
    fi

    if [ -e "$link_path" ] && [ ! -L "$link_path" ]; then
        warn "$link_path already exists and is not a symlink; leaving it unchanged"
        return 0
    fi

    if ln -sf "$INSTALL_DIR/oxidns-next" "$link_path" 2>/dev/null; then
        return 0
    fi

    warn "failed to create symlink at $link_path; copying binary instead"
    cp "$INSTALL_DIR/oxidns-next" "$link_path"
    chmod 755 "$link_path"
}

if [ -z "$TARGET" ]; then
    TARGET="$(detect_target)"
fi
BUNDLE="$(printf '%s' "$BUNDLE" | tr '[:upper:]' '[:lower:]')"

case "$TARGET" in
    *windows*|*msvc*)
        err "Windows targets are installed with scripts/install.ps1"
        ;;
esac

case "$BUNDLE" in
    full)
        ASSET="oxidns-next-$TARGET.tar.gz"
        ;;
    minimal|standard)
        case "$TARGET" in
            x86_64-unknown-linux-musl|aarch64-unknown-linux-musl)
                ASSET="oxidns-next-$BUNDLE-$TARGET.tar.gz"
                ;;
            *)
                err "OXIDNS_NEXT_BUNDLE=$BUNDLE is only published for x86_64-unknown-linux-musl and aarch64-unknown-linux-musl"
                ;;
        esac
        ;;
    *)
        err "unsupported OXIDNS_NEXT_BUNDLE=$BUNDLE; expected full, minimal, or standard"
        ;;
esac

if [ -z "$INSTALL_DIR" ]; then
    if is_root; then
        INSTALL_DIR="/opt/oxidns-next"
    else
        [ -n "${HOME:-}" ] || err "HOME is not set; set OXIDNS_NEXT_INSTALL_DIR explicitly"
        INSTALL_DIR="$HOME/.oxidns-next"
    fi
fi

if [ -z "$BIN_DIR" ]; then
    if is_root; then
        BIN_DIR="/usr/local/bin"
    else
        [ -n "${HOME:-}" ] || err "HOME is not set; set OXIDNS_NEXT_BIN_DIR explicitly"
        BIN_DIR="$HOME/.local/bin"
    fi
fi

if should_install_service && ! is_root; then
    err "service installation requires root; rerun with sudo or set OXIDNS_NEXT_INSTALL_SERVICE=0"
fi

if [ "$VERSION" = "latest" ]; then
    URL="https://github.com/$REPO/releases/latest/download/$ASSET"
else
    URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
fi

for command_name in tar mkdir cp chmod mv mktemp; do
    command -v "$command_name" >/dev/null 2>&1 || err "required command not found: $command_name"
done

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/oxidns-next-install.XXXXXX")"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT HUP INT TERM

ARCHIVE="$TMP_DIR/$ASSET"
UNPACK_DIR="$TMP_DIR/unpack"

log "Downloading $ASSET from $REPO ($VERSION)..."
download "$URL" "$ARCHIVE"

mkdir -p "$UNPACK_DIR"
tar -xzf "$ARCHIVE" -C "$UNPACK_DIR"

[ -f "$UNPACK_DIR/oxidns-next" ] || err "archive does not contain oxidns-next"
[ -f "$UNPACK_DIR/config.yaml" ] || err "archive does not contain config.yaml"

mkdir -p "$INSTALL_DIR"
cp "$UNPACK_DIR/oxidns-next" "$INSTALL_DIR/oxidns-next.tmp"
chmod 755 "$INSTALL_DIR/oxidns-next.tmp"
mv "$INSTALL_DIR/oxidns-next.tmp" "$INSTALL_DIR/oxidns-next"

if [ -f "$INSTALL_DIR/config.yaml" ]; then
    cp "$UNPACK_DIR/config.yaml" "$INSTALL_DIR/config.yaml.example"
    CONFIG_PATH="$INSTALL_DIR/config.yaml"
    log "Keeping existing config: $CONFIG_PATH"
    log "Wrote release example config: $INSTALL_DIR/config.yaml.example"
else
    cp "$UNPACK_DIR/config.yaml" "$INSTALL_DIR/config.yaml"
    CONFIG_PATH="$INSTALL_DIR/config.yaml"
fi

if [ -f "$UNPACK_DIR/LICENSE" ]; then
    cp "$UNPACK_DIR/LICENSE" "$INSTALL_DIR/LICENSE"
fi

if [ -d "$UNPACK_DIR/webui" ]; then
    rm -rf "$INSTALL_DIR/webui"
    cp -R "$UNPACK_DIR/webui" "$INSTALL_DIR/webui"
fi

if ! is_truthy "$NO_PATH"; then
    install_link
fi

if "$INSTALL_DIR/oxidns-next" check -c "$CONFIG_PATH" -d "$INSTALL_DIR" >/dev/null 2>&1; then
    log "Config check passed: $CONFIG_PATH"
else
    warn "installed binary is ready, but config check failed: $CONFIG_PATH"
fi

if should_install_service; then
    "$INSTALL_DIR/oxidns-next" service install -d "$INSTALL_DIR" -c "$CONFIG_PATH"
    if is_truthy "$START_SERVICE"; then
        "$INSTALL_DIR/oxidns-next" service start
    elif ! is_falsey "$START_SERVICE"; then
        err "unsupported OXIDNS_NEXT_START_SERVICE=$START_SERVICE; expected 1 or 0"
    fi
elif is_openwrt; then
    log "OpenWrt detected; installed the core without a LuCI package or system service"
fi

log "OxiDNS Next installed to $INSTALL_DIR"
if ! is_truthy "$NO_PATH"; then
    log "Command shim: $BIN_DIR/oxidns-next"
    if ! contains_path "$BIN_DIR"; then
        warn "$BIN_DIR is not in PATH; add it to your shell profile or run $INSTALL_DIR/oxidns-next directly"
    fi
fi
log "Try: oxidns-next start -c $CONFIG_PATH -d $INSTALL_DIR"
