//go:build wasip1

package wasi

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strconv"
	"sync"
	"time"
	"unsafe"
)

var (
	netOnce sync.Once
	network uint32
)

func instance() uint32 {
	netOnce.Do(func() { network = instanceNetwork() })
	return network
}

// ErrorCode is a wasi:sockets error-code.
type ErrorCode uint8

const errWouldBlock ErrorCode = 8

// maxRead bounds one read, and so what it takes of the arena.
const maxRead = 64 << 10

// MaxConns bounds the connections open at once, so a leak or a flood of
// dials fails here instead of piling up sockets in the host.
const MaxConns = 256

var conns = make(chan struct{}, MaxConns)

// ErrTooManyConns is returned by Dial while MaxConns connections are open.
var ErrTooManyConns = errors.New("too many open connections")

func (e ErrorCode) Error() string { return fmt.Sprintf("wasi:sockets error-code %d", uint8(e)) }

// waitCtx parks until p is ready or ctx ends. It returns only once the
// poller has let go of p, so the caller can drop it.
func waitCtx(ctx context.Context, p uint32) error {
	stop := make(chan time.Time)
	done := make(chan bool, 1)
	go func() { done <- Wait(p, stop) }()
	select {
	case <-done:
		return nil
	case <-ctx.Done():
		close(stop)
		if <-done {
			return nil
		}
		return ctx.Err()
	}
}

// Resolve looks name up with wasi:sockets/ip-name-lookup.
func Resolve(ctx context.Context, name string) ([]net.IP, error) {
	if ip := net.ParseIP(name); ip != nil {
		return []net.IP{ip}, nil
	}
	if name == "" {
		return nil, errors.New("empty host name")
	}
	var ret [8]uint32
	b := []byte(name)
	resolveAddresses(instance(), unsafe.Pointer(&b[0]), uint32(len(b)), unsafe.Pointer(&ret))
	r := (*[32]byte)(unsafe.Pointer(&ret))
	if r[0] != 0 {
		return nil, ErrorCode(r[4])
	}
	stream := ret[1]
	defer dropResolveStream(stream)
	var ips []net.IP
	for {
		var out [12]uint16
		o := (*[24]byte)(unsafe.Pointer(&out))
		resolveNextAddress(stream, unsafe.Pointer(&out))
		if o[0] != 0 {
			if ErrorCode(o[2]) == errWouldBlock {
				p := resolveSubscribe(stream)
				err := waitCtx(ctx, p)
				dropPollable(p)
				if err != nil {
					return nil, err
				}
				continue
			}
			return nil, ErrorCode(o[2])
		}
		if o[2] == 0 {
			return ips, nil
		}
		if o[4] == 0 {
			ips = append(ips, net.IPv4(o[6], o[7], o[8], o[9]))
		} else {
			ip := make(net.IP, 16)
			for i := 0; i < 8; i++ {
				v := out[3+i]
				ip[2*i], ip[2*i+1] = byte(v>>8), byte(v)
			}
			ips = append(ips, ip)
		}
	}
}

// Dial opens a TCP connection; it has the signature of net.Dialer.DialContext.
func Dial(ctx context.Context, _network, addr string) (net.Conn, error) {
	host, portStr, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, err
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return nil, err
	}
	ips, err := Resolve(ctx, host)
	if err != nil {
		return nil, &net.OpError{Op: "dial", Net: "tcp", Err: fmt.Errorf("lookup %s: %w", host, err)}
	}
	var last error = errors.New("no addresses")
	for _, ip := range ips {
		c, err := dialIP(ctx, ip, port)
		if err == nil {
			return c, nil
		}
		last = err
	}
	return nil, &net.OpError{Op: "dial", Net: "tcp", Err: last}
}

