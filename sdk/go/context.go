// Package qmsg is the Go SDK for qmsg providers.
//
// A provider is a wasip1 Go module made into a `wasm32-wasip2`-compatible
// component by cmd/componentize. It registers its entry point with Register
// from an init function; the orchestrator calls it through the `run` export
// with a Context connected to the orchestrator's WebSocket.
//
// Unlike Rust providers, Go providers may block: each goroutine waiting on a
// socket parks while others run. Answer commands from their own goroutines,
// so Shutdown and cancellations are seen while platform work is slow.
package qmsg

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

// RunFunc runs a provider until it is done. Returning ends the provider.
type RunFunc func(cx *Context) error

var registered RunFunc

// Register sets the provider's entry point. Call it from an init function.
func Register(run RunFunc) { registered = run }

// Context is a provider's connection to the orchestrator. Its methods are
// safe for concurrent use.
type Context struct {
	config ProviderConfig
	conn   *websocket.Conn

	writeMu sync.Mutex

	mu sync.Mutex
	// Commands not yet taken; the orchestrator bounds how many wait.
	pending []Command
	notify  chan struct{}
	// Cancels for commands taken and not yet answered, by request.
	running map[uint64]context.CancelFunc
	// Ids of SendMessage functions, to report skipped calls as NotSent.
	sendFunctions map[string]bool
	closed        error
	done          chan struct{}

	// One secret operation runs at a time; its answer comes here.
	secretMu sync.Mutex
	answers  chan HostMessage

	// Browser logins waiting for their answer, by request.
	logins    map[uint64]chan HmBrowserLogin
	nextLogin uint64
	blobReads map[blobReadKey]chan HmBlob
}

// Connect connects to the orchestrator named in config. The `run` export
// does this; tests use it to run a provider against their own host.
func Connect(ctx context.Context, config ProviderConfig) (*Context, error) {
	d := websocket.Dialer{
		NetDialContext:   Dial,
		HandshakeTimeout: 30 * time.Second,
	}
	conn, _, err := d.DialContext(ctx, config.HostURL, nil)
	if err != nil {
		return nil, fmt.Errorf("connecting to orchestrator: %w", err)
	}
	conn.SetReadLimit(MaxFrame)
	cx := &Context{
		config:        config,
		conn:          conn,
		notify:        make(chan struct{}, 1),
		running:       map[uint64]context.CancelFunc{},
		sendFunctions: map[string]bool{},
		done:          make(chan struct{}),
		answers:       make(chan HostMessage, 1),
		logins:        map[uint64]chan HmBrowserLogin{},
	}
	go cx.readLoop()
	return cx, nil
}

// RunWith connects and runs run, closing the connection after.
func RunWith(config ProviderConfig, run RunFunc) error {
	cx, err := Connect(context.Background(), config)
	if err != nil {
		return err
	}
	defer cx.Close()
	return run(cx)
}

func (cx *Context) Config() ProviderConfig { return cx.config }

// Setting returns a setting from the provider's config.
func (cx *Context) Setting(key string) (string, bool) {
	v, ok := cx.config.Settings[key]
	return v, ok
}

// Done is closed once the connection to the orchestrator is lost.
func (cx *Context) Done() <-chan struct{} { return cx.done }

// Close closes the connection to the orchestrator.
func (cx *Context) Close() error {
	cx.writeMu.Lock()
	cx.conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.CloseNormalClosure, ""), time.Now().Add(time.Second))
	cx.writeMu.Unlock()
	return cx.conn.Close()
}

func (cx *Context) send(m ProviderMessage) error {
	b := EncodeProviderMessage(m)
	if len(b) > MaxFrame {
		return fmt.Errorf("message is %d bytes, over the %d byte limit", len(b), MaxFrame)
	}
	cx.writeMu.Lock()
	defer cx.writeMu.Unlock()
	return cx.conn.WriteMessage(websocket.BinaryMessage, b)
}

// Log sends a log line to the orchestrator. It never fails the provider.
func (cx *Context) Log(level LogLevel, format string, a ...any) {
	_ = cx.send(PmLog{level, fmt.Sprintf(format, a...)})
}

// Status reports where the provider is in connecting to its platform.
// Report Syncing, the directory, then Ready.
func (cx *Context) Status(s ProviderStatus) error { return cx.send(PmStatus{s}) }

