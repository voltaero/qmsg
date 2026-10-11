// Command discord is the qmsg provider for Discord user accounts, built on
// ningen and arikawa. It runs only as a Wasm component; see README.md.
package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"sync"
	"sync/atomic"
	"time"

	"github.com/diamondburned/arikawa/v3/api"
	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/arikawa/v3/gateway"
	"github.com/diamondburned/arikawa/v3/session"
	"github.com/diamondburned/arikawa/v3/state"
	"github.com/diamondburned/arikawa/v3/state/store/defaultstore"
	"github.com/diamondburned/arikawa/v3/utils/handler"
	"github.com/diamondburned/arikawa/v3/utils/httputil"
	"github.com/diamondburned/arikawa/v3/utils/ws"
	"github.com/diamondburned/ningen/v3"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

func init() { qmsg.Register(run) }

func main() {}

// tokenKey is the secret holding the session token.
const tokenKey = "token"

// maxRunning bounds commands worked on at once; the rest wait in order.
const maxRunning = 16

type provider struct {
	cx  *qmsg.Context
	net *network

	mu sync.Mutex
	st *ningen.State // nil while logged out or connecting
	me discord.UserID
	// Whether the directory has been sent since the last connect, so
	// events can refer to it.
	synced bool
	events eventTracker
	// Users reported per organization ("" for none), to report new ones
	// before their messages.
	users map[string]map[discord.UserID]bool
	// Ends the current session, such as on logout.
	endSession context.CancelFunc
	// Signalled when a login stored a token.
	loggedIn chan struct{}
	// Set while a login runs; one at a time.
	loggingIn atomic.Bool
}

func run(cx *qmsg.Context) error {
	n, err := newNetwork(cx.Config().Settings)
	if err != nil {
		return err
	}
	// arikawa's REST clients use the default transport.
	http.DefaultTransport = n.transport()
	p := &provider{cx: cx, net: n, loggedIn: make(chan struct{}, 1)}

	ctx, stop := context.WithCancel(context.Background())
	defer stop()
	served := make(chan error, 1)
	go func() { served <- p.serve(ctx, stop) }()
	go p.sessions(ctx)

	err = <-served
	stop()
	p.mu.Lock()
	if p.st != nil {
		p.st.Close()
	}
	p.mu.Unlock()
	return err
}

// serve answers commands until Shutdown, or the orchestrator goes away.
func (p *provider) serve(ctx context.Context, stop context.CancelFunc) error {
	slots := make(chan struct{}, maxRunning)
	var running sync.WaitGroup
	defer running.Wait()
	for {
		c, reqCtx, err := p.cx.NextCommand(ctx)
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return err
		}
		if _, ok := c.(qmsg.CmdShutdown); ok {
			stop()
			return nil
		}
		select {
		case slots <- struct{}{}:
		case <-ctx.Done():
			return nil
		}
		running.Add(1)
		go func() {
			defer func() { <-slots; running.Done() }()
			p.handle(ctx, reqCtx, c)
		}()
	}
}

// sessions keeps a Discord session running while a token is stored, and
// reports a login as needed while not.
func (p *provider) sessions(ctx context.Context) {
	p.cx.Status(qmsg.StatusConnecting{})
	backoff := time.Second
	for ctx.Err() == nil {
		token, err := p.token()
		if err != nil {
			p.cx.Status(qmsg.StatusFailed{Reason: "reading the stored session: " + err.Error()})
			return
		}
		if token == "" {
			p.loginRequired("Sign in to Discord to connect this account.")
			select {
			case <-p.loggedIn:
				p.cx.Status(qmsg.StatusConnecting{})
				continue
			case <-ctx.Done():
				return
			}
		}
		sessionCtx, end := context.WithCancel(ctx)
		p.mu.Lock()
		p.endSession = end
		p.mu.Unlock()
		err = p.connect(sessionCtx, token)
		end()
		switch {
		case ctx.Err() != nil:
			return
		case errors.Is(err, errLoggedOut):
			continue
		case err != nil:
			p.cx.Log(qmsg.LogWarn, "discord: %v", err)
			p.cx.Status(qmsg.StatusDisconnected{Reason: err.Error()})
			select {
			case <-time.After(backoff):
			case <-ctx.Done():
				return
			}
			backoff = min(backoff*2, time.Minute)
		default:
			backoff = time.Second
		}
	}
}

