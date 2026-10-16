# Native app API

The orchestrator serves `qmsg.app.v1.App` over gRPC, and over gRPC-Web for
browsers on the same port. Native desktop and mobile clients generate bindings
from [app.proto](proto/app.proto); the [web app](../../apps/web/README.md) uses
gRPC-Web. Provider WASM uses the existing ABI 3; rebuild Rust and Go providers
when changing its types during development.

The protobuf envelope carries JSON for qmsg domain data. This keeps function
forms, results and provider extensions in one shared model. Generate its JSON
Schema bundle with `cargo run -p qmsg-orchestrator -- schema`. The individual
[checked-in schemas](proto/schema/) are ready to use without a Rust build. Use both the protobuf
contract and JSON schemas when building a client. JSON enums use Serde's external
tag, for example `{"History":{"channel":{"organization":null,"channel":"3100"},"cursor":null,"limit":50}}`.
Treat provider IDs, message IDs and blob handles as opaque strings. Decode integer
fields with a lossless JSON decoder; timestamps, sizes and sequences are unsigned
64-bit values and generic `Value::Integer` is signed 64-bit.

## Network setup

The default `[app]` listener is `127.0.0.1:50051`. This suits an app on the host,
or a secure tunnel forwarding that port. A plaintext connection is allowed only
on loopback. The tunnel must carry HTTP/2, not only HTTP/1.1. Set `app.endpoint`
to the address as seen by the client, including the forwarded port.

For a direct LAN connection:

1. Run `cargo run -p qmsg-orchestrator -- tls .qmsg qmsg.local 192.168.1.10`.
   This creates owner-only PEM files and refuses to overwrite them.
2. Set `app.listen = "0.0.0.0:50051"`,
   `app.endpoint = "https://qmsg.local:50051"`,
   `app.certificate = ".qmsg/app-cert.pem"`, and
   `app.private_key = ".qmsg/app-key.pem"` in your config. Use your real LAN name
   and IP. The name or IP in the endpoint must appear in the certificate.
3. Run `cargo run -p qmsg-orchestrator -- orchestrator.toml`.

A TLS tunnel or reverse proxy can terminate TLS and forward HTTP/2 to localhost.
Use its `https://` endpoint in pairing. A private LAN listener still requires its
own TLS identity. A wildcard listener requires an explicit, reachable endpoint.

Browser-based provider login is a separate connection. To use it from another
machine, set `browser.port` and `browser.viewer_url` and forward its `/view/`
path through your LAN TLS proxy or tunnel. Never forward `/sessions`. See the
browser service README. The login link is a short-lived browser capability.

### Browsers

The listener also accepts HTTP/1.1, so a browser can call the API with
gRPC-Web in binary mode (`application/grpc-web+proto`). gRPC-Web has no
streaming requests, so a browser sends an `Upload` as one request body with all
of its frames; the limits above still apply to each frame. Set `app.web_dir` to
a built web app to serve it at the same origin. A page on another origin needs
that origin in `app.web_origins`, such as `["https://chat.example.com"]`.
Origins are exact: scheme, host and port, without a path. Requests carry the
device token in the `authorization` header, never in cookies.

## Pairing and device tokens

On the server, run `cargo run -p qmsg-orchestrator -- pair orchestrator.toml`.
The terminal shows a QR code and its JSON payload. For tooling, `pair
orchestrator.toml --json` prints only JSON. Protect a saved pairing file with an
owner-only mode. A QR grants full access to this backend, so show it only to the
person pairing a device.

The payload has `version: 1`, `endpoint`, `code`, `expires_at_ms`, and optional
`certificate_pem` / `certificate_sha256`. A native client trusts the certificate
from the QR, checks its SHA-256 fingerprint and the endpoint's host name, and
keeps that trust with its device token. Do not disable certificate checks or
fetch trust material from an unauthenticated network endpoint. Public-CA tunnel
endpoints can use normal platform certificate validation instead.

