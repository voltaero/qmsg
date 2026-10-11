# qmsg for iOS

A SwiftUI client for the qmsg orchestrator, for iPhone and iPad on iOS 18 or
newer. It uses the existing [native app API](../../crates/qmsg-orchestrator/APP_API.md).

The app pairs by QR code or pasted JSON, keeps its device token in Keychain,
shows accounts and channels, and streams messages while it is active. Chats
include older history, replies, reactions, text sends, file uploads, pending
send status, and attachment preview and sharing. Account, channel and message
forms use the actions each provider offers. Settings lists and revokes devices.

## Build on a Mac

Use Xcode 26 or newer with a Swift 6.2 toolchain. The package locks dependencies
in `QmsgKit/Package.resolved`. The app target uses Swift 5 language mode with
complete concurrency warnings; the client package uses Swift 6.

```sh
brew install xcodegen
cd apps/ios
xcodegen generate
open Qmsg.xcodeproj
```

Select the `Qmsg` scheme. For a physical device, choose your signing team in
Signing & Capabilities and change the bundle ID if needed. Then build and run.
The generated project is ignored; `project.yml` is its source.

To check an unsigned simulator build:

```sh
xcodebuild -project Qmsg.xcodeproj -scheme Qmsg \
  -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build
```

## Connect a phone

The phone must reach the endpoint in the pairing code. `localhost` on a phone
means the phone itself. Follow the API's LAN TLS or HTTP/2 tunnel setup, then
start your orchestrator with your providers.

On the server, create a fresh, one-use pairing code:

```sh
cargo run -p qmsg-orchestrator -- pair orchestrator.toml
```

Scan its QR code in the app or paste the full JSON payload. The camera scanner
needs a supported device and camera access; pasted JSON also works in the
simulator. The app checks the server certificate pin and endpoint host, and
uses normal certificate trust when the payload has no private certificate.

Tap an account in Chats to see its login state and account actions. Browser
login requires a viewer URL reachable from the phone, as described in the API
setup guide. Long login calls allow ten minutes. Structured function inputs
outside the built-in text, integer, boolean, choice and text-list fields use
qmsg's tagged `Value` JSON (for example `{"Text":"hello"}`).

## Tests

`QmsgKit` builds and runs on macOS or Linux with Swift 6.2:

```sh
cd apps/ios/QmsgKit
swift test
```

The live tests run only when `QMSG_PAIRING` contains a fresh pairing payload.
They need an already running orchestrator with a ready provider named `echo`,
connected to a TCP server that answers each line with `echo: <line>` and echoes
file payloads. Run the pairing command from the same directory as your server
config so its relative data paths match:

```sh
QMSG_PAIRING="$(/path/to/qmsg-orchestrator pair orchestrator.toml --json)" \
  swift test --package-path /path/to/qmsg/apps/ios/QmsgKit
```

These tests cover pairing, watch events, history, sends, upload/download,
action replay, rotation, revocation, certificate pins, and host checks. They
create and revoke a test device. Regenerate the checked-in protobuf bindings
with `scripts/generate-proto.sh` after changing the API; it needs `protoc`.

## Current limits

The app suspends its watch stream in the background and resumes from its cursor
when active. It has no push notifications or offline message cache. Formatted
messages show their plain text; email HTML and custom content are not rendered.
A delivery-unknown send stays visible until a server event settles it or you
remove it. Check the chat before composing another copy. Pending sends live in
memory and do not survive quitting the app.

The client and live tests have been checked on Linux. The SwiftUI sources pass
Swift's syntax check, but an Xcode build and a device/simulator check still need
to run on a Mac.