var errLoggedOut = errors.New("logged out")

// token returns the stored session token, or "" when logged out.
func (p *provider) token() (string, error) {
	b, err := p.cx.SecretGet(tokenKey)
	return string(b), err
}

func (p *provider) loginRequired(msg string) {
	p.cx.Status(qmsg.StatusLoginRequired{Step: qmsg.LoginStep{Message: msg, Function: qmsg.Str(fnLogin)}})
}

// login signs in through the orchestrator's browser, stores the token, and
// has the session loop connect with it. Cancelling closes the browser.
func (p *provider) login(ctx context.Context) qmsg.CommandError {
	if !p.loggingIn.CompareAndSwap(false, true) {
		return qmsg.ErrFailed{Reason: "already signing in"}
	}
	defer p.loggingIn.Store(false)
	token, err := p.cx.BrowserLogin(ctx, "discord")
	if ctx.Err() != nil {
		p.loginRequired("Sign in to Discord to connect this account.")
		return qmsg.ErrCancelled{}
	}
	if err != nil {
		p.loginRequired("Signing in failed: " + err.Error())
		return qmsg.ErrFailed{Reason: err.Error()}
	}
	if err := p.cx.SecretSet(tokenKey, []byte(token)); err != nil {
		return qmsg.ErrFailed{Reason: "storing the session: " + err.Error()}
	}
	select {
	case p.loggedIn <- struct{}{}:
	default:
	}
	return nil
}

// logout forgets the session, here and in the store.
func (p *provider) logout(reason string) error {
	err := p.cx.SecretDelete(tokenKey)
	p.mu.Lock()
	st, end := p.st, p.endSession
	p.st, p.synced = nil, false
	p.mu.Unlock()
	if end != nil {
		end()
	}
	if st != nil {
		st.Close()
	}
	p.cx.Directory(qmsg.DirectoryReset{})
	p.loginRequired(reason)
	return err
}

// newState builds ningen's state the way ningen.New does, but with the
// gateway's WebSocket going through the provider's network.
func (p *provider) newState(ctx context.Context, token string) (*ningen.State, error) {
	id := gateway.DefaultIdentifier(token)
	id.Capabilities = gateway.LazyUserNotes | gateway.VersionedReadStates |
		gateway.VersionedUserGuildSetttings | gateway.DedupeUserObjects |
		gateway.PrioritizedReadyPayload | gateway.MultipleGuildExperimentPopulations |
		gateway.NonChannelReadStates
	url, err := id.QueryGateway(ctx)
	if err != nil {
		return nil, fmt.Errorf("finding the gateway: %w", err)
	}
	conn := ws.NewConnWithDialer(ws.NewCodec(gateway.OpUnmarshalers), p.net.wsDialer())
	opts := gateway.DefaultGatewayOpts
	wsgw := ws.NewGateway(ws.NewCustomWebsocket(conn, gateway.AddGatewayParams(url)), &opts)
	gw := gateway.FromWebsocketGateway(wsgw, gateway.State{Identifier: id})
	sess := session.NewWithGateway(gw, handler.New())
	return ningen.FromState(state.NewFromSession(sess, defaultstore.New())), nil
}

// connect runs one session until it ends.
func (p *provider) connect(ctx context.Context, token string) error {
	st, err := p.newState(ctx, token)
	if isUnauthorized(err) {
		return p.expired()
	}
	if err != nil {
		return err
	}
	lost := make(chan error, 1)
	p.handleEvents(st, lost)
	openCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
	err = st.Open(openCtx)
	cancel()
	if err != nil {
		st.Close()
		var closed *ws.CloseEvent
		if errors.As(err, &closed) && isLoggedOutCode(closed.Code) || isUnauthorized(err) {
			return p.expired()
		}
		return fmt.Errorf("connecting: %w", err)
	}
	p.mu.Lock()
	p.st = st
	p.mu.Unlock()
	select {
	case <-ctx.Done():
	case err = <-lost:
	}
	p.mu.Lock()
	if p.st == st {
		p.st, p.synced = nil, false
	}
	p.mu.Unlock()
	st.Close()
	return err
}