func dialIP(ctx context.Context, ip net.IP, port int) (c net.Conn, err error) {
	select {
	case conns <- struct{}{}:
	default:
		return nil, ErrTooManyConns
	}
	defer func() {
		if err != nil {
			<-conns
		}
	}()
	var ret [4]uint32
	r := (*[16]byte)(unsafe.Pointer(&ret))
	var a [11]uint32
	family, disc := uint32(0), uint32(0)
	if v4 := ip.To4(); v4 != nil {
		a[0] = uint32(port)
		a[1], a[2], a[3], a[4] = uint32(v4[0]), uint32(v4[1]), uint32(v4[2]), uint32(v4[3])
	} else {
		family, disc = 1, 1
		a[0] = uint32(port)
		for i := 0; i < 8; i++ {
			a[2+i] = uint32(ip[2*i])<<8 | uint32(ip[2*i+1])
		}
	}
	createTCPSocket(family, unsafe.Pointer(&ret))
	if r[0] != 0 {
		return nil, ErrorCode(r[4])
	}
	sock := ret[1]
	startConnect(sock, instance(), disc, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], a[10], unsafe.Pointer(&ret))
	if r[0] != 0 {
		dropTCPSocket(sock)
		return nil, ErrorCode(r[4])
	}
	for {
		finishConnect(sock, unsafe.Pointer(&ret))
		if r[0] == 0 {
			c := &Conn{sock: sock, in: ret[1], out: ret[2], remote: &net.TCPAddr{IP: ip, Port: port}}
			c.closed = make(chan struct{})
			return c, nil
		}
		if ErrorCode(r[4]) != errWouldBlock {
			dropTCPSocket(sock)
			return nil, ErrorCode(r[4])
		}
		p := tcpSubscribe(sock)
		err := waitCtx(ctx, p)
		dropPollable(p)
		if err != nil {
			dropTCPSocket(sock)
			return nil, err
		}
	}
}

// Conn is a TCP connection over wasi:io streams.
type Conn struct {
	sock, in, out      uint32
	remote             net.Addr
	rmu, wmu           sync.Mutex
	dmu                sync.Mutex
	rdl, wdl           time.Time
	rchanged, wchanged chan struct{}
	closeOnce          sync.Once
	closed             chan struct{}
	pending            []byte
}

func (c *Conn) deadline(write bool) (time.Time, <-chan struct{}) {
	c.dmu.Lock()
	defer c.dmu.Unlock()
	d := c.rdl
	if c.rchanged == nil {
		c.rchanged = make(chan struct{})
	}
	if c.wchanged == nil {
		c.wchanged = make(chan struct{})
	}
	changed := c.rchanged
	if write {
		d = c.wdl
		changed = c.wchanged
	}
	return d, changed
}

// wait parks until p is ready, the deadline passes, or the conn closes.
func (c *Conn) wait(p uint32, write bool) error {
	done := make(chan bool, 1)
	cancel := make(chan time.Time)
	go func() { done <- Wait(p, cancel) }()
	// Keep the pollable registered across deadline updates. Only cancel
	// its waiter once leaving, before the caller drops the pollable.
	for {
		deadline, changed := c.deadline(write)
		var timer *time.Timer
		var expired <-chan time.Time
		if !deadline.IsZero() {
			timer = time.NewTimer(time.Until(deadline))
			expired = timer.C
		}
		select {
		case <-changed:
			if timer != nil {
				timer.Stop()
			}
			continue
		case ok := <-done:
			if timer != nil {
				timer.Stop()
			}
			if ok {
				return nil
			}
			return os.ErrDeadlineExceeded
		case <-expired:
			// A deadline update can race the old timer. Check the
			// current value before treating its expiry as final.
			if timer != nil {
				timer.Stop()
			}
			current, _ := c.deadline(write)
			if current.IsZero() || current.After(time.Now()) {
				continue
			}
			close(cancel)
			<-done
			return os.ErrDeadlineExceeded
		case <-c.closed:
			if timer != nil {
				timer.Stop()
			}
			close(cancel)
			<-done
			return net.ErrClosed
		}
	}
}

func (c *Conn) isClosed() bool {
	select {
	case <-c.closed:
		return true
	default:
		return false
	}
}

func (c *Conn) Read(b []byte) (int, error) {
	c.rmu.Lock()
	defer c.rmu.Unlock()
	if len(b) == 0 {
		return 0, nil
	}
	for {
		if c.isClosed() {
			return 0, net.ErrClosed
		}
		if deadline, _ := c.deadline(false); !deadline.IsZero() && !deadline.After(time.Now()) {
			return 0, &net.OpError{Op: "read", Net: "tcp", Err: os.ErrDeadlineExceeded}
		}
		var ret [4]uint32
		r := (*[16]byte)(unsafe.Pointer(&ret))
		n := 0
		Lists(func() { streamRead(c.in, uint64(min(len(b), maxRead)), unsafe.Pointer(&ret)) }, func() {
			if r[0] == 0 && ret[2] > 0 {
				n = copy(b, arenaSlice(ret[1], ret[2]))
			}
		})
		if r[0] != 0 {
			if r[4] == 0 {
				dropError(ret[2])
				return 0, &net.OpError{Op: "read", Net: "tcp", Err: errors.New("stream failed")}
			}
			return 0, io.EOF
		}
		if n > 0 {
			return n, nil
		}
		p := inputSubscribe(c.in)
		err := c.wait(p, false)
		dropPollable(p)
		if err != nil {
			return 0, &net.OpError{Op: "read", Net: "tcp", Err: err}
		}
	}
}