`Pair(code, device_name)` needs no bearer token. The code contains 256 random
bits, expires after five minutes, and works once. Only the local server CLI can
mint it. At most eight valid pairing grants and 128 devices exist at once.
The reply contains `device_id`, `token`, and `expires_at_ms`.

Every other RPC needs `authorization: Bearer <token>` metadata. Tokens contain
256 random secret bits, expire after 90 days, survive server restart, and can be
renewed with `Rotate`. The database keeps hashes of pairing codes and tokens.
Keep device tokens in the app's OS keychain. Every paired device has the same
backend permissions; this is single-owner device auth, not a multi-user service.

`Devices` lists paired devices; `Revoke(device_id)` removes access, including
one's own. `Rotate` replaces the old token at once. For reliable rotation,
generate 32 random bytes on the client, persist them as a pending credential,
and pass their lowercase hex encoding in `replacement_secret`. The proposed
token is `<device_id>.<replacement_secret>`. If the reply is lost, try that token
with `Snapshot`; if it fails, retry rotation with the old token. Keep the pending
credential until one succeeds. An empty replacement lets the server generate
one, but a lost reply then needs a new pairing code.

Streams check token validity before yielding each event and at least every
15 seconds while the client reads. Expiry, rotation or revocation ends the
stream with `UNAUTHENTICATED`; reconnect using a valid token.

## Chat accounts

`ProviderCatalog` returns JSON with `types` and `accounts`. Each installed type
has an `id`, `label`, connection `fields` and `defaults`. A field has `id`,
`label`, `required` and `secret`. Accounts have a stable `name`, an editable
`label`, a `type`, `enabled` and a string map of public `settings`.

`ConfigureProvider` adds or updates an account. Pass those fields and encode
the settings map in `settings_json`. An empty label keeps the old label.
The server accepts only installed types and their listed connection fields;
provider sign-in stays in the existing function and login-step flow. It never
returns saved tokens in the catalog. Turning an account off stops its provider
and keeps its saved sign-in. Turning it on starts a new runtime instance.

`RemoveProvider(name)` stops the account and deletes its saved sign-in on this
server. It does not delete the account or messages on the chat network.
These RPCs require a paired device token. The CLI enables account management;
embedded servers created with `Service::new` return `UNIMPLEMENTED` for it.

The config's provider entries supply installed components and initial accounts.
After the first change, `<data_dir>/providers.json` holds the account list and
overrides those initial accounts on restart. WASM paths remain server settings.
An account whose type is no longer installed, or whose component fails to
start, is logged and left stopped; the server and other accounts still start.
Adding, restarting or removing an active account sends a fresh `Watch` snapshot;
a resumed cursor that predates such a change also gets a snapshot.

## State and reconnects

Call `Watch` with an empty epoch to start. The first event is a full directory
snapshot: providers, their status, organizations, users, channels, presence,
and the account's own user ID. Users and channels inside a scope are maps keyed
by their provider IDs. Organizations are a list. A standalone scope has no
organization, as with Discord DMs.

Keep `(epoch, sequence)` from each snapshot or update, and resume with them
after a connection loss. Sequences belong to this server run, not to a provider.
The server keeps the most recent 2,048 updates, up to 64 MiB. It subscribes and
captures the snapshot under one lock, so changes cannot slip between them.
A changed epoch, stale cursor or cursor ahead of the server returns a fresh
snapshot. Replace local directory state and reload the visible channel's
history; the snapshot does not contain messages or read cursors. A provider
`Gap` also requires a history refresh. De-duplicate messages by channel and ID.

Heartbeats carry the last delivered sequence. They do not advance a cursor
past queued updates. A slow reader receives `ABORTED` when its queue of 128
updates fills; reconnect with the last applied cursor. The server keeps running
providers without waiting for a slow app. `Snapshot` also includes the epoch and
sequence, which can be passed directly to `Watch`.

