// Command fakediscord is a test double for the Discord provider's
// integration tests; it is never part of the provider. It serves:
//
//   - `fakediscord`: a Discord-like REST API and gateway over TLS, with a
//     certificate for discord.com and gateway.discord.gg from its own CA,
//     plus a plain HTTP control API for the test. It prints one JSON line:
//     {"tls": "127.0.0.1:port", "control": "http://...", "ca": "PEM"}.
//   - `fakediscord browser`: the browser login service's host API, as in
//     services/browser-login/README.md, without a browser. The test finishes
//     a login by posting to its viewer page's /complete. It prints
//     {"url": "http://127.0.0.1:port/"} like the real one.
//
// It holds one account, a friend, a guild with a writable, a read-only and a
// hidden channel, and a direct message channel.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/subtle"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "browser" {
		browser()
		return
	}
	discordServer()
}

const (
	token    = "fake-token"
	meID     = "1001"
	friendID = "1002"
	guildID  = "2001"
	general  = "3001"
	readonly = "3002"
	hidden   = "3003"
	dmID     = "3100"
)

type msg struct {
	ID          string           `json:"id"`
	ChannelID   string           `json:"channel_id"`
	GuildID     string           `json:"guild_id,omitempty"`
	Author      any              `json:"author"`
	Content     string           `json:"content"`
	Timestamp   string           `json:"timestamp"`
	Edited      *string          `json:"edited_timestamp"`
	Type        int              `json:"type"`
	Nonce       string           `json:"nonce,omitempty"`
	Mentions    []map[string]any `json:"mentions,omitempty"`
	Attachments []map[string]any `json:"attachments,omitempty"`
}

func user(id, name string) map[string]any {
	return map[string]any{"id": id, "username": name, "discriminator": "0", "global_name": strings.ToUpper(name[:1]) + name[1:]}
}

type server struct {
	mu                sync.Mutex
	token             string
	messages          map[string][]*msg // by channel, oldest first
	next              uint64
	gw                *websocket.Conn
	gwMu              sync.Mutex
	seq               int
	log               []string
	stats             map[string]int
	delay             time.Duration
	eventDelay        time.Duration
	holdResponse      bool
	blockMessageReads bool
}

func (s *server) id() string {
	s.next++
	return strconv.FormatUint(s.next, 10)
}

func now() string { return time.Now().UTC().Format(time.RFC3339Nano) }

func guildOf(ch string) string {
	if ch == dmID {
		return ""
	}
	return guildID
}

func (s *server) ready() map[string]any {
	everyone := 1<<10 | 1<<11 | 1<<16 | 1<<6
	ch := func(id, name string, overwrites ...map[string]any) map[string]any {
		if overwrites == nil {
			overwrites = []map[string]any{}
		}
		return map[string]any{"id": id, "type": 0, "name": name, "guild_id": guildID, "position": 0, "permission_overwrites": overwrites}
	}
	return map[string]any{
		"v": 9, "user": user(meID, "tester"), "session_id": "session-1",
		"resume_gateway_url": "wss://gateway.discord.gg",
		"private_channels":   []any{map[string]any{"id": dmID, "type": 1, "recipients": []any{user(friendID, "friend")}}},
		"guilds": []any{map[string]any{
			"id": guildID, "name": "Test Guild", "owner_id": friendID,
			"roles": []any{map[string]any{"id": guildID, "name": "@everyone", "permissions": strconv.Itoa(everyone)}},
			"channels": []any{
				ch(general, "general"),
				ch(readonly, "announcements", map[string]any{"id": guildID, "type": 0, "allow": "0", "deny": strconv.Itoa(1 << 11)}),
				ch(hidden, "secret", map[string]any{"id": guildID, "type": 0, "allow": "0", "deny": strconv.Itoa(1 << 10)}),
			},
			"members": []any{
				map[string]any{"user": user(meID, "tester"), "roles": []any{}, "joined_at": now()},
				map[string]any{"user": user(friendID, "friend"), "roles": []any{}, "joined_at": now()},
			},
		}},
	}
}

func (s *server) dispatch(t string, d any) {
	s.gwMu.Lock()
	defer s.gwMu.Unlock()
	if s.gw == nil {
		return
	}
	s.seq++
	s.gw.WriteJSON(map[string]any{"op": 0, "t": t, "s": s.seq, "d": d})
}

// Snapshot before delaying, since subsequent REST changes may mutate the message.
func (s *server) dispatchLater(kind string, data any) {
	payload, _ := json.Marshal(data)
	delay := s.eventDelay // caller holds s.mu
	go func() { time.Sleep(delay); s.dispatch(kind, json.RawMessage(payload)) }()
}

