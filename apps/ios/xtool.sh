#!/usr/bin/env bash
# Build and sideload from Linux. QMSG_IOS_STATE can reuse an existing xtool login/SDK.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
STATE="${QMSG_IOS_STATE:-${XDG_DATA_HOME:-$HOME/.local/share}/qmsg-xtool}"
IMAGE="${QMSG_IOS_IMAGE:-qmsg-xtool}"
mkdir -p "$STATE/home" "$STATE/lockdown"
if ! podman image exists "$IMAGE"; then
    podman build -t "$IMAGE" "$ROOT/toolchain"
fi
case "${1:-}" in
    setup|dev|install|devices|auth|sdk|new|ds|uninstall|launch) set -- xtool "$@" ;;
esac
TTY=()
[ -t 0 ] && TTY=(-it)
exec podman run --rm "${TTY[@]}" \
    --privileged --network host \
    -v /dev/bus/usb:/dev/bus/usb \
    -v "$STATE/home:/root" \
    -v "$STATE/lockdown:/var/lib/lockdown" \
    -v "$HOME/Downloads:/downloads:ro" \
    -v "$ROOT:/work" -w /work \
    "$IMAGE" "$@"