Provider references contain a configured `name` and runtime `instance`. Refresh
state on `FAILED_PRECONDITION` if that instance changed. Retry an action with
the refreshed instance and its original operation ID; the journal uses the
configured provider name, so restarting cannot run the action twice. Function discovery is
live; discover functions for the provider, organization, channel or message
context as needed. Login steps and their browser link come through provider
status. No provider session token, secret-store operation, kill or shutdown
command appears in the app API.

## Commands and results

`Execute` supports function discovery, function calls, named-input or standalone
verification and completion, history, one-message reads, sends and opening a
conversation. Pass function IDs and arguments, not function declarations; the
server resolves and checks the current declaration. Verification and completion
never run an action. History and completion page sizes must be 1..100. App
history pages sort oldest first; the cursor still requests older pages.

`Outcome.reply` contains JSON matching the `reply` schema. `error_json` matches
the `error` schema: a stable backend code, message, optional structured provider
`CommandError`, and `delivery_unknown`. Invalid inputs keep per-field issues;
provider errors can include typed details and rate-limit delay. Transport auth,
invalid requests, quota limits and unavailable media use normal gRPC status.
`observed_sequence` is the event position after draining events queued before
the provider answered; the app can wait for its stream to reach it before
assuming its view reflects an action.

Every action, including `Call` of an action function, needs `operation_id` and
its original `created_at_ms`. Keep both and the exact original command across
retries. IDs must be unique per paired device and at most 128 bytes. The durable
journal claims an operation before handing it to the provider. A repeat returns
the saved result; a changed payload with the same ID returns `ALREADY_EXISTS`.
An operation still running, interrupted by a crash, or left without a durable
reply reports `delivery_unknown` and never runs again. Inspect history or current
state before issuing a new action. Sends get a stable nonce when none is given.
`NotSent` confirms that a send did not start; other failures can leave delivery
unknown. The journal is not a promise of exactly-once platform delivery.

Operation timestamps can be at most seven days old and five minutes ahead.
Records older than seven days can be removed because the old request can no
longer run. There are limits of 5,000 records per device, 50,000 overall and
512 MiB of occupied device-database pages; new actions fail at those limits
with `RESOURCE_EXHAUSTED` before running. Action replies over 128 KiB cannot be
journaled and return `UNKNOWN`: the action ran, so check state.
Large reads belong in history or lookup calls, not an action result.

Requests default to 60 seconds; `timeout_ms` is bounded to 100..900,000 ms.
Use a longer timeout for a provider login. Timeouts or a lost connection do not
prove an action failed. Quotas are 64 active command/read requests globally,
16 slots per device (including streams), 64 streams globally, four uploads,
and four media reads. Give a device one long-lived stream. Native clients should
use HTTP/2 keepalive and reconnect with backoff. No connection needs to stay open
for token validity or operation recovery.

## Media and profile pictures

Users have display names, optional usernames and avatars. Organizations have
icons; channels have icons and topics. Images, video, audio and files use the
same `Media` model: name, MIME type, size, source and display metadata. Metadata
includes width, height, duration, expiry, preview URL, description and spoiler.
Unknown values stay absent. Render a fallback for unknown custom content kinds.
Discord exposes embeds and stickers as JSON custom content; it does not turn
them into editable native qmsg objects.

`Url` sources are public download URLs. The native app downloads them without
a provider token. Respect expiry and fetch the message again to renew a signed
URL. The backend does not fetch arbitrary app-supplied URLs. Treat all provider
URLs as untrusted input: render/download media, never execute it.

`Blob` sources in app data are opaque device-owned handles. Use `ReadBlob` in
chunks of at most 256 KiB and `ReleaseBlob` when finished. Handles last ten
minutes after a response or stream event grants them; refresh the message or
snapshot if one expires. One device cannot read or release another device's
handle. A provider's source stays alive while events, directory state or another
reader hold it. Inline media bytes are moved into handles before JSON encoding.
There are 256 handles per device, 4,096 overall, 8,192 retained provider sources,
and 64 MiB of retained inline media bytes.