func (s *server) gateway(w http.ResponseWriter, r *http.Request) {
	c, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
	if err != nil {
		return
	}
	c.WriteJSON(map[string]any{"op": 10, "d": map[string]any{"heartbeat_interval": 30000}})
	for {
		var p struct {
			Op int             `json:"op"`
			D  json.RawMessage `json:"d"`
		}
		if err := c.ReadJSON(&p); err != nil {
			s.gwMu.Lock()
			if s.gw == c {
				s.gw = nil
			}
			s.gwMu.Unlock()
			return
		}
		switch p.Op {
		case 1:
			s.gwMu.Lock()
			c.WriteJSON(map[string]any{"op": 11})
			s.gwMu.Unlock()
		case 2, 6:
			var d struct {
				Token string `json:"token"`
			}
			json.Unmarshal(p.D, &d)
			s.mu.Lock()
			valid := d.Token == s.token
			if p.Op == 2 {
				s.stats["identify"]++
			} else {
				s.stats["resume"]++
			}
			s.mu.Unlock()
			if !valid {
				c.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(4004, "Authentication failed."), time.Now().Add(time.Second))
				c.Close()
				return
			}
			s.gwMu.Lock()
			if s.gw != nil && s.gw != c {
				s.gw.Close()
			}
			s.gw = c
			s.gwMu.Unlock()
			if p.Op == 2 {
				s.dispatch("READY", s.ready())
			} else {
				s.dispatch("RESUMED", map[string]any{})
			}
		}
	}
}

type apiError struct {
	Message string `json:"message"`
	Code    int    `json:"code"`
}