// Directory reports a change to the organizations, users and channels the
// provider is part of, before any message in them.
func (cx *Context) Directory(u DirectoryUpdate) error { return cx.send(PmDirectory{u}) }

// Emit reports something that happened to a message.
func (cx *Context) Emit(e MessageEvent) error { return cx.send(PmMessage{e}) }

// Reply answers a command. err, if not nil, is sent instead of r. Emit the
// events that must come first before replying.
func (cx *Context) Reply(request uint64, r Reply, err CommandError) error {
	cx.mu.Lock()
	if cancel, ok := cx.running[request]; ok {
		cancel()
		delete(cx.running, request)
	}
	if fs, ok := r.(ReplyFunctions); ok && err == nil {
		for _, f := range fs.Functions {
			if a, ok := f.Kind.(KindAction); ok && a.Action.Tag == ActionSendMessage {
				cx.sendFunctions[f.ID] = true
			}
		}
	}
	cx.mu.Unlock()
	return cx.send(PmReply{Request: request, Reply: r, Err: err})
}

// NextCommand waits for the next command, or until ctx ends. The returned
// context ends when the orchestrator cancels the request or it is
// answered; check it before a step that can't be undone. Commands without a
// request, such as Shutdown, get a context that never ends.
func (cx *Context) NextCommand(ctx context.Context) (Command, context.Context, error) {
	for {
		cx.mu.Lock()
		if len(cx.pending) > 0 {
			c := cx.pending[0]
			cx.pending = cx.pending[1:]
			reqCtx := context.Background()
			if r, ok := RequestOf(c); ok {
				var cancel context.CancelFunc
				reqCtx, cancel = context.WithCancel(context.Background())
				cx.running[r] = cancel
			}
			cx.mu.Unlock()
			return c, reqCtx, nil
		}
		closed := cx.closed
		cx.mu.Unlock()
		if closed != nil {
			return nil, nil, closed
		}
		select {
		case <-cx.notify:
		case <-cx.done:
		case <-ctx.Done():
			return nil, nil, ctx.Err()
		}
	}
}

func (cx *Context) readLoop() {
	err := cx.read()
	cx.mu.Lock()
	cx.closed = err
	for _, cancel := range cx.running {
		cancel()
	}
	cx.mu.Unlock()
	close(cx.done)
}

func (cx *Context) read() error {
	for {
		kind, b, err := cx.conn.ReadMessage()
		if err != nil {
			if websocket.IsCloseError(err, websocket.CloseNormalClosure) {
				return errors.New("orchestrator closed the connection")
			}
			return err
		}
		if kind != websocket.BinaryMessage {
			continue
		}
		m, err := DecodeHostMessage(b)
		if err != nil {
			return fmt.Errorf("bad message from orchestrator: %w", err)
		}
		switch m := m.(type) {
		case HmCommand:
			if err := cx.accept(m.Command); err != nil {
				return err
			}
		case HmBrowserLogin:
			cx.mu.Lock()
			if ch, ok := cx.logins[m.Request]; ok {
				delete(cx.logins, m.Request)
				ch <- m
			}
			cx.mu.Unlock()
		case HmBlob:
			cx.mu.Lock()
			key := blobReadKey{m.Blob, m.Offset}
			if ch, ok := cx.blobReads[key]; ok {
				delete(cx.blobReads, key)
				ch <- m
			}
			cx.mu.Unlock()
		case HmSecret, HmSecretStored:
			select {
			case cx.answers <- m:
			default:
				return fmt.Errorf("unexpected %T from orchestrator", m)
			}
		default:
			// Blob reads and lookups this SDK never starts.
			return fmt.Errorf("unexpected %T from orchestrator", m)
		}
	}
}

