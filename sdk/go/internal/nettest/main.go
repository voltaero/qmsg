// Command nettest is a provider that only exercises the SDK's networking
// from inside Wasm, for the orchestrator's tests. Its `probe` lookup runs
// one check and returns what happened as text. Settings: `tls`, the address
// of a TLS server with a certificate for discord.com and gateway.discord.gg,
// and `ca`, the PEM of its CA.
package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/gorilla/websocket"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

func init() { qmsg.Register(run) }

func main() {}

var probe = qmsg.Function{
	ID: "probe", Label: "Probe",
	Kind: qmsg.KindLookup{Result: qmsg.TypeText{}},
	Inputs: []qmsg.FunctionInput{
		{ID: "kind", Label: "Kind", ValueType: qmsg.TypeText{}, Required: true, Hint: qmsg.HintPlain{}},
		{ID: "target", Label: "Target", ValueType: qmsg.TypeText{}, Required: true, Hint: qmsg.HintPlain{}},
	},
}

func run(cx *qmsg.Context) error {
	cx.Status(qmsg.StatusReady{})
	for {
		c, reqCtx, err := cx.NextCommand(context.Background())
		if err != nil {
			return nil
		}
		switch c := c.(type) {
		case qmsg.CmdShutdown:
			return nil
		case qmsg.CmdFunctions:
			cx.Reply(c.Request, qmsg.ReplyFunctions{Functions: []qmsg.Function{probe}}, nil)
		case qmsg.CmdCall:
			// Each on its own goroutine, so a hung one blocks nothing.
			go func() {
				kind, _ := c.Call.Arguments["kind"].(qmsg.Text)
				target, _ := c.Call.Arguments["target"].(qmsg.Text)
				start := time.Now()
				out, err := check(reqCtx, cx, string(kind), string(target))
				if err != nil {
					out = "error: " + err.Error()
				}
				out += fmt.Sprintf(" (%dms)", time.Since(start).Milliseconds())
				cx.Reply(c.Request, qmsg.ReplyValue{Value: qmsg.Text(out)}, nil)
			}()
		default:
			if r, ok := qmsg.RequestOf(c); ok {
				cx.Reply(r, nil, qmsg.ErrUnsupported{})
			}
		}
	}
}

func tlsConfig(cx *qmsg.Context) *tls.Config {
	pool := x509.NewCertPool()
	ca, _ := cx.Setting("ca")
	pool.AppendCertsFromPEM([]byte(ca))
	return &tls.Config{RootCAs: pool}
}

// toServer sends every connection to the `tls` setting's address.
func toServer(cx *qmsg.Context) func(context.Context, string, string) (net.Conn, error) {
	addr, _ := cx.Setting("tls")
	return func(ctx context.Context, network, _ string) (net.Conn, error) {
		return qmsg.Dial(ctx, network, addr)
	}
}

func check(ctx context.Context, cx *qmsg.Context, kind, target string) (string, error) {
	switch kind {
	case "resolve":
		ips, err := qmsg.Resolve(ctx, target)
		if err != nil {
			return "", err
		}
		var out []string
		for _, ip := range ips {
			out = append(out, ip.String())
		}
		return strings.Join(out, ","), nil
	case "dial":
		ctx, cancel := context.WithTimeout(ctx, 500*time.Millisecond)
		defer cancel()
		c, err := qmsg.Dial(ctx, "tcp", target)
		if err != nil {
			return "", err
		}
		c.Close()
		return "ok", nil
	case "dial-until-cancelled":
		c, err := qmsg.Dial(ctx, "tcp", target)
		if err == nil {
			c.Close()
		}
		return "", err
	case "read-timeout":
		c, err := qmsg.Dial(ctx, "tcp", target)
		if err != nil {
			return "", err
		}
		defer c.Close()
		c.SetReadDeadline(time.Now().Add(300 * time.Millisecond))
		_, err = c.Read(make([]byte, 1))
		return "", err
	case "deadline-update", "deadline-extend", "deadline-clear":
		c, err := qmsg.Dial(ctx, "tcp", target)
		if err != nil {
			return "", err
		}
		defer c.Close()
		if kind != "deadline-update" {
			c.SetReadDeadline(time.Now().Add(150 * time.Millisecond))
		}
		done := make(chan error, 1)
		go func() { _, err := c.Read(make([]byte, 1)); done <- err }()
		time.Sleep(50 * time.Millisecond)
		if kind == "deadline-clear" {
			c.SetReadDeadline(time.Time{})
		} else {
			c.SetReadDeadline(time.Now().Add(350 * time.Millisecond))
		}
		if kind != "deadline-update" {
			select {
			case err := <-done:
				return "", fmt.Errorf("old deadline fired: %v", err)
			case <-time.After(200 * time.Millisecond):
			}
		}
		if kind == "deadline-clear" {
			c.SetDeadline(time.Now().Add(50 * time.Millisecond))
		}
		select {
		case err := <-done:
			if !errors.Is(err, os.ErrDeadlineExceeded) {
				return "", fmt.Errorf("expected deadline: %v", err)
			}
			return "ok", nil
		case <-time.After(time.Second):
			return "", fmt.Errorf("pending read did not receive its new deadline")
		}
	case "hang":
		// Reads until the request is cancelled, which closes the socket.
		c, err := qmsg.Dial(ctx, "tcp", target)
		if err != nil {
			return "", err
		}
		go func() { <-ctx.Done(); c.Close() }()
		_, err = c.Read(make([]byte, 1))
		return "", err
	case "churn":
		// Sockets are freed: far more than the host would let leak.
		for i := 0; i < 300; i++ {
			c, err := qmsg.Dial(ctx, "tcp", target)
			if err != nil {
				return "", fmt.Errorf("connection %d: %w", i, err)
			}
			if _, err := c.Write([]byte("x")); err != nil {
				return "", err
			}
			c.Close()
		}
		return "ok", nil
	case "conns":
		// Connections beyond the SDK's limit fail until others close.
		var open []net.Conn
		defer func() {
			for _, c := range open {
				c.Close()
			}
		}()
		for {
			c, err := qmsg.Dial(ctx, "tcp", target)
			if err != nil {
				return fmt.Sprintf("%d open, then %v", len(open), err), nil
			}
			open = append(open, c)
		}
	case "https":
		client := &http.Client{Transport: &http.Transport{DialContext: toServer(cx), TLSClientConfig: tlsConfig(cx)}}
		req, _ := http.NewRequestWithContext(ctx, "GET", target, nil)
		resp, err := client.Do(req)
		if err != nil {
			return "", err
		}
		defer resp.Body.Close()
		n, err := io.Copy(io.Discard, resp.Body)
		if err != nil {
			return "", err
		}
		return fmt.Sprintf("%d %d", resp.StatusCode, n), nil
	case "wss":
		d := websocket.Dialer{NetDialContext: toServer(cx), TLSClientConfig: tlsConfig(cx)}
		c, _, err := d.DialContext(ctx, target, nil)
		if err != nil {
			return "", err
		}
		defer c.Close()
		var hello, ack struct{ Op int }
		if err := c.ReadJSON(&hello); err != nil {
			return "", err
		}
		if err := c.WriteJSON(map[string]any{"op": 1, "d": nil}); err != nil {
			return "", err
		}
		if err := c.ReadJSON(&ack); err != nil {
			return "", err
		}
		return fmt.Sprintf("op %d, op %d", hello.Op, ack.Op), nil
	}
	return "", fmt.Errorf("unknown probe %q", kind)
}