func reply(w http.ResponseWriter, status int, v any) {
	if v == nil {
		w.WriteHeader(status)
		return
	}
	w.Header().Set("content-type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(v)
}

func (s *server) api(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/api/v9")
	s.mu.Lock()
	s.log = append(s.log, r.Method+" "+path)
	authorized := r.Header.Get("Authorization") == s.token
	delay := s.delay
	holdResponse := s.holdResponse
	blockReads := s.blockMessageReads
	s.mu.Unlock()
	if path == "/big" {
		// A large body, for transfer tests.
		n, _ := strconv.Atoi(r.URL.Query().Get("n"))
		w.Header().Set("content-length", strconv.Itoa(n))
		chunk := make([]byte, 64<<10)
		for n > 0 {
			k, err := w.Write(chunk[:min(n, len(chunk))])
			if err != nil {
				return
			}
			n -= k
		}
		return
	}
	if path == "/gateway" {
		reply(w, 200, map[string]string{"url": "wss://gateway.discord.gg"})
		return
	}
	if !authorized {
		reply(w, 401, apiError{"401: Unauthorized", 0})
		return
	}
	parts := strings.Split(strings.Trim(path, "/"), "/")
	if len(parts) == 3 && parts[0] == "users" && parts[2] == "channels" && r.Method == "POST" {
		var body struct {
			RecipientID string `json:"recipient_id"`
		}
		json.NewDecoder(r.Body).Decode(&body)
		if body.RecipientID != friendID {
			reply(w, 400, apiError{"Invalid Recipient(s)", 50033})
			return
		}
		reply(w, 200, map[string]any{"id": dmID, "type": 1, "recipients": []any{user(friendID, "friend")}})
		return
	}
	if len(parts) < 3 || parts[0] != "channels" {
		reply(w, 404, apiError{"404: Not Found", 0})
		return
	}
	ch := parts[1]
	if ch != general && ch != readonly && ch != dmID {
		reply(w, 404, apiError{"Unknown Channel", 10003})
		return
	}
	if parts[2] == "typing" && r.Method == "POST" {
		reply(w, 204, nil)
		return
	}
	if parts[2] != "messages" {
		reply(w, 404, apiError{"404: Not Found", 0})
		return
	}
	if blockReads && r.Method == "GET" && (len(parts) == 4 || r.URL.Query().Get("around") != "") {
		s.mu.Lock()
		s.stats["discovery_blocked"]++
		s.mu.Unlock()
		<-r.Context().Done()
		s.mu.Lock()
		s.stats["discovery_cancelled"]++
		s.mu.Unlock()
		return
	}
	if delay > 0 && r.Method != "GET" {
		select {
		case <-time.After(delay):
		case <-r.Context().Done():
			return
		}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(parts) == 3 {
		switch r.Method {
		case "GET":
			before, _ := strconv.ParseUint(r.URL.Query().Get("before"), 10, 64)
			limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
			out := []*msg{}
			list := s.messages[ch]
			if around := r.URL.Query().Get("around"); around != "" {
				for _, message := range list {
					if message.ID == around {
						reply(w, 200, []*msg{message})
						return
					}
				}
				// A missing target may return a neighbour, as real history can.
			}
			for i := len(list) - 1; i >= 0 && len(out) < limit; i-- {
				id, _ := strconv.ParseUint(list[i].ID, 10, 64)
				if before == 0 || id < before {
					out = append(out, list[i])
				}
			}
			reply(w, 200, out)
		case "POST":
			if ch == readonly {
				reply(w, 403, apiError{"Missing Permissions", 50013})
				return
			}

			var body struct {
				Content         string `json:"content"`
				Nonce           string `json:"nonce"`
				AllowedMentions struct {
					Parse []string `json:"parse"`
				} `json:"allowed_mentions"`
			}
			var attachments []map[string]any
			if strings.HasPrefix(r.Header.Get("Content-Type"), "multipart/") {
				reader, err := r.MultipartReader()
				if err != nil {
					reply(w, 400, apiError{"bad multipart", 0})
					return
				}
				for {
					part, err := reader.NextPart()
					if err == io.EOF {
						break
					}
					if err != nil {
						reply(w, 400, apiError{"bad multipart", 0})
						return
					}
					data, err := io.ReadAll(io.LimitReader(part, 10*1024*1024+1))
					if err != nil || len(data) > 10*1024*1024 {
						reply(w, 400, apiError{"bad upload", 0})
						return
					}
					if part.FormName() == "payload_json" {
						if json.Unmarshal(data, &body) != nil {
							reply(w, 400, apiError{"bad payload", 0})
							return
						}
					} else {
						s.stats["uploaded_bytes"] += len(data)
						attachments = append(attachments, map[string]any{"id": s.id(), "filename": part.FileName(), "size": len(data), "content_type": http.DetectContentType(data), "url": "https://cdn.discordapp.com/attachments/test/file?ex=ffffffff", "width": 640, "height": 480})
					}
				}
			} else if json.NewDecoder(r.Body).Decode(&body) != nil {
				reply(w, 400, apiError{"bad JSON", 0})
				return
			}
			if len(body.Nonce) > 25 {
				reply(w, 400, apiError{"Invalid Form Body", 50035})
				return
			}
			m := &msg{ID: s.id(), ChannelID: ch, GuildID: guildOf(ch), Author: user(meID, "tester"), Content: body.Content, Timestamp: now(), Nonce: body.Nonce, Attachments: attachments}

			if strings.Contains(body.Content, "<@"+friendID+">") {
				for _, kind := range body.AllowedMentions.Parse {
					if kind == "users" {
						m.Mentions = []map[string]any{user(friendID, "friend")}
						s.stats["user_mentions"]++
						break
					}
				}
			}
			s.messages[ch] = append(s.messages[ch], m)
			s.stats["accepted"]++
			s.dispatchLater("MESSAGE_CREATE", m)
			if holdResponse {
				s.mu.Unlock()
				<-r.Context().Done()
				s.mu.Lock()
				return
			}
			reply(w, 200, m)
		default:
			reply(w, 405, apiError{"405: Method Not Allowed", 0})
		}
		return
	}
	if r.Method == "GET" {
		reply(w, 403, apiError{"Only bots can use this endpoint.", 20002})
		return
	}
	var m *msg
	for _, c := range s.messages[ch] {
		if c.ID == parts[3] {
			m = c
		}
	}
	if m == nil {
		reply(w, 404, apiError{"Unknown Message", 10008})
		return
	}
	ref := map[string]any{"id": m.ID, "channel_id": ch, "guild_id": guildOf(ch)}
	switch {
	case len(parts) == 4 && r.Method == "GET":
		reply(w, 200, m)
	case len(parts) == 4 && r.Method == "PATCH":
		var body struct {
			Content string `json:"content"`
		}
		json.NewDecoder(r.Body).Decode(&body)
		t := now()
		m.Content, m.Edited = body.Content, &t
		reply(w, 200, m)
		s.dispatchLater("MESSAGE_UPDATE", m)
	case len(parts) == 4 && r.Method == "DELETE":
		list := s.messages[ch]
		for i, c := range list {
			if c == m {
				s.messages[ch] = append(list[:i:i], list[i+1:]...)
			}
		}
		reply(w, 204, nil)
		s.dispatchLater("MESSAGE_DELETE", ref)
	case len(parts) == 7 && parts[4] == "reactions" && parts[6] == "@me":
		ev := map[string]any{"user_id": meID, "channel_id": ch, "message_id": m.ID, "guild_id": guildOf(ch), "emoji": map[string]any{"id": nil, "name": parts[5]}}
		reply(w, 204, nil)
		if r.Method == "PUT" {
			s.dispatchLater("MESSAGE_REACTION_ADD", ev)
		} else {
			s.dispatchLater("MESSAGE_REACTION_REMOVE", ev)
		}
	case len(parts) == 5 && parts[4] == "ack" && r.Method == "POST":
		reply(w, 200, map[string]any{"token": nil})
		s.dispatchLater("MESSAGE_ACK", map[string]any{"channel_id": ch, "message_id": m.ID, "version": 1})
	default:
		reply(w, 404, apiError{"404: Not Found", 0})
	}
}

// control lets the test act as Discord and the other users.
func (s *server) control(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case "/history-author":
		s.mu.Lock()
		for _, message := range s.messages[general] {
			message.Author = user("1009", "history-user")
		}
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/message":
		// A message from the friend in channel ?channel=.
		ch := r.URL.Query().Get("channel")
		s.mu.Lock()
		m := &msg{ID: s.id(), ChannelID: ch, GuildID: guildOf(ch), Author: user(friendID, "friend"), Content: r.URL.Query().Get("content"), Timestamp: now()}
		s.messages[ch] = append(s.messages[ch], m)
		s.mu.Unlock()
		s.dispatch("MESSAGE_CREATE", m)
		reply(w, 200, m)
	case "/typing":
		s.dispatch("TYPING_START", map[string]any{"channel_id": general, "guild_id": guildID, "user_id": friendID, "timestamp": time.Now().Unix()})
		reply(w, 204, nil)
	case "/drop":
		// Closes the gateway with ?code=, as Discord does.
		code, _ := strconv.Atoi(r.URL.Query().Get("code"))
		s.gwMu.Lock()
		if s.gw != nil {
			s.gw.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(code, "test"), time.Now().Add(time.Second))
			s.gw.Close()
			s.gw = nil
		}
		s.gwMu.Unlock()
		reply(w, 204, nil)
	case "/revoke":
		s.mu.Lock()
		s.token = "revoked"
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/delay":
		ms, _ := strconv.Atoi(r.URL.Query().Get("ms"))
		s.mu.Lock()
		s.delay = time.Duration(ms) * time.Millisecond
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/event-delay":
		ms, _ := strconv.Atoi(r.URL.Query().Get("ms"))
		s.mu.Lock()
		s.eventDelay = time.Duration(ms) * time.Millisecond
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/hold-response":
		s.mu.Lock()
		s.holdResponse = r.URL.Query().Get("on") == "1"
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/block-discovery":
		s.mu.Lock()
		s.blockMessageReads = r.URL.Query().Get("on") == "1"
		s.mu.Unlock()
		reply(w, 204, nil)
	case "/state":
		s.mu.Lock()
		defer s.mu.Unlock()
		reply(w, 200, map[string]any{"requests": s.log, "stats": s.stats})
	default:
		reply(w, 404, nil)
	}
}

