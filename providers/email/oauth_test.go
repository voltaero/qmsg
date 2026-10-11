package main

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	qmsg "github.com/voltaero/qmsg/sdk/go"
	"net/url"
	"testing"
	"time"
)

func TestOAuthNativeCallback(t *testing.T) {
	p, s := testProvider(t)
	p.ready = false
	p.cfg.service = "gmail"
	p.cfg.clientID = "fixture-client"
	p.cfg.redirects["oauth2.googleapis.com"] = s.OAuth
	r, e := p.beginOAuth("http://127.0.0.1:43123/callback")
	if e != nil {
		t.Fatal(e)
	}
	values := r.(qmsg.ReplyValue).Value.(qmsg.Record)
	u, _ := url.Parse(string(values["url"].(qmsg.Text)))
	params := u.Query()
	challenge := sha256.Sum256([]byte(p.pending.verifier))
	if params.Get("code_challenge") != base64.RawURLEncoding.EncodeToString(challenge[:]) || params.Get("code_challenge_method") != "S256" || len(p.pending.state) != 43 {
		t.Fatal("missing PKCE/state")
	}
	oldState := p.pending.state
	if _, e = p.beginOAuth("http://127.0.0.1:43124/callback"); e != nil {
		t.Fatal(e)
	}
	if _, e = p.finishOAuth(context.Background(), "fixture-code", oldState); e == nil {
		t.Fatal("replaced login accepted")
	}
	state := p.pending.state
	if _, e = p.finishOAuth(context.Background(), "fixture-code", "wrong"); e == nil {
		t.Fatal("wrong state accepted")
	}
	if s.Exchanges.Load() != 0 || p.pending == nil {
		t.Fatal("wrong state changed login")
	}
	cred, e := p.finishOAuth(context.Background(), "fixture-code", state)
	if e != nil {
		t.Fatal(e)
	}
	if cred.RefreshToken != "fixture-refresh" || cred.ClientID != "fixture-client" || s.Exchanges.Load() != 1 {
		t.Fatal("bad exchange")
	}
	if _, e = p.finishOAuth(context.Background(), "fixture-code", state); e == nil || s.Exchanges.Load() != 1 {
		t.Fatal("replay accepted")
	}
	p.beginOAuth("http://127.0.0.1:43123/callback")
	state = p.pending.state
	p.pending.deadline = time.Now().Add(-time.Second)
	if _, e = p.finishOAuth(context.Background(), "fixture-code", state); e == nil || s.Exchanges.Load() != 1 {
		t.Fatal("expired login accepted")
	}
	p.beginOAuth("http://127.0.0.1:43123/callback")
	if _, e = p.call(context.Background(), qmsg.FunctionCall{Function: "cancel_login", Context: qmsg.InProvider{}, Arguments: qmsg.Arguments{}}); e != nil || p.pending != nil {
		t.Fatalf("cancel: %v", e)
	}
}
func TestOAuthRedirectValidation(t *testing.T) {
	for _, u := range []string{"https://remote.test/callback", "http://localhost:123/callback", "http://127.0.0.1/callback", "http://127.0.0.1:0/callback", "http://127.0.0.1:65536/callback", "http://user@127.0.0.1:123/callback", "http://127.0.0.1:123/callback?x=1", "http://127.0.0.1:123/callback#x"} {
		if callbackURI(u) == nil {
			t.Errorf("accepted %s", u)
		}
	}
	for _, u := range []string{"http://127.0.0.1:123/callback", "http://[::1]:123/callback"} {
		if e := callbackURI(u); e != nil {
			t.Error(e)
		}
	}
}
