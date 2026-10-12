# Email provider

One Go WASM component supports general IMAP/SMTP accounts and Gmail over
IMAP/SMTP with OAuth 2.0. All mail networking and token refresh run inside WASM.
The native client opens sign-in links; the host stores credentials in its secret store.

Build from the repository root, with the host and other providers:

```sh
sdk/go/build.sh providers/email target/go/providers-email.wasm
```

## General mail account

```toml
[[provider]]
name = "email"
wasm = "target/go/providers-email.wasm"
[provider.settings]
service = "imap"
address = "you@example.com"
imap_addr = "imap.example.com:993"
smtp_addr = "smtp.example.com:465"
smtp_tls = "tls" # or "starttls", usually with smtp_addr port 587
# sent_mailbox = "Sent"  # needed if the server does not label its Sent folder
# trash_mailbox = "Trash" # needed if the server does not label its Trash folder
# ca_pem = "..." # optional extra CA for a private mail server
```

The `login` function takes a secret `password` input. It checks IMAP login before
saving it. SMTP authentication happens on send. Both services use the account
address as their login name and the same password. IMAP requires implicit TLS;
SMTP requires implicit TLS or STARTTLS. Certificate checks cannot be disabled.
`test_dial` is a fixture-only host redirect; omit it for real accounts.

## Gmail OAuth

```toml
[[provider]]
name = "gmail"
wasm = "target/go/providers-email.wasm"
[provider.settings]
service = "gmail"
address = "you@gmail.com"
oauth_client_id = "YOUR_QMSG_DESKTOP_CLIENT_ID"
# oauth_client_secret = "..." # if supplied with the registered desktop client
```