func discordServer() {
	caPEM, leaf := certificates()
	s := &server{token: token, messages: map[string][]*msg{}, next: 1 << 40, stats: map[string]int{}}
	// History to page through.
	for i := 0; i < 5; i++ {
		s.messages[general] = append(s.messages[general], &msg{ID: s.id(), ChannelID: general, GuildID: guildID,
			Author: user(friendID, "friend"), Content: fmt.Sprintf("old %d", i), Timestamp: now()})
	}
	tlsLn, err := tls.Listen("tcp", "127.0.0.1:0", &tls.Config{Certificates: []tls.Certificate{leaf}})
	if err != nil {
		panic(err)
	}
	mux := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if websocket.IsWebSocketUpgrade(r) && strings.HasPrefix(r.Host, "gateway.discord.gg") {
			s.gateway(w, r)
			return
		}
		s.api(w, r)
	})
	go http.Serve(tlsLn, mux)
	ctl, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		panic(err)
	}
	go http.Serve(ctl, http.HandlerFunc(s.control))
	json.NewEncoder(os.Stdout).Encode(map[string]string{
		"tls": tlsLn.Addr().String(), "control": "http://" + ctl.Addr().String(), "ca": caPEM,
	})
	// Exits with the test, which holds stdin.
	os.Stdin.Read(make([]byte, 1))
}

