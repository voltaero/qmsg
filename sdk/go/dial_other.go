//go:build !wasip1

package qmsg

import (
	"context"
	"net"
)

// Dial opens a TCP connection, with the signature of net.Dialer.DialContext.
// Under Wasm it goes through wasi:sockets; pass it to HTTP and WebSocket
// clients so they work in both.
var Dial = (&net.Dialer{}).DialContext

// Resolve looks a host name up; under Wasm through wasi:sockets.
func Resolve(ctx context.Context, name string) ([]net.IP, error) {
	return net.DefaultResolver.LookupIP(ctx, "ip", name)
}
