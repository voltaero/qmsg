# Go SDK

Write qmsg providers in Go. They run as Wasm components in the
orchestrator's Wasmtime like Rust ones; nothing runs natively.

```go
func init() { qmsg.Register(run) }
func main() {}

func run(cx *qmsg.Context) error {
	cx.Status(qmsg.StatusReady{})
	for {
		c, reqCtx, err := cx.NextCommand(context.Background())
		// answer commands, each on its own goroutine
	}
}
```

Build one with:

```sh
sdk/go/build.sh providers/discord target/go/discord.wasm
```

This builds the provider with Go (1.24 or newer) as a `wasip1` reactor
(`-buildmode=c-shared`), then turns it into a component with
`cmd/componentize`. The pinned tools live in `target/go-wasm-tools`
(`$QMSG_GO_TOOLS` to move them): `wasm-tools` 1.261.0, installed there with
`cargo install`, and the Wasmtime 49.0.2 `wasi_snapshot_preview1` reactor
adapter, downloaded and checked against its SHA-256. Nothing is installed
globally.

## How it works

- `export_wasip1.go` exports the world's `abi-version` and `run`.
  `cmd/componentize` wraps both so Go's runtime initializes first, since
  components never call `_initialize`.
- The canonical ABI allocates through `cabi_realloc`, which Go can't serve.
  `componentize` adds one in WAT: a bump allocator over a 4 MiB static array
  in `wasi/alloc.go`, found by its magic bytes. Imports that return lists are
  wrapped in `wasi.Lists`, which copies out and frees right after. The config
  passed to `run` also lands there, so it must fit in 4 MiB.
- `wasi/` calls `wasi:sockets`, `wasi:io` and `wasi:clocks` 0.2.12 directly,
  declared in `wit/go.wit`; the adapter provides the rest of WASI to Go's
  runtime.
- Go on Wasm has one thread. A goroutine waiting on a socket parks on a
  channel while one poller goroutine checks every waiting pollable without
  blocking, then sleeps through the Go scheduler, at most 10 ms, between
  checks. Other goroutines keep running, so commands, cancellations and
  events are handled while requests wait.

## Networking

Go's `net` package can't open sockets on `wasip1`. Use `qmsg.Dial`, which
has the signature of `net.Dialer.DialContext`, for `http.Transport`,
WebSocket dialers and the like, and `qmsg.Resolve` for lookups. Outside Wasm
they use the standard library. Wasm has no certificate store, so import
`golang.org/x/crypto/x509roots/fallback` for TLS.

`qmsg.Dial` connects to each address of a name in turn and honours the
context's deadline and cancellation. Connections honour read and write
deadlines, and `Close` wakes blocked reads and writes, waits up to a second
for written data to leave, then frees the socket's handles. One read takes at
most 64 KiB.

Tested in Wasmtime 49 by `crates/qmsg-orchestrator/tests/go_net.rs`, which
runs `internal/nettest`: name lookups and failed lookups, refused
connections, read timeouts, 300 connections in a row freeing their sockets,
an 8 MB HTTPS download, WebSocket over TLS, rejecting a certificate for
another name, and a blocked read that is cancelled while other requests are
answered. `tests/discord.rs` runs the Discord provider with arikawa's HTTP
client and gateway over TLS.

Limits:

- TCP only; no UDP, listening sockets or `LocalAddr`.
- Lookups use `wasi:sockets/ip-name-lookup`, so they don't go through
  `Orchestrator::set_resolver`, and their concurrency is Wasmtime's.
- An idle provider wakes up to every 10 ms while something waits on a
  socket.
- Components are large (the Discord one is 16 MB) and take seconds to
  compile when spawned.

## Logins in a browser

`Context.BrowserLogin(ctx, recipe)` asks the orchestrator to sign the
account in with its browser login service, and returns the session token
once the user is done; store it with `SecretSet`. The orchestrator shows the
user the link meanwhile. Ending `ctx` closes the browser.

## Tests

```sh
cd sdk/go && go vet ./... && GOOS=wasip1 GOARCH=wasm go vet ./... && go test ./...
```

`fixtures_test.go` checks the Go types encode and decode exactly as the Rust
ones, against the golden files in `testdata/fixtures` that
`crates/qmsg-types/tests/go_fixtures.rs` checks the Rust side against. After
changing the types, run that test with `QMSG_UPDATE_FIXTURES=1` and update
the Go mirror until both pass.