func (p *provider) expired() error {
	p.logout("Your Discord session ended. Sign in again.")
	return errLoggedOut
}

func isLoggedOutCode(code int) bool {
	for _, c := range gateway.DefaultGatewayOpts.FatalCloseCodes {
		if c == code {
			return true
		}
	}
	return false
}

func isUnauthorized(err error) bool {
	var h *httputil.HTTPError
	return errors.As(err, &h) && h.Status == http.StatusUnauthorized
}

// sync reports everything the session knows: a reset, the direct
// messages, each guild, then Ready.
func (p *provider) sync(st *ningen.State) {
	me, err := st.Me()
	if err != nil {
		p.cx.Status(qmsg.StatusFailed{Reason: "no account in the session"})
		return
	}
	p.cx.Status(qmsg.StatusSyncing{})
	p.mu.Lock()
	p.me = me.ID
	p.users = map[string]map[discord.UserID]bool{}
	p.mu.Unlock()
	p.cx.Directory(qmsg.DirectoryReset{})
	p.cx.Directory(qmsg.Me{ID: qmsg.Str(me.ID.String())})
	p.knowUser(nil, *me)
	if dms, err := st.Cabinet.PrivateChannels(); err == nil {
		for _, ch := range dms {
			for _, u := range ch.DMRecipients {
				p.knowUser(nil, u)
			}
			p.cx.Directory(qmsg.ChannelUpserted{Channel: channel(ch, me.ID, true)})
		}
	}
	if guilds, err := st.Cabinet.Guilds(); err == nil {
		for _, g := range guilds {
			p.cx.Directory(qmsg.OrganizationUpserted{Organization: p.organization(st, g)})
		}
	}
	p.mu.Lock()
	p.synced = true
	p.mu.Unlock()
	p.cx.Status(qmsg.StatusReady{})
}

func (p *provider) organization(st *ningen.State, g discord.Guild) qmsg.Organization {
	me := p.me
	o := qmsg.Organization{ID: g.ID.String(), Name: g.Name, Me: qmsg.Str(me.String())}
	known := map[discord.UserID]bool{}
	if members, err := st.Cabinet.Members(g.ID); err == nil {
		for _, m := range members {
			known[m.User.ID] = true
			o.Users = append(o.Users, qmsg.User{ID: m.User.ID.String(), Name: memberName(m)})
		}
	}
	if !known[me] {
		known[me] = true
		if u, err := st.Me(); err == nil {
			o.Users = append(o.Users, qmsg.User{ID: me.String(), Name: userName(*u)})
		}
	}
	p.mu.Lock()
	p.users[g.ID.String()] = known
	p.mu.Unlock()
	if chs, err := st.Cabinet.Channels(g.ID); err == nil {
		for _, ch := range chs {
			if !p.can(st, ch, discord.PermissionViewChannel) {
				continue
			}
			o.Channels = append(o.Channels, channel(ch, me, p.canSend(st, ch)))
		}
	}
	return o
}

// knowUser reports u in org once, before a message refers to it.
func (p *provider) knowUser(org *string, u discord.User) {
	key := ""
	if org != nil {
		key = *org
	}
	p.mu.Lock()
	known := p.users[key]
	if known == nil {
		known = map[discord.UserID]bool{}
		p.users[key] = known
	}
	seen := known[u.ID]
	known[u.ID] = true
	p.mu.Unlock()
	if !seen {
		p.cx.Directory(qmsg.UserUpserted{Organization: org, User: qmsg.User{ID: u.ID.String(), Name: userName(u)}})
	}
}

// can reports whether the account has perm in ch. Direct messages allow
// everything.
func (p *provider) can(st *ningen.State, ch discord.Channel, perm discord.Permissions) bool {
	if !ch.GuildID.IsValid() {
		return true
	}
	perms, err := st.Permissions(ch.ID, p.me)
	return err == nil && perms.Has(perm)
}

