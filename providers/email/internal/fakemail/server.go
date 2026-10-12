// Package fakemail runs isolated TLS mail servers for provider tests.
package fakemail

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
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
	"strings"
	"sync"
	"sync/atomic"
	"time"

	imap "github.com/emersion/go-imap/v2"
	"github.com/emersion/go-imap/v2/imapserver"
	"github.com/emersion/go-imap/v2/imapserver/imapmemserver"
	"github.com/emersion/go-sasl"
	smtp "github.com/emersion/go-smtp"
)

const Address = "alice@example.test"
const Password = "fixture-password"

type Server struct {
	oauth                           *http.Server
	Exchanges                       atomic.Int64
	Refreshes                       atomic.Int64
	Fetches                         atomic.Int64
	FullBodyFetches                 atomic.Int64
	RejectRefresh                   atomic.Bool
	IMAP, SMTP, StartTLS, OAuth, CA string
	User                            *imapmemserver.User
	imap                            *imapserver.Server
	smtp, starttls                  *smtp.Server
	mu                              sync.Mutex
	Mails                           [][]byte
	Recipients                      [][]string
	DropDATA                        atomic.Bool
	DenySent                        atomic.Bool
}

func Start() (*Server, error) {
	key, e := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if e != nil {
		return nil, e
	}
	cert := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "Mail test only"}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), DNSNames: []string{"imap.example.test", "smtp.example.test", "imap.gmail.com", "smtp.gmail.com", "oauth2.googleapis.com"}, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}, IsCA: true, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	der, e := x509.CreateCertificate(rand.Reader, cert, cert, &key.PublicKey, key)
	if e != nil {
		return nil, e
	}
	ca := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	kb, e := x509.MarshalECPrivateKey(key)
	if e != nil {
		return nil, e
	}
	pair, e := tls.X509KeyPair(ca, pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: kb}))
	if e != nil {
		return nil, e
	}
	tc := &tls.Config{Certificates: []tls.Certificate{pair}, MinVersion: tls.VersionTLS12}
	s := &Server{CA: string(ca), User: imapmemserver.NewUser(Address, Password)}
	for _, f := range []string{"INBOX", "Sent", "Trash"} {
		if e = s.User.Create(f, nil); e != nil {
			return nil, e
		}
	}
	mem := imapmemserver.New()
	mem.AddUser(s.User)
	s.imap = imapserver.New(&imapserver.Options{NewSession: func(*imapserver.Conn) (imapserver.Session, *imapserver.GreetingData, error) {
		return &session{Session: mem.NewSession(), s: s}, nil, nil
	}, Caps: imap.CapSet{imap.CapIMAP4rev1: {}, imap.CapMove: {}, imap.CapUIDPlus: {}, imap.CapListExtended: {}, imap.CapSpecialUse: {}}, TLSConfig: tc})
	li, e := tls.Listen("tcp", "127.0.0.1:0", tc)
	if e != nil {
		return nil, e
	}
	s.IMAP = li.Addr().String()
	go s.imap.Serve(li)
	backend := smtp.BackendFunc(func(c *smtp.Conn) (smtp.Session, error) { return &smtpSession{s: s, conn: c}, nil })
	s.smtp = smtp.NewServer(backend)
	s.smtp.Domain = "example.test"
	s.smtp.MaxMessageBytes = 8 << 20
	s.smtp.ReadTimeout = 45 * time.Second
	s.smtp.WriteTimeout = 45 * time.Second
	ls, e := tls.Listen("tcp", "127.0.0.1:0", tc)
	if e != nil {
		s.Close()
		return nil, e
	}
	s.SMTP = ls.Addr().String()
	go s.smtp.Serve(ls)
	s.starttls = smtp.NewServer(backend)
	s.starttls.Domain = "example.test"
	s.starttls.TLSConfig = tc
	s.starttls.MaxMessageBytes = 8 << 20
	lp, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		s.Close()
		return nil, e
	}
	s.StartTLS = lp.Addr().String()
	go s.starttls.Serve(lp)
	lo, e := tls.Listen("tcp", "127.0.0.1:0", tc)
	if e != nil {
		s.Close()
		return nil, e
	}
	s.OAuth = lo.Addr().String()
	s.oauth = &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r.ParseForm()
		client := r.Form.Get("client_id")
		valid := r.URL.Path == "/token" && (client == "fixture-client" || client == "123-qmsg.apps.googleusercontent.com")
		if r.Form.Get("grant_type") == "authorization_code" {
			s.Exchanges.Add(1)
			redirectOK := strings.HasPrefix(r.Form.Get("redirect_uri"), "http://127.0.0.1:")
			if client == "123-qmsg.apps.googleusercontent.com" {
				redirectOK = r.Form.Get("redirect_uri") == "com.googleusercontent.apps.123-qmsg:/oauth2redirect"
			}
			valid = valid && r.Form.Get("code") == "fixture-code" && len(r.Form.Get("code_verifier")) == 43 && redirectOK
		} else {
			s.Refreshes.Add(1)
			valid = valid && r.Form.Get("grant_type") == "refresh_token" && r.Form.Get("refresh_token") == "fixture-refresh" && !s.RejectRefresh.Load()
		}
		if !valid {
			w.WriteHeader(400)
			json.NewEncoder(w).Encode(map[string]string{"error": "invalid_grant"})
			return
		}
		// A short first token makes the provider refresh before opening IMAP.
		expires := 3600
		if r.Form.Get("grant_type") == "authorization_code" {
			expires = 1
		}
		json.NewEncoder(w).Encode(map[string]any{"access_token": "fixture-access", "refresh_token": "fixture-refresh", "expires_in": expires, "token_type": "Bearer"})
	})}
	go s.oauth.Serve(lo)
	return s, nil
}
func (s *Server) Close() {
	if s.oauth != nil {
		s.oauth.Close()
	}
	if s.imap != nil {
		s.imap.Close()
	}
	if s.smtp != nil {
		s.smtp.Close()
	}
	if s.starttls != nil {
		s.starttls.Close()
	}
}

