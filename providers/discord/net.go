package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/gorilla/websocket"
	qmsg "github.com/voltaero/qmsg/sdk/go"

	// Wasm has no system certificate store, so carry Mozilla's roots.
	_ "golang.org/x/crypto/x509roots/fallback"
)

// network is how the provider reaches Discord: through wasi:sockets, with
// the standard roots. Two settings exist only for tests against a local
// mock: `test_dial` sends hosts elsewhere, as `host=ip:port,...`, and
// `test_ca_pem` replaces the trusted roots. TLS still checks the real
// host name.
type network struct {
	redirect map[string]string
	tls      *tls.Config
}

func newNetwork(settings map[string]string) (*network, error) {
	n := &network{redirect: map[string]string{}, tls: &tls.Config{MinVersion: tls.VersionTLS12}}
	if pem := settings["test_ca_pem"]; pem != "" {
		pool := x509.NewCertPool()
		if !pool.AppendCertsFromPEM([]byte(pem)) {
			return nil, errors.New("test_ca_pem holds no certificate")
		}
		n.tls.RootCAs = pool
	}
	if spec := settings["test_dial"]; spec != "" {
		for _, pair := range strings.Split(spec, ",") {
			host, addr, ok := strings.Cut(strings.TrimSpace(pair), "=")
			if !ok {
				return nil, errors.New("test_dial must be host=ip:port pairs")
			}
			n.redirect[host] = addr
		}
	}
	return n, nil
}

func (n *network) dial(ctx context.Context, network, addr string) (net.Conn, error) {
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, err
	}
	if to, ok := n.redirect[host]; ok {
		addr = to
	}
	return qmsg.Dial(ctx, network, addr)
}

func (n *network) transport() *http.Transport {
	return &http.Transport{
		DialContext:           n.dial,
		TLSClientConfig:       n.tls.Clone(),
		TLSHandshakeTimeout:   30 * time.Second,
		ResponseHeaderTimeout: 60 * time.Second,
		IdleConnTimeout:       90 * time.Second,
		MaxIdleConnsPerHost:   4,
	}
}

func (n *network) wsDialer() websocket.Dialer {
	return websocket.Dialer{
		NetDialContext:    n.dial,
		TLSClientConfig:   n.tls.Clone(),
		HandshakeTimeout:  30 * time.Second,
		ReadBufferSize:    4096,
		WriteBufferSize:   4096,
		EnableCompression: true,
	}
}
