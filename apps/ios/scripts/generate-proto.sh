#!/usr/bin/env bash
# Regenerates QmsgKit's Swift code from the orchestrator's app.proto.
#
# Needs `protoc` and a Swift toolchain. The plugins are built from the exact
# package versions pinned in QmsgKit/Package.resolved, so the generated code
# always matches the runtime libraries.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
proto_dir="$here/../../crates/qmsg-orchestrator/proto"
out="$here/QmsgKit/Sources/QmsgKit/Generated"

cd "$here/QmsgKit"
swift build -c release --product protoc-gen-swift
swift build -c release --product protoc-gen-grpc-swift-2
bin="$(swift build -c release --show-bin-path)"

mkdir -p "$out"
protoc \
  --proto_path="$proto_dir" \
  --plugin=protoc-gen-swift="$bin/protoc-gen-swift" \
  --plugin=protoc-gen-grpc-swift="$bin/protoc-gen-grpc-swift-2" \
  --swift_out="$out" \
  --swift_opt=Visibility=Public \
  --grpc-swift_out="$out" \
  --grpc-swift_opt=Visibility=Public,Server=false,Client=true \
  "$proto_dir/app.proto"
echo "wrote $out"