To send media, stream `Upload`: the first frame contains provider, name, MIME,
size and optional data; subsequent frames contain only data. Each data chunk
has at most 256 KiB. Size must match exactly. Uploads are temporary owner-only
files, capped at 100 MiB each, 512 MiB per device and 120 seconds per stream.
The reply is a `Media` with a Blob source. Keep its handle until the action
finishes; use it only with the same device and provider instance. A received
provider blob must be downloaded and uploaded to send it elsewhere.

Discord currently accepts up to ten attachments, with a conservative provider
limit of 10 MiB each, plus a text caption of up to 2,000 code points. It streams
host uploads through multipart requests under WASM. Larger account-specific
Discord limits are not advertised yet. Attachment editing remains text-only;
function discovery states which edits are supported.

## Scope and limits

This endpoint targets native apps over a LAN or secure tunnel, and browsers
through gRPC-Web. It does not add push notifications, media transcoding or a server
message-search index. Those can be separate app needs after the native client
uses the API. Provider-specific features remain typed functions or custom
content rather than silent omissions.

Request frames have a 1 MiB limit; response frames have an 8 MiB limit. Very
large directories need a later paged-snapshot API. Event replay is in memory,
so restart means a fresh snapshot and history reload. Tokens and the action
journal persist in `devices.db`, with owner-only filesystem permissions. The
existing provider secret store uses the OS keychain when available and warns
when it falls back to plaintext; provision a working keychain on the server if
you need encryption at rest. Device-token auth does not change that storage rule.

Tests cover actual gRPC pairing/auth, rotation/revocation, restart persistence,
retry suppression, stream cursor ordering, device-isolated blob access, and an
upload through the WASM echo provider. Discord tests run the actual Go WASM
component against a local TLS REST/gateway fixture, including streamed multipart
media. They do not require or send messages to a real Discord account.

## Email

`Message.email` carries the mail envelope, RFC thread references, read/star/draft
flags and an optional HTML alternative. `Channel.mailbox` carries folder
attributes and its delimiter. Plain text and attachments keep the shared
content/media format, including device-owned media handles in gRPC replies.
HTML and remote image URLs are untrusted. Sanitize HTML and block automatic
remote image loading. Resolve `cid:` media using `email.content_ids`, whose keys
are stringified content indices. Group threads by RFC Message-ID/References;
mail IDs and history cursors are mailbox-scoped `UIDVALIDITY:UID`.

Discover `compose` and `reply` through Functions. Their typed result distinguishes
SMTP acceptance from saving a Sent copy. Treat `delivery_unknown` as an uncertain
send and check Sent before retrying. Email read/unread applies to one message;
these actions use custom kinds rather than the chat read-through contract.
The standard delete action, when offered, moves a message to Trash. See
[email provider setup and limits](../../providers/email/README.md).

A provider error can set a typed details record with `delivery_unknown: false`
when it can prove an action did not take effect. The API keeps this distinction
in its error envelope. Without that explicit proof it keeps its conservative
unknown-outcome behavior. The email provider uses this for a definite SMTP
rejection or a failure before DATA; loss after DATA remains uncertain.

### Gmail desktop sign-in

Gmail uses normal discovered provider functions, with no public callback route.
Login actions may declare typed results for each step; clients read their result
type from function discovery. Logout and other standard action contracts retain
their fixed result types.
The native client starts a temporary loopback listener on its own device, calls
`login` with `redirect_uri`, and opens the returned `url` in the regular browser.
The result also provides `state` and `expires_at` (Unix seconds). Validate the
callback path and state, then discover and call `finish_login` with `code` and
`state`. On denial, call `cancel_login`. Close the listener on completion,
cancellation or expiry. Use fresh operation IDs for a new login; reuse the same
ID only when retrying the same request. Codes are secret and must not be logged.
The provider keeps PKCE verifiers and exchanged tokens private. This works when
qmsg runs elsewhere on the LAN or behind a tunnel, since the callback belongs to
the client device. On iOS, a provider configured with `oauth_redirect_uri` takes
no callback input: the app opens the sign-in page in the system authentication
browser and submits the returned code and state the same way. See the
[email provider guide](../../providers/email/README.md).
