# Browser login

Host-side login service for qmsg. Providers remain in WASM and select a
host-owned login recipe. The service uses Playwright and headless Chromium;
it needs no desktop or display server. Discord defaults live in
`src/defaults.ts`. Each login gets a fresh browser context. Session tokens
belong in qmsg's secret store, not browser profiles.

Run from this directory with Bun:

```sh
bun install --frozen-lockfile
bun start
```

The host supplies `QMSG_BROWSER_KEY` (a private random key of at least 32
characters) in the environment. The service binds only to `127.0.0.1` on
an available port and prints one JSON line with its address. Set
`QMSG_BROWSER_PORT` to choose a port. Chromium installs automatically on
first use. Downloads need network access. Missing Linux libraries still
require the server administrator to install Playwright's OS dependencies;
the service never changes system libraries. SIGINT/SIGTERM closes contexts
and the browser. The Rust orchestrator starts this service on demand, generates
its key, and stops it on shutdown. Bootstrap installs pinned Bun dependencies
when missing.

The host API requires `Authorization: Bearer <QMSG_BROWSER_KEY>`:

| Request | Result |
| --- | --- |
| `POST /sessions` with `{"provider":"discord"}` | `id`, viewer `path`, `expiresAt` |
| `GET /sessions/:id` | `state`: starting, active, complete, failed |
| `POST /sessions/:id/claim` | One-time `{"token": "..."}` after completion |
| `DELETE /sessions/:id` | Cancels and closes the login |

The host publishes the viewer path in `LoginRequired.link`, then polls state,
claims the token, stores it as a provider secret, and closes the login on
cancel/provider exit. Keep the management key inside the host. Forward only
`/view/` routes through the server's authenticated HTTPS client connection;
never expose `/sessions` or a browser debugging port. Set the orchestrator's
`browser.viewer_url` to the public base URL and `browser.port` to the port
your proxy forwards. The host passes its trusted public origin in
`QMSG_BROWSER_VIEWER_ORIGIN`; the service does not trust forwarded headers.
The opaque viewer URL
grants control of that login, so do not log or share it. Internal responses
and screenshots disable caching. Tokens never reach the viewer page.

The viewer shows screenshots and accepts clicks, text, Tab, Enter, and
scrolling. Requests to control a browser must come from the viewer's origin.
The page does not save text. There are at most four sessions, including
unclaimed results, and they expire after ten minutes. A busy browser rejects
input instead of building a queue. A host can import `LoginService` to set
different limits or add trusted provider recipes.

Discord's recipe observes successful official login/MFA response JSON for
the token. QR sign-in uses the Authorization header of a successful signed-in
GET to Discord's own `/api/v*/users/@me` routes in this fresh context. It does
not scrape existing browser sessions or bypass challenges. Codes and CAPTCHA
require user input. Live QR sign-in passed with a real Discord account.
Password/MFA flows have local Chromium tests; passkeys and popups remain
untested.

```sh
bun run check
bun test
```

## Gmail OAuth

Gmail does not use this service. The native client receives its callback locally
and submits the code through qmsg's paired API. The email WASM provider handles
PKCE, code exchange and token refresh. See the
[email setup guide](../../providers/email/README.md).
