#!/usr/bin/env bash
# Builds a Go provider into a qmsg component:
#
#   sdk/go/build.sh <provider-dir> <out.wasm>
#
# Tools are pinned and kept in target/go-wasm-tools (or $QMSG_GO_TOOLS):
# wasm-tools is installed there with cargo, and the preview1 reactor adapter
# from the Wasmtime release the orchestrator uses is downloaded and checked.
# Nothing is installed globally.
set -euo pipefail

WASM_TOOLS_VERSION=1.261.0
WASMTIME_VERSION=49.0.2
ADAPTER_SHA256=90b99ee01bfdb8f128bed56240f43a60ae5b016151f2f0c94bc4814a62f17d50

if [ $# -ne 2 ]; then
	echo "usage: $0 <provider-dir> <out.wasm>" >&2
	exit 2
fi
sdk=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$sdk/../.." && pwd)
provider=$(cd "$1" && pwd)
out=$(realpath -m "$2")
tools=${QMSG_GO_TOOLS:-$root/target/go-wasm-tools}
mkdir -p "$tools" "$(dirname "$out")"

wasm_tools=$tools/bin/wasm-tools
if ! "$wasm_tools" --version 2>/dev/null | grep -qx "wasm-tools $WASM_TOOLS_VERSION"; then
	echo "installing wasm-tools $WASM_TOOLS_VERSION into $tools" >&2
	cargo install --quiet --locked --root "$tools" "wasm-tools@$WASM_TOOLS_VERSION"
fi

adapter=$tools/wasi_snapshot_preview1.reactor-$WASMTIME_VERSION.wasm
if ! echo "$ADAPTER_SHA256  $adapter" | sha256sum -c --status 2>/dev/null; then
	echo "downloading the Wasmtime $WASMTIME_VERSION preview1 adapter" >&2
	curl -fsSL -o "$adapter.part" \
		"https://github.com/bytecodealliance/wasmtime/releases/download/v$WASMTIME_VERSION/wasi_snapshot_preview1.reactor.wasm"
	echo "$ADAPTER_SHA256  $adapter.part" | sha256sum -c --status ||
		{ echo "adapter checksum mismatch" >&2; rm -f "$adapter.part"; exit 1; }
	mv "$adapter.part" "$adapter"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
(cd "$sdk" && go build -o "$work/componentize" ./cmd/componentize)
(cd "$provider" && GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared -trimpath -o "$work/core.wasm" .)
"$work/componentize" "$wasm_tools" "$adapter" "$sdk/wit" "$work/core.wasm" "$out"
"$wasm_tools" validate "$out"