func certificates() (string, tls.Certificate) {
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "fakediscord CA"},
		NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(24 * time.Hour),
		IsCA: true, KeyUsage: x509.KeyUsageCertSign, BasicConstraintsValid: true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		panic(err)
	}
	ca, _ := x509.ParseCertificate(der)
	leafKey, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	leafTmpl := &x509.Certificate{
		SerialNumber: big.NewInt(2), Subject: pkix.Name{CommonName: "discord.com"},
		DNSNames:  []string{"discord.com", "gateway.discord.gg", "cdn.discordapp.com"},
		NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(24 * time.Hour),
		KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}
	leafDER, err := x509.CreateCertificate(rand.Reader, leafTmpl, ca, &leafKey.PublicKey, key)
	if err != nil {
		panic(err)
	}
	caPEM := string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}))
	return caPEM, tls.Certificate{Certificate: [][]byte{leafDER}, PrivateKey: leafKey}
}

// The browser login service's host API, without a browser.

type login struct {
	ID, Viewer, State, Token string
}

func browser() {
	key := os.Getenv("QMSG_BROWSER_KEY")
	if len(key) < 32 {
		fmt.Fprintln(os.Stderr, "QMSG_BROWSER_KEY must have at least 32 characters")
		os.Exit(1)
	}
	var mu sync.Mutex
	logins := map[string]*login{}
	var events []string
	n := 0
	ln, err := net.Listen("tcp", "127.0.0.1:"+os.Getenv("QMSG_BROWSER_PORT"))
	if err != nil {
		panic(err)
	}
	handler := func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/")
		if parts[0] == "events" {
			reply(w, 200, events)
			return
		}
		if parts[0] == "view" && len(parts) >= 2 {
			var l *login
			for _, c := range logins {
				if c.Viewer == parts[1] {
					l = c
				}
			}
			if l == nil {
				reply(w, 404, nil)
				return
			}
			if len(parts) == 3 && parts[2] == "complete" && r.Method == "POST" {
				l.State, l.Token = "complete", r.URL.Query().Get("token")
				events = append(events, "complete "+l.ID)
			}
			reply(w, 200, map[string]string{"state": l.State})
			return
		}
		if parts[0] != "sessions" || subtle.ConstantTimeCompare([]byte(r.Header.Get("Authorization")), []byte("Bearer "+key)) != 1 {
			reply(w, 401, map[string]string{"error": "Unauthorized"})
			return
		}
		if len(parts) == 1 && r.Method == "POST" {
			var body struct {
				Provider string
				ID       string
			}
			json.NewDecoder(r.Body).Decode(&body)
			if body.Provider != "discord" {
				reply(w, 400, map[string]string{"error": "Unknown provider"})
				return
			}
			if len(logins) >= 4 {
				reply(w, 429, map[string]string{"error": "Login limit reached"})
				return
			}
			n++
			id := body.ID
			if id == "" {
				id = fmt.Sprint("login-", n)
			}
			l := &login{ID: id, Viewer: fmt.Sprint("viewer-", n), State: "active"}
			logins[l.ID] = l
			events = append(events, "create "+l.ID)
			reply(w, 201, map[string]any{"id": l.ID, "path": "/view/" + l.Viewer, "expiresAt": time.Now().Add(10 * time.Minute).UnixMilli()})
			return
		}
		l := logins[parts[len(parts)-1]]
		if len(parts) == 3 {
			l = logins[parts[1]]
		}
		if l == nil {
			reply(w, 404, map[string]string{"error": "No such session"})
			return
		}
		switch {
		case len(parts) == 2 && r.Method == "GET":
			reply(w, 200, map[string]string{"state": l.State})
		case len(parts) == 2 && r.Method == "DELETE":
			delete(logins, l.ID)
			events = append(events, "delete "+l.ID)
			reply(w, 200, map[string]string{"state": "cancelled"})
		case len(parts) == 3 && parts[2] == "claim" && r.Method == "POST" && l.State == "complete":
			delete(logins, l.ID)
			events = append(events, "claim "+l.ID)
			reply(w, 200, map[string]string{"token": l.Token})
		default:
			reply(w, 409, map[string]string{"error": "No result"})
		}
	}
	fmt.Printf("{\"url\":\"http://%s/\"}\n", ln.Addr())
	srv := &http.Server{Handler: http.HandlerFunc(handler)}
	srv.Serve(ln)
}
