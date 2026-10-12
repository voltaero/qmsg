package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net"
	"net/http"
	"strings"
	"time"

	qmsg "github.com/voltaero/qmsg/sdk/go"
	_ "golang.org/x/crypto/x509roots/fallback"
)

type config struct {
	service, address, imap, smtp, smtpTLS string
	sentMailbox, trashMailbox             string
	clientID, clientSecret                string
	oauthRedirect                         string
	gmailTabs                             bool
	tls                                   *tls.Config
	redirects                             map[string]string
}

func configuration(s map[string]string) (config, error) {
	c := config{sentMailbox: s["sent_mailbox"], trashMailbox: s["trash_mailbox"], service: s["service"], address: s["address"], imap: s["imap_addr"], smtp: s["smtp_addr"], smtpTLS: s["smtp_tls"], tls: &tls.Config{MinVersion: tls.VersionTLS12}, redirects: map[string]string{}}
	if c.service == "gmail" {
		c.clientID, c.clientSecret = s["oauth_client_id"], s["oauth_client_secret"]
		c.oauthRedirect = s["oauth_redirect_uri"]
		switch s["gmail_tabs"] {
		case "", "true":
			c.gmailTabs = true
		case "false":
		default:
			return c, fmt.Errorf("gmail_tabs must be true or false")
		}
		c.imap = "imap.gmail.com:993"
		c.smtp = "smtp.gmail.com:465"
		c.smtpTLS = "tls"
	} else if c.service != "" && c.service != "imap" {
		return c, fmt.Errorf("service must be imap or gmail")
	}
	if a, err := parseAddress(c.address); err != nil {
		return c, fmt.Errorf("set address to a valid email address")
	} else {
		c.address = a.Address
	}
	if c.smtpTLS == "" {
		c.smtpTLS = "tls"
	}
	if c.smtpTLS != "tls" && c.smtpTLS != "starttls" {
		return c, fmt.Errorf("smtp_tls must be tls or starttls")
	}
	for _, a := range []string{c.imap, c.smtp} {
		if _, _, err := net.SplitHostPort(a); err != nil {
			return c, fmt.Errorf("set imap_addr and smtp_addr to host:port")
		}
	}
	if pem := s["ca_pem"]; pem != "" {
		pool, err := x509.SystemCertPool()
		if err != nil {
			pool = x509.NewCertPool()
		}
		if !pool.AppendCertsFromPEM([]byte(pem)) {
			return c, fmt.Errorf("invalid ca_pem")
		}
		c.tls.RootCAs = pool
	}
	for _, v := range strings.Split(s["test_dial"], ",") {
		if h, a, ok := strings.Cut(v, "="); ok {
			c.redirects[h] = a
		}
	}
	return c, nil
}
func (c config) tlsFor(addr string) *tls.Config {
	t := c.tls.Clone()
	t.ServerName, _, _ = net.SplitHostPort(addr)
	return t
}
func (c config) dial(ctx context.Context, network, addr string) (net.Conn, error) {
	h, _, _ := net.SplitHostPort(addr)
	if a := c.redirects[h]; a != "" {
		addr = a
	}
	return qmsg.Dial(ctx, network, addr)
}
func (c config) connect(ctx context.Context, addr string, secure bool) (net.Conn, error) {
	conn, err := c.dial(ctx, "tcp", addr)
	if err != nil {
		return nil, err
	}
	conn.SetDeadline(time.Now().Add(45 * time.Second))
	if secure {
		t := tls.Client(conn, c.tlsFor(addr))
		if err := t.HandshakeContext(ctx); err != nil {
			conn.Close()
			return nil, err
		}
		conn = t
	}
	return conn, nil
}
func (c config) httpClient() *http.Client {
	return &http.Client{Timeout: 30 * time.Second, Transport: &http.Transport{DialContext: c.dial, TLSClientConfig: c.tls.Clone(), TLSHandshakeTimeout: 15 * time.Second, DisableKeepAlives: true}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
}