func (p *provider) canSend(st *ningen.State, ch discord.Channel) bool {
	need := discord.PermissionViewChannel | discord.PermissionSendMessages
	if isThread(ch.Type) {
		need = discord.PermissionViewChannel | discord.PermissionSendMessagesInThreads
	}
	return holdsMessages(ch.Type) && p.can(st, ch, need)
}

// handleEvents reports the session's events. arikawa calls these in order
// from the gateway's loop.
func (p *provider) handleEvents(st *ningen.State, lost chan<- error) {
	// Only events of the live session, after its directory.
	live := func() bool {
		p.mu.Lock()
		defer p.mu.Unlock()
		return p.synced && (p.st == st || p.st == nil)
	}
	st.AddHandler(func(ev *ningen.ConnectedEvent) {
		switch ev.Event.(type) {
		case *gateway.ReadyEvent:
			// A new session: anything may have changed. Commands can use
			// it from now, before Open returns.
			p.mu.Lock()
			p.st = st
			p.mu.Unlock()
			p.sync(st)
		case *gateway.ResumedEvent:
			// Discord replays what was missed, so the directory still holds.
			p.mu.Lock()
			p.synced = p.st == st
			p.mu.Unlock()
			p.cx.Status(qmsg.StatusReady{})
		}
	})
	st.AddHandler(func(ev *ningen.DisconnectedEvent) {
		if ev.IsLoggedOut() {
			select {
			case lost <- p.expired():
			default:
			}
			return
		}
		p.mu.Lock()
		p.synced = false
		p.mu.Unlock()
		p.cx.Status(qmsg.StatusDisconnected{Reason: fmt.Sprintf("gateway closed (%d); reconnecting", ev.Code)})
	})
	st.AddHandler(func(ev *gateway.MessageCreateEvent) {
		if !live() {
			return
		}
		p.knowUser(orgOf(ev.GuildID), ev.Author)
		p.emit(qmsg.Received{Message: message(ev.Message)})
	})
	st.AddHandler(func(ev *gateway.MessageUpdateEvent) {
		// Updates without an edit time only add embeds.
		if !live() || !ev.EditedTimestamp.IsValid() {
			return
		}
		updated := ev.Message
		if cached, err := st.Cabinet.Message(ev.ChannelID, ev.ID); err == nil {
			updated = *cached
		}
		p.emit(qmsg.Edited{
			Channel: channelRef(ev.GuildID, ev.ChannelID), ID: ev.ID.String(),
			Content: contents(updated), EditedAt: millis(ev.EditedTimestamp),
		})
	})
	st.AddHandler(func(ev *gateway.MessageDeleteEvent) {
		if live() {
			p.emit(qmsg.Deleted{Channel: channelRef(ev.GuildID, ev.ChannelID), ID: ev.ID.String()})
		}
	})
	st.AddHandler(func(ev *gateway.MessageDeleteBulkEvent) {
		if live() {
			for _, id := range ev.IDs {
				p.emit(qmsg.Deleted{Channel: channelRef(ev.GuildID, ev.ChannelID), ID: id.String()})
			}
		}
	})
	st.AddHandler(func(ev *gateway.MessageReactionAddEvent) {
		if live() {
			p.emit(qmsg.Reacted{Channel: channelRef(ev.GuildID, ev.ChannelID), ID: ev.MessageID.String(),
				User: ev.UserID.String(), Key: reactionKey(ev.Emoji), Added: true})
		}
	})
	st.AddHandler(func(ev *gateway.MessageReactionRemoveEvent) {
		if live() {
			p.emit(qmsg.Reacted{Channel: channelRef(ev.GuildID, ev.ChannelID), ID: ev.MessageID.String(),
				User: ev.UserID.String(), Key: reactionKey(ev.Emoji), Added: false})
		}
	})
	st.AddHandler(func(ev *gateway.TypingStartEvent) {
		if live() && ev.UserID != p.me {
			p.emit(qmsg.Typing{Channel: channelRef(ev.GuildID, ev.ChannelID), User: ev.UserID.String()})
		}
	})
	st.AddHandler(func(ev *gateway.MessageAckEvent) {
		if live() {
			ref := qmsg.ChannelRef{Channel: ev.ChannelID.String()}
			if ch, err := st.Cabinet.Channel(ev.ChannelID); err == nil {
				ref = channelRef(ch.GuildID, ch.ID)
			}
			p.emit(qmsg.Read{Channel: ref, User: p.me.String(), UpTo: qmsg.Str(ev.MessageID.String())})
		}
	})
	upsert := func(ch discord.Channel) {
		if !live() {
			return
		}
		if ch.GuildID.IsValid() && !p.can(st, ch, discord.PermissionViewChannel) {
			p.cx.Directory(qmsg.ChannelRemoved{Channel: channelRef(ch.GuildID, ch.ID)})
			return
		}
		for _, u := range ch.DMRecipients {
			p.knowUser(nil, u)
		}
		p.cx.Directory(qmsg.ChannelUpserted{Organization: orgOf(ch.GuildID), Channel: channel(ch, p.me, p.canSend(st, ch))})
	}
	st.AddHandler(func(ev *gateway.ChannelCreateEvent) { upsert(ev.Channel) })
	st.AddHandler(func(ev *gateway.ChannelUpdateEvent) { upsert(ev.Channel) })
	st.AddHandler(func(ev *gateway.ChannelDeleteEvent) {
		if live() {
			p.cx.Directory(qmsg.ChannelRemoved{Channel: channelRef(ev.GuildID, ev.ID)})
		}
	})
	st.AddHandler(func(ev *gateway.GuildCreateEvent) {
		if live() {
			p.cx.Directory(qmsg.OrganizationUpserted{Organization: p.organization(st, ev.Guild)})
		}
	})
	st.AddHandler(func(ev *gateway.GuildUpdateEvent) {
		if live() {
			p.cx.Directory(qmsg.OrganizationUpdated{ID: ev.ID.String(), Name: ev.Name})
		}
	})
	st.AddHandler(func(ev *gateway.GuildDeleteEvent) {
		if live() {
			p.cx.Directory(qmsg.OrganizationRemoved{ID: ev.ID.String()})
		}
	})
	st.AddHandler(func(ev *gateway.GuildMemberAddEvent) {
		if live() {
			p.knowUser(orgOf(ev.GuildID), ev.User)
		}
	})
	st.AddHandler(func(ev *gateway.GuildMemberRemoveEvent) {
		if live() {
			org := ev.GuildID.String()
			p.mu.Lock()
			delete(p.users[org], ev.User.ID)
			p.mu.Unlock()
			p.cx.Directory(qmsg.UserRemoved{Organization: &org, ID: ev.User.ID.String()})
		}
	})
}

