# qmsg web app

A browser client for the qmsg orchestrator, alongside the [terminal app](../../crates/qmsg/README.md)
and the [iOS app](../ios/README.md). Its layout takes inspiration from
[T3 Code](https://github.com/pingdotgg/t3code): accounts, organizations and
channels in a sidebar, a floating composer, and a command palette.

It talks to the orchestrator's app API with gRPC-Web, in the same JSON model as
the native clients ([app API guide](../../crates/qmsg-orchestrator/APP_API.md)).

## Try it

```sh
bun install
bun run dev
```

Open the printed address and choose **Try the demo**. The demo runs offline
with made-up accounts and never contacts a server.

## Connect to an orchestrator

During development, Vite forwards the API to `http://127.0.0.1:50051`; set
`QMSG_API` for another address. Start the orchestrator, then create a pairing
code and paste its JSON into the pairing screen:

```sh
cargo run -p qmsg-orchestrator -- orchestrator.toml
cargo run -p qmsg-orchestrator -- pair orchestrator.toml --json
QMSG_API=http://127.0.0.1:50051 bun run dev
```

For everyday use, build the app and let the orchestrator serve it beside the
API, so the page and the API share an origin:

```sh
bun run build
```

```toml
[app]
listen = "127.0.0.1:50051"
web_dir = "apps/web/dist"
```

Then open `http://127.0.0.1:50051`. On a LAN listener with its own
certificate, the browser warns about the self-signed certificate once; that
browser has no pinned fingerprint as the native apps do, so check the
certificate before accepting it, or use a tunnel with a public certificate.

To host the files elsewhere, add the page's origin to `app.web_origins`, such as
`["https://chat.example.com"]`. The paired endpoint must then be reachable from
the browser, over HTTPS unless it is on localhost.

## What it does

- Pairing, token rotation before expiry, and recovery of a lost rotation reply.
- One resumable `Watch` stream with backoff; the directory, presence and
  provider status stay live. Reloading the page reopens the linked conversation.
- History pages as you scroll, de-duplicated with live events; edits,
  deletions, reactions, replies, typing and read markers.
- Sending text, mentions (Discord and echo) and files, with pending sends that
  can be retried or discarded and an honest "maybe sent" state.
- Images, video, audio and files from public URLs or backend blobs, read in
  256 KiB chunks, at most four at a time, and released after reading.
- Provider-declared functions as forms: sign-in (links, QR codes, OAuth through a
  pop-up), message actions, new conversations and custom actions, with
  completion and verification helpers.
- Mail folders with a reading pane. Mail HTML is sanitized, shown in a
  sandboxed frame without scripts, and remote images stay blocked until asked for.
- Account management, paired devices, light and dark themes, and optional
  notifications while the tab is in the background.

## Keys

| Key | Action |
| --- | --- |
| ⌘K / Ctrl+K | Jump to a conversation, account or command |
| Alt+↑ / Alt+↓ | Previous / next recent conversation |
| ⌘, / Ctrl+, | Settings |
| Enter / Shift+Enter | Send / new line |
| Esc | Close a dialog, cancel a reply or edit |

## Security

A page has no OS keychain. The device token lives in this origin's local
storage, so anything that can run script on the origin can read it; serve the
app only from an origin you control. Unpairing revokes the token on the server.

## Checks

```sh
bun run check   # TypeScript
bun run test    # unit tests
bun run build
```

`bun run generate` regenerates `src/gen/app_pb.ts` from the protobuf contract.
