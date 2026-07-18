#!/bin/sh
# Uninstall OxiDNS Next files installed by scripts/install.sh.
#
# Common overrides:
#   OXIDNS_NEXT_INSTALL_DIR=/opt/oxidns-next
#   OXIDNS_NEXT_BIN_DIR=/usr/local/bin
#   OXIDNS_NEXT_UNINSTALL_SERVICE=auto
#   OXIDNS_NEXT_PURGE=1

set -eu

INSTALL_DIR="${OXIDNS_NEXT_INSTALL_DIR:-}"
BIN_DIR="${OXIDNS_NEXT_BIN_DIR:-}"
NO_PATH="${OXIDNS_NEXT_NO_PATH:-0}"
UNINSTALL_SERVICE="${OXIDNS_NEXT_UNINSTALL_SERVICE:-auto}"
PURGE="${OXIDNS_NEXT_PURGE:-0}"

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

is_root() {
    [ "$(id -u 2>/dev/null || printf '1')" = "0" ]
}

should_uninstall_service() {
    case "$UNINSTALL_SERVICE" in
        auto|"")
            is_root
            ;;
        *)
            is_truthy "$UNINSTALL_SERVICE"
            ;;
    esac
}

resolve_safe_install_dir() {
    candidate="$1"
    case "$candidate" in
        /*) ;;
        *) return 1 ;;
    esac
    case "$candidate" in
        ..|../*|*/..|*/../*) return 1 ;;
    esac

    if [ -d "$candidate" ]; then
        resolved="$(cd "$candidate" && pwd -P)" || return 1
    else
        parent="$(dirname "$candidate")"
        base="$(basename "$candidate")"
        resolved_parent="$(cd "$parent" && pwd -P)" || return 1
        resolved="$resolved_parent/$base"
    fi

    case "$resolved" in
        /|/bin|/dev|/etc|/home|/lib|/opt|/proc|/root|/sbin|/sys|/tmp|/usr|/var|/Users)
            return 1
            ;;
    esac
    case "$(basename "$resolved")" in
        *oxidns-next*)
            printf '%s\n' "$resolved"
            ;;
        *)
            return 1
            ;;
    esac
}

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

EXE="$INSTALL_DIR/oxidns-next"
LINK="$BIN_DIR/oxidns-next"

if should_uninstall_service; then
    if ! is_root; then
        err "service removal requires root; rerun with sudo or set OXIDNS_NEXT_UNINSTALL_SERVICE=0"
    fi
    if [ -x "$EXE" ]; then
        "$EXE" service stop >/dev/null 2>&1 || true
        "$EXE" service uninstall >/dev/null 2>&1 || warn "could not remove the OxiDNS Next service automatically"
    fi
fi

if ! is_truthy "$NO_PATH"; then
    if [ -L "$LINK" ]; then
        rm -f "$LINK"
        log "Removed command shim: $LINK"
    elif [ -f "$LINK" ] && [ "$BIN_DIR" != "$INSTALL_DIR" ]; then
        warn "$LINK is not a symlink; leaving it unchanged"
    fi
fi

if is_truthy "$PURGE"; then
    PURGE_DIR="$(resolve_safe_install_dir "$INSTALL_DIR")" || err "refusing to purge unsafe install directory: $INSTALL_DIR"
    if [ -e "$PURGE_DIR" ]; then
        rm -rf "$PURGE_DIR"
        log "Purged OxiDNS Next install directory: $PURGE_DIR"
    fi
else
    rm -f "$INSTALL_DIR/oxidns-next" "$INSTALL_DIR/oxidns-next.tmp"
    rm -rf "$INSTALL_DIR/webui"
    log "Removed OxiDNS Next binary and WebUI from $INSTALL_DIR"
    if [ -f "$INSTALL_DIR/config.yaml" ]; then
        log "Kept configuration: $INSTALL_DIR/config.yaml"
        log "Use OXIDNS_NEXT_PURGE=1 to remove the install directory and configuration"
    fi
fi

log "OxiDNS Next uninstall complete"