// session returns the live session, or ErrLoginRequired.
func (p *provider) session() (*ningen.State, qmsg.CommandError) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.st == nil {
		return nil, qmsg.ErrLoginRequired{}
	}
	return p.st, nil
}

// commandError turns a Discord API error into the protocol's.
func commandError(err error, unknown qmsg.CommandError) qmsg.CommandError {
	var ce qmsg.CommandError
	if errors.As(err, &ce) {
		return ce
	}
	if errors.Is(err, context.Canceled) {
		return qmsg.ErrCancelled{}
	}
	var h *httputil.HTTPError
	if errors.As(err, &h) {
		switch h.Status {
		case http.StatusUnauthorized:
			return qmsg.ErrLoginRequired{}
		case http.StatusForbidden:
			return qmsg.ErrForbidden{Reason: h.Message}
		case http.StatusNotFound:
			if unknown != nil {
				return unknown
			}
		case http.StatusTooManyRequests:
			return qmsg.ErrRateLimited{}
		}
		return qmsg.ErrProvider{Code: fmt.Sprintf("discord_%d", h.Code), Message: h.Message, Details: qmsg.Integer(h.Status)}
	}
	return qmsg.ErrFailed{Reason: err.Error()}
}

// client is the session's REST client, bound to ctx.
func client(st *ningen.State, ctx context.Context) *api.Client {
	return st.Client.WithContext(ctx)
}

// hasToken reports whether a session token is stored, so a session is
// coming without a login.
func (p *provider) hasToken() bool {
	t, err := p.token()
	return err == nil && t != ""
}
