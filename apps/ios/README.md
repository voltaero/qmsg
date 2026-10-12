# qmsg for iOS

A SwiftUI client for the qmsg orchestrator, for iPhone and iPad on iOS 18 or
newer. It uses the existing [native app API](../../crates/qmsg-orchestrator/APP_API.md).

The app pairs by QR code or pasted JSON, keeps its device token in Keychain,
shows accounts and channels, and streams messages while it is active. Chats
include older history, replies, reactions, text sends, file uploads, pending
send status, and attachment preview and sharing. Account, channel and message
controls use the actions each provider offers. Accounts manages chat networks;
Settings manages the server connection and paired devices.

## Build and sideload on Linux

The Podman wrapper uses Swift 6.4, xtool 1.20.1, and usbmuxd, following the
working Garmin app setup. It passes the connected phone's USB bus into the
container. The first run builds the image; setup stores the SDK, Apple login,
and phone pairing outside the repo.

```sh
apps/ios/xtool.sh setup
apps/ios/xtool.sh devices
apps/ios/xtool.sh dev run --usb
```

After installation, open qmsg on the phone. xtool may add its signing-team
prefix to the installed bundle ID.

You can reuse a configured xtool state directory instead of logging in and
importing the SDK again. On Claudio's machine:

```sh
QMSG_IOS_STATE="$HOME/.local/share/tub-xtool" apps/ios/xtool.sh dev run --usb
```

`QMSG_IOS_IMAGE` can select another prepared image. `Package.swift` and
`xtool.yml` define the Linux app build; `project.yml` defines the Xcode build.
The linker SDK version matches the existing iOS 27.1 SDK. Build files, IPAs,
credentials, and pairing records stay out of git.

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

## Using the app

The app has Chats and Settings, plus Mail when a mail account is set up:

- **Chats** lists recent chats, servers and direct messages across accounts.
  Opening a server shows its channels grouped by category.
  Images use provider previews; large originals without previews download only
  when opened. Small images can load inline.
  Chips at the top filter by account. Accounts that need you, such as one
  waiting for sign-in, show as a banner that opens that account.
- **Mail** lists folders by account, Inbox and special folders first.
  A folder opens as a message list, newest first; a message opens with its
  sender, recipients, body and attachments. HTML mail renders with scripts,
  network loads and remote images blocked; links open in Safari. Reply,
  compose, star, read state, move and trash use the provider's mail actions.
  Mail lists load up to 50 headers per page and fetch a full body when you open
  a message. Gmail Inbox includes all categories and shows individual messages;
  Gmail on the web can show only Primary and group replies into a thread.
  Reopening a folder refreshes its contents; external moves also remove loaded
  rows during syncing. Large mail does not block the rest of the list.
- **Settings** lists accounts first, then the server and paired devices.
  Tap **Add Account** to pick a network your server has installed, give it a
  name like "Work", and fill in any connection fields. You can add several
  accounts of the same network. Open an account to sign in, rename it, edit
  its connection details, turn it off, sign out or remove it. Swipe a row to
  turn it off or remove it. Removing asks first.
  Settings also lets you unpair this phone.

Tap the compose button in Chats to search contacts and start a conversation.
Long-press a message to reply, react, copy, edit or delete. Type **@** in a
Discord or Echo chat to pick someone to mention; mentions in messages show as
names. Server channels read as a feed, direct chats as bubbles. Tap **Members**
to see the chat's members or the available server members. The chat stays
where you are while reading older messages and shows a jump-to-latest button
instead of scrolling for every new message. A send that
fails shows **Not sent** with **Retry**. A send the server couldn't confirm
shows **Could not confirm delivery** with **Reload chat**; it may have gone
through, so the app never resends it on its own. Browser login needs a viewer
URL reachable from the phone, as described in the API setup guide. Long login
calls allow ten minutes.

## Connect Discord

Build the provider with `sdk/go/build.sh providers/discord target/go/discord.wasm`
and add it to your server config as shown in the
[Discord provider guide](../../providers/discord/README.md).
For a phone, set `browser.port` and an HTTPS `browser.viewer_url`, and forward
only `/view/` to the local browser service. See the
[browser login guide](../../services/browser-login/README.md).

In qmsg, tap the **Sign in to Discord** banner in Chats (or open the account in
**Accounts**), then **Sign in** and **Open sign-in page**.
The page opens inside the app. For a private server certificate, the viewer
must use the server's paired host and certificate. The app checks the saved
certificate pin and normal TLS checks before loading it. Complete Discord's
login there; the server stores the session and starts syncing your chats.
To add a second Discord account, tap **+** in **Accounts**, choose Discord and
give it a different name, then sign in to it the same way.

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

The client and live tests pass on Linux. The full iOS app also builds with the
Garmin xtool toolchain, signs, and installs on the connected iPhone. The Linux
build needs an explicit Quick Look SwiftUI overlay import for file previews.
Xcode and simulator builds have not been checked.

## Gmail sign-in

Install the email component on the server and register a Google iOS OAuth client
for the app's **signed** bundle ID. In the `gmail` provider template, set
`service = "gmail"`, `oauth_client_id` and `oauth_redirect_uri`. See
[`providers/email/README.md`](../../providers/email/README.md) for callback setup.
Keep the corresponding callback scheme in `Qmsg/Info.plist`.

Accounts → Gmail → Sign in with Google uses the system authentication browser.
The callback's scheme, path and state must match; qmsg sends the code to the paired
server, where PKCE verification, token exchange and mail login run. Google
passwords and mail tokens never enter the sign-in form.

While the Google project is in Testing, add the intended account in Google Auth
Platform → Audience → Test users. Live Google consent still needs a user; the
fake mail tests cover the iOS callback, code exchange, refresh and XOAUTH2.