type literal struct{ *bytes.Reader }

func (l literal) Size() int64 { return int64(l.Len()) }
func (s *Server) Append(folder string, raw []byte) error {
	_, e := s.User.Append(folder, literal{bytes.NewReader(raw)}, &imap.AppendOptions{})
	return e
}
func (s *Server) Snapshot() ([][]byte, [][]string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([][]byte{}, s.Mails...), append([][]string{}, s.Recipients...)
}

type session struct {
	imapserver.Session
	s *Server
}

func (s *session) Fetch(w *imapserver.FetchWriter, set imap.NumSet, opts *imap.FetchOptions) error {
	s.s.Fetches.Add(1)
	for _, section := range opts.BodySection {
		if section.Specifier == "" && len(section.Part) == 0 {
			s.s.FullBodyFetches.Add(1)
		}
	}
	return s.Session.Fetch(w, set, opts)
}

func (s *session) Append(mailbox string, r imap.LiteralReader, opts *imap.AppendOptions) (*imap.AppendData, error) {
	if mailbox == "Sent" && s.s.DenySent.Load() {
		return nil, fmt.Errorf("Sent storage unavailable")
	}
	return s.Session.Append(mailbox, r, opts)
}

// Embedding only Session hides MOVE's extension interface; forward it explicitly.
func (s *session) Move(w *imapserver.MoveWriter, set imap.NumSet, dest string) error {
	return s.Session.(imapserver.SessionMove).Move(w, set, dest)
}

type smtpSession struct {
	s      *Server
	conn   *smtp.Conn
	authed bool
	to     []string
}

func (s *smtpSession) Reset()        { s.to = nil }
func (s *smtpSession) Logout() error { return nil }
func (s *smtpSession) Mail(string, *smtp.MailOptions) error {
	if !s.authed {
		return smtp.ErrAuthRequired
	}
	return nil
}
func (s *smtpSession) Rcpt(to string, _ *smtp.RcptOptions) error { s.to = append(s.to, to); return nil }
func (s *smtpSession) Data(r io.Reader) error {
	b, e := io.ReadAll(io.LimitReader(r, 8<<20+1))
	if e != nil {
		return e
	}
	s.s.mu.Lock()
	s.s.Mails = append(s.s.Mails, b)
	s.s.Recipients = append(s.s.Recipients, append([]string{}, s.to...))
	s.s.mu.Unlock()
	if e = s.s.Append("INBOX", b); e != nil {
		return e
	}
	if s.s.DropDATA.Load() {
		s.conn.Close()
	}
	return nil
}
func (s *smtpSession) AuthMechanisms() []string { return []string{"PLAIN", "XOAUTH2"} }
func (s *smtpSession) Auth(mech string) (sasl.Server, error) {
	if mech == "XOAUTH2" {
		return &oauthSASL{success: func() error { s.authed = true; return nil }}, nil
	}
	if mech != "PLAIN" {
		return nil, smtp.ErrAuthUnknownMechanism
	}
	return sasl.NewPlainServer(func(_, user, password string) error {
		if user != Address || password != Password {
			return smtp.ErrAuthFailed
		}
		s.authed = true
		return nil
	}), nil
}

type oauthSASL struct{ success func() error }

func (x *oauthSASL) Next(response []byte) ([]byte, bool, error) {
	if response == nil {
		return nil, false, nil
	}
	if string(response) != "user="+Address+"\x01auth=Bearer fixture-access\x01\x01" {
		return nil, false, smtp.ErrAuthFailed
	}
	return nil, true, x.success()
}
func (s *session) AuthenticateMechanisms() []string { return []string{"PLAIN", "XOAUTH2"} }
func (s *session) Authenticate(mech string) (sasl.Server, error) {
	if strings.EqualFold(mech, "XOAUTH2") {
		return &oauthSASL{success: func() error { return s.Login(Address, Password) }}, nil
	}
	if mech == "PLAIN" {
		return sasl.NewPlainServer(func(_, user, password string) error { return s.Login(user, password) }), nil
	}
	return nil, imapserver.ErrAuthFailed
}
