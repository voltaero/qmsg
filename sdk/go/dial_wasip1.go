//go:build wasip1

package qmsg

import "github.com/voltaero/qmsg/sdk/go/wasi"

// Dial opens a TCP connection through wasi:sockets, with the signature of
// net.Dialer.DialContext. The Go standard library can't open sockets on
// wasip1, so pass this to HTTP and WebSocket clients.
var Dial = wasi.Dial

// Resolve looks a host name up through wasi:sockets.
var Resolve = wasi.Resolve