func (c *Conn) Write(b []byte) (int, error) {
	c.wmu.Lock()
	defer c.wmu.Unlock()
	n := 0
	for n < len(b) {
		if c.isClosed() {
			return n, net.ErrClosed
		}
		if deadline, _ := c.deadline(true); !deadline.IsZero() && !deadline.After(time.Now()) {
			return n, &net.OpError{Op: "write", Net: "tcp", Err: os.ErrDeadlineExceeded}
		}
		var ret [4]uint64
		r := (*[32]byte)(unsafe.Pointer(&ret))
		checkWrite(c.out, unsafe.Pointer(&ret))
		if r[0] != 0 {
			dropStreamError(r[8:])
			return n, &net.OpError{Op: "write", Net: "tcp", Err: io.ErrClosedPipe}
		}
		allowed := ret[1]
		if allowed == 0 {
			p := outputSubscribe(c.out)
			err := c.wait(p, true)
			dropPollable(p)
			if err != nil {
				return n, &net.OpError{Op: "write", Net: "tcp", Err: err}
			}
			continue
		}
		chunk := b[n:]
		if uint64(len(chunk)) > allowed {
			chunk = chunk[:allowed]
		}
		var wret [3]uint32
		w := (*[12]byte)(unsafe.Pointer(&wret))
		streamWrite(c.out, unsafe.Pointer(&chunk[0]), uint32(len(chunk)), unsafe.Pointer(&wret))
		if w[0] != 0 {
			dropStreamError(w[4:])
			return n, &net.OpError{Op: "write", Net: "tcp", Err: io.ErrClosedPipe}
		}
		n += len(chunk)
	}
	return n, nil
}

func (c *Conn) Close() error {
	c.closeOnce.Do(func() {
		close(c.closed)
		// Readers and writers leave once they see closed; take both locks
		// so no import uses the handles while they're dropped.
		c.rmu.Lock()
		c.wmu.Lock()
		c.flush()
		var ret [2]uint32
		tcpShutdown(c.sock, 2, unsafe.Pointer(&ret))
		dropInputStream(c.in)
		dropOutputStream(c.out)
		dropTCPSocket(c.sock)
		<-conns
		c.wmu.Unlock()
		c.rmu.Unlock()
	})
	return nil
}

// flush waits up to a second for written data to leave, since dropping the
// stream discards what it still buffers.
func (c *Conn) flush() {
	var ret [3]uint32
	streamFlush(c.out, unsafe.Pointer(&ret))
	if f := (*[12]byte)(unsafe.Pointer(&ret)); f[0] != 0 {
		dropStreamError(f[4:])
		return
	}
	deadline := time.After(time.Second)
	for {
		var cw [4]uint64
		checkWrite(c.out, unsafe.Pointer(&cw))
		if r := (*[32]byte)(unsafe.Pointer(&cw)); r[0] != 0 {
			dropStreamError(r[8:])
			return
		}
		if cw[1] > 0 {
			return
		}
		p := outputSubscribe(c.out)
		ok := Wait(p, deadline)
		dropPollable(p)
		if !ok {
			return
		}
	}
}

func (c *Conn) LocalAddr() net.Addr  { return &net.TCPAddr{} }
func (c *Conn) RemoteAddr() net.Addr { return c.remote }
func (c *Conn) SetDeadline(t time.Time) error {
	c.dmu.Lock()
	c.rdl, c.wdl = t, t
	if c.rchanged != nil {
		close(c.rchanged)
	}
	if c.wchanged != nil {
		close(c.wchanged)
	}
	c.rchanged, c.wchanged = make(chan struct{}), make(chan struct{})
	c.dmu.Unlock()
	return nil
}
func (c *Conn) SetReadDeadline(t time.Time) error {
	c.dmu.Lock()
	c.rdl = t
	if c.rchanged != nil {
		close(c.rchanged)
	}
	c.rchanged = make(chan struct{})
	c.dmu.Unlock()
	return nil
}
func (c *Conn) SetWriteDeadline(t time.Time) error {
	c.dmu.Lock()
	c.wdl = t
	if c.wchanged != nil {
		close(c.wchanged)
	}
	c.wchanged = make(chan struct{})
	c.dmu.Unlock()
	return nil
}

// dropStreamError frees the error in a stream-error, laid out as a tag
// byte, then at offset 4 the error's handle when the tag is 0
// (last-operation-failed); 1 is closed.
func dropStreamError(e []byte) {
	if e[0] == 0 {
		dropError(uint32(e[4]) | uint32(e[5])<<8 | uint32(e[6])<<16 | uint32(e[7])<<24)
	}
}