func (cx *Context) accept(c Command) error {
	cancel, ok := c.(CmdCancel)
	if !ok {
		cx.mu.Lock()
		cx.pending = append(cx.pending, c)
		cx.mu.Unlock()
		select {
		case cx.notify <- struct{}{}:
		default:
		}
		return nil
	}
	cx.mu.Lock()
	var queued Command
	for i, p := range cx.pending {
		if r, ok := RequestOf(p); ok && r == cancel.Request {
			queued = p
			cx.pending = append(cx.pending[:i:i], cx.pending[i+1:]...)
			break
		}
	}
	if queued == nil {
		if stop, ok := cx.running[cancel.Request]; ok {
			stop()
		}
		cx.mu.Unlock()
		return nil
	}
	channel, nonce, unsent := cx.unsent(queued)
	cx.mu.Unlock()
	// It never reached the platform.
	if unsent {
		if err := cx.Emit(NotSent{channel, nonce, ErrCancelled{}}); err != nil {
			return err
		}
	}
	return cx.Reply(cancel.Request, nil, ErrCancelled{})
}

// unsent returns where a send with a nonce was going. Holds cx.mu.
func (cx *Context) unsent(c Command) (ChannelRef, string, bool) {
	switch c := c.(type) {
	case CmdSend:
		if c.Nonce != nil {
			return c.Channel, *c.Nonce, true
		}
	case CmdCall:
		if cx.sendFunctions[c.Call.Function] {
			ch, ok := c.Call.Context.(InChannel)
			nonce, ok2 := c.Call.Arguments[InputNonce].(Text)
			if ok && ok2 {
				return ch.Channel, string(nonce), true
			}
		}
	}
	return ChannelRef{}, "", false
}

func (cx *Context) secret(m ProviderMessage, key string) (HostMessage, error) {
	if len(key) > MaxID {
		return nil, fmt.Errorf("secret key is over %d bytes", MaxID)
	}
	cx.secretMu.Lock()
	defer cx.secretMu.Unlock()
	if err := cx.send(m); err != nil {
		return nil, err
	}
	select {
	case a := <-cx.answers:
		return a, nil
	case <-cx.done:
		return nil, errors.New("orchestrator connection lost")
	}
}

// SecretGet reads a secret stored under the provider's name. It returns nil
// and no error when the secret was never set.
func (cx *Context) SecretGet(key string) ([]byte, error) {
	a, err := cx.secret(PmSecretGet{key}, key)
	if err != nil {
		return nil, err
	}
	s, ok := a.(HmSecret)
	if !ok || s.Key != key {
		return nil, fmt.Errorf("unexpected answer %T to a secret read", a)
	}
	if s.Err != nil {
		return nil, errors.New(*s.Err)
	}
	if !s.Found {
		return nil, nil
	}
	return s.Value, nil
}

func (cx *Context) stored(m ProviderMessage, key string) error {
	a, err := cx.secret(m, key)
	if err != nil {
		return err
	}
	s, ok := a.(HmSecretStored)
	if !ok || s.Key != key {
		return fmt.Errorf("unexpected answer %T to a secret write", a)
	}
	if s.Err != nil {
		return errors.New(*s.Err)
	}
	return nil
}

// SecretSet stores a secret under the provider's name.
func (cx *Context) SecretSet(key string, value []byte) error {
	return cx.stored(PmSecretSet{key, value}, key)
}

// SecretDelete removes a secret.
func (cx *Context) SecretDelete(key string) error {
	return cx.stored(PmSecretDelete{key}, key)
}

// BrowserLogin signs in through the orchestrator's browser login service,
// with its recipe for provider, such as "discord", and returns the session
// token it produced. Meanwhile the orchestrator reports LoginRequired with a
// link to the browser. Store the token with SecretSet: the orchestrator
// keeps no copy. Ending ctx closes the browser. One runs at a time.
func (cx *Context) BrowserLogin(ctx context.Context, provider string) (string, error) {
	answer := make(chan HmBrowserLogin, 1)
	cx.mu.Lock()
	cx.nextLogin++
	request := cx.nextLogin
	cx.logins[request] = answer
	cx.mu.Unlock()
	forget := func() {
		cx.mu.Lock()
		delete(cx.logins, request)
		cx.mu.Unlock()
	}
	if err := cx.send(PmBrowserLogin{request, provider}); err != nil {
		forget()
		return "", err
	}
	select {
	case a := <-answer:
		if a.Err != nil {
			return "", errors.New(*a.Err)
		}
		return a.Token, nil
	case <-ctx.Done():
		forget()
		_ = cx.send(PmCancelBrowserLogin{request})
		return "", ctx.Err()
	case <-cx.done:
		forget()
		return "", errors.New("orchestrator connection lost")
	}
}
