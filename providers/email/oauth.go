package main

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	qmsg "github.com/voltaero/qmsg/sdk/go"
)

type oauthLogin struct {
	state, verifier, redirect string
	deadline                  time.Time
}

var oauthResultType = qmsg.TypeRecord{Fields: []qmsg.Field{
	{ID: "url", Label: "Open in browser", ValueType: qmsg.TypeText{}, Required: true},
	{ID: "state", Label: "Expected callback state", ValueType: qmsg.TypeText{}, Required: true},
	{ID: "expires_at", Label: "Expiry (Unix seconds)", ValueType: qmsg.TypeInteger{}, Required: true},
}}

// Desktop native clients create the listener on their device, not the qmsg host.
// iOS clients use their registered Google client ID scheme instead.
func callbackURI(raw string) error {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "http" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Opaque != "" || (u.Hostname() != "127.0.0.1" && u.Hostname() != "::1") {
		return fmt.Errorf("use an HTTP loopback IP callback on the client device")
	}
	port, err := strconv.Atoi(u.Port())
	if err != nil || port < 1 || port > 65535 || len(raw) > 2048 {
		return fmt.Errorf("callback needs the client's listening port")
	}
	return nil
}
func randomOAuth() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}
func (p *provider) beginOAuth(redirect string) (qmsg.Reply, error) {
	if p.cfg.clientID == "" {
		return nil, fmt.Errorf("qmsg needs its registered Google OAuth client ID")
	}
	if p.cfg.oauthRedirect != "" {
		expected := "com.googleusercontent.apps." + strings.TrimSuffix(p.cfg.clientID, ".apps.googleusercontent.com") + ":/oauth2redirect"
		if !strings.HasSuffix(p.cfg.clientID, ".apps.googleusercontent.com") || p.cfg.oauthRedirect != expected || (redirect != "" && redirect != expected) {
			return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "redirect_uri", Message: "Use the registered iOS callback for this Google client."}}}
		}
		redirect = expected
	} else if err := callbackURI(redirect); err != nil {
		return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "redirect_uri", Message: err.Error()}}}
	}
	state, err := randomOAuth()
	if err != nil {
		return nil, err
	}
	verifier, err := randomOAuth()
	if err != nil {
		return nil, err
	}
	challenge := sha256.Sum256([]byte(verifier))
	pending := &oauthLogin{state: state, verifier: verifier, redirect: redirect, deadline: time.Now().Add(10 * time.Minute)}
	form := url.Values{"client_id": {p.cfg.clientID}, "redirect_uri": {redirect}, "response_type": {"code"}, "scope": {"https://mail.google.com/"}, "access_type": {"offline"}, "prompt": {"consent"}, "state": {state}, "code_challenge_method": {"S256"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(challenge[:])}, "login_hint": {p.cfg.address}}
	link := "https://accounts.google.com/o/oauth2/v2/auth?" + form.Encode()
	p.pending = pending
	p.status(qmsg.StatusLoginRequired{Step: qmsg.LoginStep{Message: "Open the sign-in link in your browser.", Function: qmsg.Str("finish_login"), Link: qmsg.Str(link)}})
	return qmsg.ReplyValue{Value: qmsg.Record{"url": qmsg.Text(link), "state": qmsg.Text(state), "expires_at": qmsg.Integer(pending.deadline.Unix())}}, nil
}
func (p *provider) finishOAuth(ctx context.Context, code, state string) (credential, error) {
	pending := p.pending
	if pending == nil || !time.Now().Before(pending.deadline) {
		p.pending = nil
		p.loginRequired()
		return credential{}, fmt.Errorf("OAuth login expired; start again")
	}
	if subtle.ConstantTimeCompare([]byte(state), []byte(pending.state)) != 1 || code == "" || len(code) > 4096 {
		return credential{}, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "oauth_callback", Message: "Invalid OAuth callback code or state"}}}
	}
	// Consume before network I/O: an ambiguous exchange must never replay a code.
	p.pending = nil
	p.loginRequired()
	form := url.Values{"client_id": {p.cfg.clientID}, "redirect_uri": {pending.redirect}, "grant_type": {"authorization_code"}, "code": {code}, "code_verifier": {pending.verifier}}
	if p.cfg.clientSecret != "" {
		form.Set("client_secret", p.cfg.clientSecret)
	}
	req, _ := http.NewRequestWithContext(ctx, "POST", "https://oauth2.googleapis.com/token", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	resp, err := p.cfg.httpClient().Do(req)
	if err != nil {
		return credential{}, fmt.Errorf("OAuth exchange connection failed; start a new login")
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 65537))
	if err != nil || len(body) > 65536 || resp.StatusCode != 200 {
		return credential{}, fmt.Errorf("OAuth exchange failed; start a new login")
	}
	var token struct {
		Access  string `json:"access_token"`
		Refresh string `json:"refresh_token"`
		Type    string `json:"token_type"`
		Expires int64  `json:"expires_in"`
	}
	if json.Unmarshal(body, &token) != nil || token.Access == "" || token.Refresh == "" || !strings.EqualFold(token.Type, "Bearer") || token.Expires <= 0 || token.Expires > 86400 || len(token.Access)+len(token.Refresh) > 8192 {
		return credential{}, fmt.Errorf("invalid OAuth token response")
	}
	return credential{AccessToken: token.Access, RefreshToken: token.Refresh, ClientID: p.cfg.clientID, ClientSecret: p.cfg.clientSecret, ExpiresAt: time.Now().Unix() + token.Expires}, nil
}