Gmail defaults to `imap.gmail.com:993` and `smtp.gmail.com:465`. It uses XOAUTH2,
not a password. Google documents the endpoints and full-mail OAuth scope in
[IMAP/SMTP OAuth](https://developers.google.com/workspace/gmail/imap/imap-smtp)
and [XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol).

Use a Google **Desktop app** OAuth registration owned by the qmsg project. The
project maintainer configures it once; users should only choose their account and
sign in. We have not registered a production qmsg client yet. Do not copy
Thunderbird's client ID. A bundled desktop client secret is not confidential.
Allow test accounts in the consent screen while the Google project is in testing.

There is no hosted OAuth callback, public hostname, or Gmail browser-service
setup. The native desktop client follows these steps over the paired app API:

1. Listen temporarily on `127.0.0.1` (or `[::1]`) on a free port, on the **client
   device**. Choose a callback path and keep the listener alive until completion.
2. Discover `login` and call it with `redirect_uri`, for example
   `http://127.0.0.1:43123/callback`. The typed result contains `url`, `state`, and
   `expires_at` (Unix seconds). Open `url` in the user's regular browser.
3. Check the callback path and state. Discover `finish_login` and submit `code`
   and `state` over the authenticated app connection. Never log the callback code.
   On denial or cancellation call `cancel_login`, then close the listener.
4. qmsg exchanges the code with its private PKCE verifier inside WASM, checks
   IMAP access for the configured address, and stores credentials privately.
   Wait for `Ready`; close the listener. Expired/replayed codes need a fresh login.

Only one pending login exists per provider. Starting another replaces it. Logins
expire after ten minutes; cancellation and provider shutdown discard the verifier.
The client receives neither the verifier nor the resulting access/refresh tokens.
The callback stays on the client device even when qmsg runs on a remote LAN host
or behind a tunnel. Gmail never starts the headless browser service.

This desktop flow follows [Google's native-app OAuth guide](https://developers.google.com/identity/protocols/oauth2/native-app).
iOS uses its own Google **iOS** client registration and the system authentication
browser. Set `oauth_client_id` to that client ID, and set `oauth_redirect_uri` to
`com.googleusercontent.apps.<client-ID-prefix>:/oauth2redirect` (remove the
`.apps.googleusercontent.com` suffix from the client ID). Register the same scheme
in the app's `CFBundleURLTypes`. The registered bundle ID must match the signed app;
xtool's installed bundle ID may have an `XTL-` prefix.

With `oauth_redirect_uri` set, `login` takes no callback input. qmsg opens Google's
page with `ASWebAuthenticationSession`, checks the returned scheme, path and state,
and submits the code through `finish_login`. PKCE and tokens stay on the server.
The provider rejects any other callback, including desktop loopback callbacks.
Do not use a desktop registration to bypass Google's mobile redirect rules.

Revoked refresh tokens return the account to `LoginRequired`. `logout` deletes
local credentials; it does not revoke the Google grant. Google project
verification, consent limits and refresh-token lifetimes still apply. Live Google
sign-in has not yet been tested with a real account.

## Model and actions

An account is an organization. Selectable mailboxes are channels with
`kind = Custom("mailbox")` and typed `mailbox` attributes/delimiter. A message ID
is `UIDVALIDITY:UID`, scoped to its mailbox. Recreating a folder invalidates old
message IDs and cursors instead of reading or changing a different message.

`Message.email` holds subject, From/To/Cc/Bcc/Reply-To addresses, RFC Message-ID,
In-Reply-To/References, HTML, Content-IDs by content index, and read/starred/draft
flags. Plain text and attachments use the shared content/media fields. HTML is
untrusted: a client must sanitize it, block scripts and remote image loads, and
resolve `cid:` references through the message's content. qmsg does not fetch
remote images, invent profile pictures, or turn RFC thread references into
channel-local reply IDs. Clients can group threads using RFC IDs and references.

Provider, account and mailbox contexts expose `compose`: required `to`,
`subject`, `content`; optional `cc`, `bcc`, `html`. Recipients are lists of
addresses. `reply` on a message keeps RFC thread references and prefers Reply-To.
A successful send returns a typed record:

```json
{"accepted": true, "message_id": "...", "sent_copy_saved": true, "warning": ""}
```

`accepted` means SMTP accepted the mail, not that the recipient received it.
General SMTP saves a Sent copy through IMAP if the folder is known. A failed
Sent save still returns `accepted: true` with a warning, so clients do not resend.
Gmail saves its own Sent copy; qmsg does not append a second copy. A dropped
connection after SMTP DATA starts returns `delivery_unknown`: check Sent before
retrying. Bcc recipients go only in the SMTP envelope, never in headers.

Message functions also include `mark_read`, `mark_unread`, `star`, `unstar`,
`move` when the server supports MOVE, and `trash` when a Trash folder is known.
`trash` declares the standard delete action and means moving out of the current
folder into Trash. No action permanently expunges messages, and qmsg never runs
a global EXPUNGE. It does not offer editing for received mail.

`verify_address`, `complete_address`, `find_addresses` work outside actions too.
Address suggestions learn from mail read through qmsg, not a remote contacts
API. `complete_mailbox` suggests destinations for moves. Verification checks
address syntax; it cannot prove that an address exists or will accept mail.

## Bounds and sync

- 8 MiB encoded outgoing mail and incoming raw/decoded mail; 20 outgoing
  attachments, 100 recipients and 100 incoming MIME parts (including containers),
  with at most 30 nesting levels and 64 KiB in the main header.
- History returns up to 20 header-only messages in arrival-date order, with
  `Custom("email-summary")` in their content. Use `GetMessage` when opening one
  to fetch its body and attachments. Reply and move actions fetch headers only.
  Headers load in one IMAP request; bodies
  over the mail limit no longer prevent the rest of a folder from loading.
  Pagination uses date order, even when old mail has been restored to Inbox.
  The most recently visited folder's ordering is cached until its UID set changes.
- 100,000 messages per selected mailbox, 1,024 folders, 2,000 learned addresses.
- 16 queued/running command workers with one mail operation at a time. Each
  operation opens its own connection, bounds network time and closes on cancel.
- A 60-second folder poll retries connection failures. It compares each
  folder's `STATUS` (UIDVALIDITY, UIDNEXT, MESSAGES, UNSEEN, and HIGHESTMODSEQ
  when the server has CONDSTORE) with the previous poll and emits `Gap` only for
  folders that changed, new folders included. A failed poll keeps the previous
  state, so changes made while disconnected are reported after reconnecting.
  Unchanged polls repeat neither the directory nor the `Ready` status. Without
  CONDSTORE, a star changed by another client is not detected. Apps reload
  visible history on a gap. This is polling, not IMAP IDLE push.

Large-mail lazy part downloads, draft editing, server-side message search,
Gmail-specific label/thread IDs and alternate SMTP credentials are not implemented.
Gmail labels appear as IMAP folders. These limits are explicit rather than
silently truncating mail or claiming unsupported actions.

## Tests

From `providers/email`, `go test ./...` starts temporary TLS IMAP/SMTP servers
with an in-memory mailbox. It covers MIME, attachments, Bcc, flags, paging,
UIDVALIDITY, MOVE, poll change detection, TLS/STARTTLS, cancellation, Sent-copy failure, unknown SMTP
delivery, Gmail defaults, PKCE, callback validation, replay/expiry/cancellation, OAuth refresh
and XOAUTH2. No live account is used.

From the repository root:

```sh
cargo test -p qmsg-orchestrator --test email
```

The Rust tests build and run the real WASM component through Wasmtime. The Gmail
test calls the native login functions against a TLS token endpoint, then checks
code exchange, refresh, IMAP and SMTP XOAUTH2 over WASM sockets. No browser-service
process runs for Gmail.
