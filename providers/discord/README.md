# Discord provider

Connects a Discord user account, using
[ningen](https://github.com/diamondburned/ningen) and
[arikawa](https://github.com/diamondburned/arikawa). It is written with the Go
SDK and runs only as a Wasm component:

```sh
sdk/go/build.sh providers/discord target/go/discord.wasm
```

```toml
[[provider]]
name = "discord"
wasm = "target/go/discord.wasm"
```

## Signing in

With no stored session the provider reports `LoginRequired` with its `login`
function. Calling it starts a browser login with the orchestrator's `discord`
recipe; the orchestrator reports `LoginRequired` with the viewer link while
the user signs in, then hands the provider the token, which it stores as the
`token` secret and connects with. The call answers once the token is stored,
so call it without a short timeout; giving up on it closes the browser. A
restart connects with the stored token. A token Discord refuses, or
`logout`, deletes it and asks to sign in again. There is no form to paste a
token.

## What works

Tested against a local Discord-like REST API and gateway over TLS
(`fakediscord`, in `crates/qmsg-orchestrator/tests/discord.rs`):

- Signing in through the browser login service, then restoring the session
  after a restart; cancelling a login, and one at a time.
- The directory on READY: the account, direct messages, guilds with their
  members, profile pictures, server icons, channel topics, and the channels
  the account can see, accepting text and media where it may send.
- Live messages, edits, deletes, reactions, typing, read markers and presence.
- Streaming host-blob attachments through multipart requests, plus captions,
  spoilers, dimensions, previews and signed URL expiry on received media.
- Paged history, newest first, and single messages.
- Sending, with a nonce that comes back on the message; a send cancelled
  before submission is reported `NotSent`. Once the REST call starts, a
  timeout or cancellation leaves delivery unknown; a later gateway message
  can still confirm the send.
- Edit, delete, react, unreact and mark read, offered by permission and
  authorship; typing.
- Finding, verifying and completing known users, and opening direct
  messages.
- Resuming after the gateway drops, and logging out when the session is
  revoked.

Not done: editing attachments, rendering/editing Discord embeds and stickers
as native qmsg types, group direct messages beyond opening them, threads and
forums beyond listing them,
Discord's rate limit details (`RateLimited` has no retry time), and mark
unread. Live QR sign-in, gateway readiness, directory sync and provider
function discovery passed with a real Discord account. A live DM test also
passed user lookup by id, verification, autocomplete, the last five messages,
sending, editing, adding/removing a reaction and deletion. Uncached message
fetches use history around the id, since Discord restricts its single-message
endpoint to bots.

## Live smoke test

After building the component, run from the repository root:

```sh
cargo run -p qmsg-orchestrator --example discord_login
```

Open its `LOGIN_VIEWER` link and scan Discord's QR code. The test uses the OS
keychain to protect the session in `target/live-discord`, prints status and
counts, and sends no messages. Ctrl-C stops it. Run it again to check that
the stored session connects without another login.

To read the last five messages with one known user, use:

```sh
cargo run -p qmsg-orchestrator --example discord_dm -- <username>
```

This checks lookup, verification and autocomplete, requires one matching
user, opens their DM and prints the five messages. Adding `--mutate` sends
one marked test message, edits it, adds/removes a reaction and deletes it.
It checks the sent and edited content and confirms the message is gone.

## Testing settings

`test_dial` (`host=ip:port,...`) sends connections for a host elsewhere, and
`test_ca_pem` replaces the trusted roots; TLS still checks the real host
name. They exist only for tests against `fakediscord`.
