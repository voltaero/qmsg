package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"strings"
	"testing"
	"time"

	imap "github.com/emersion/go-imap/v2"
	smtp "github.com/emersion/go-smtp"
	"github.com/voltaero/qmsg/providers/email/internal/fakemail"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

func testProvider(t *testing.T) (*provider, *fakemail.Server) {
	t.Helper()
	s, e := fakemail.Start()
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(s.Close)
	cfg, e := configuration(map[string]string{"address": fakemail.Address, "imap_addr": "imap.example.test:993", "smtp_addr": "smtp.example.test:465", "ca_pem": s.CA, "test_dial": "imap.example.test=" + s.IMAP + ",smtp.example.test=" + s.SMTP, "sent_mailbox": "Sent"})
	if e != nil {
		t.Fatal(e)
	}
	p := &provider{cfg: cfg, cred: credential{Password: fakemail.Password}, folders: map[string]qmsg.Channel{}, known: map[string]qmsg.EmailAddress{}}
	if e = p.list(context.Background()); e != nil {
		t.Fatal(e)
	}
	p.ready = true
	return p, s
}
func inbox() qmsg.ChannelRef {
	return qmsg.ChannelRef{Organization: qmsg.Str(fakemail.Address), Channel: "INBOX"}
}

func TestInboxListsLargeMailWithoutDownloadingBodies(t *testing.T) {
	p, s := testProvider(t)
	raw := []byte("From: bob@example.test\r\nTo: alice@example.test\r\nSubject: Big mail\r\n\r\n" + strings.Repeat("x", maxMail+1))
	if e := s.Append("INBOX", raw); e != nil {
		t.Fatal(e)
	}
	if e := s.User.Create("All Mail", nil); e != nil {
		t.Fatal(e)
	}
	if e := s.Append("All Mail", []byte("From: bob@example.test\r\nSubject: Archived only\r\n\r\nArchived")); e != nil {
		t.Fatal(e)
	}
	if e := p.list(context.Background()); e != nil {
		t.Fatal(e)
	}
	page, e := p.history(context.Background(), inbox(), nil, 20)
	if e != nil {
		t.Fatal(e)
	}
	if len(page.Messages) != 1 || page.Messages[0].Email.Subject != "Big mail" {
		t.Fatalf("wrong inbox: %v", page.Messages)
	}
	if mailSize(page.Messages[0]) > 4096 {
		t.Fatal("folder listing fetched the body")
	}
}

func TestInboxPageUsesBulkMetadataRequests(t *testing.T) {
	p, s := testProvider(t)
	for i := 0; i < 20; i++ {
		if e := s.Append("INBOX", []byte("From: bob@example.test\r\nSubject: Mail\r\n\r\nBody")); e != nil {
			t.Fatal(e)
		}
	}
	page, e := p.history(context.Background(), inbox(), nil, 20)
	if e != nil {
		t.Fatal(e)
	}
	if len(page.Messages) != 20 {
		t.Fatal("page lost messages")
	}
	if n := s.Fetches.Load(); n > 2 {
		t.Fatalf("listing fetched messages one by one: %d requests", n)
	}
	if s.FullBodyFetches.Load() != 0 {
		t.Fatal("listing downloaded message bodies")
	}
}

type testMailLiteral struct{ *bytes.Reader }

func (r testMailLiteral) Size() int64 { return int64(r.Len()) }

func TestInboxOrdersByArrivalRatherThanUID(t *testing.T) {
	p, s := testProvider(t)
	for i, age := range []time.Duration{0, -time.Hour} {
		raw := []byte(fmt.Sprintf("From: bob@example.test\r\nSubject: Mail %d\r\n\r\nBody", i))
		if _, e := s.User.Append("INBOX", testMailLiteral{bytes.NewReader(raw)}, &imap.AppendOptions{Time: time.Now().Add(age)}); e != nil {
			t.Fatal(e)
		}
	}
	page, e := p.history(context.Background(), inbox(), nil, 1)
	if e != nil {
		t.Fatal(e)
	}
	if page.Messages[0].Email.Subject != "Mail 0" {
		t.Fatal("newest arrival is missing from the first page")
	}
	older, e := p.history(context.Background(), inbox(), page.NextCursor, 1)
	if e != nil {
		t.Fatal(e)
	}
	if len(older.Messages) != 1 || older.Messages[0].Email.Subject != "Mail 1" {
		t.Fatal("date-ordered pagination missed a mail")
	}
}
func TestMailServerRoundTrip(t *testing.T) {
	p, s := testProvider(t)
	ctx := context.Background()
	if len(p.folders) != 3 {
		t.Fatalf("folders: %v", p.folders)
	}
	for i := 0; i < 6; i++ {
		c := compose{to: []string{fakemail.Address}, bcc: []string{"hidden@example.test"}, subject: "Hello 😀", html: "<p>Hello <img src=\"https://example.test/image\"></p>", content: []qmsg.Content{qmsg.ContentText{Text: "Hello plain"}, qmsg.ContentFile{Media: qmsg.Media{Name: qmsg.Str("a.txt"), Mime: qmsg.Str("text/plain"), Source: qmsg.SourceBytes{Bytes: []byte("attachment")}}}}}
		r, e := p.send(ctx, c)
		if e != nil {
			t.Fatal(e)
		}
		if r["accepted"] != qmsg.Boolean(true) || r["sent_copy_saved"] != qmsg.Boolean(true) {
			t.Fatalf("send result: %v", r)
		}
	}
	mails, rs := s.Snapshot()
	if len(mails) != 6 || len(rs[0]) != 2 {
		t.Fatalf("SMTP envelopes: %v", rs)
	}
	if bytes.Contains(bytes.ToLower(mails[0]), []byte("bcc:")) {
		t.Fatal("Bcc leaked into headers")
	}
	page, e := p.history(ctx, inbox(), nil, 5)
	if e != nil {
		t.Fatal(e)
	}
	if len(page.Messages) != 5 || page.NextCursor == nil {
		t.Fatalf("page: %v", page)
	}
	m, e := p.get(ctx, inbox(), page.Messages[0].ID)
	if e != nil {
		t.Fatal(e)
	}
	if m.Email.Subject != "Hello 😀" || m.Email.HTML == nil || m.Email.Read || len(m.Content) != 2 {
		t.Fatalf("MIME/state: %#v", m)
	}
	if string(m.Content[1].(qmsg.ContentFile).Media.Source.(qmsg.SourceBytes).Bytes) != "attachment" {
		t.Fatal("attachment changed")
	}
	next, e := p.history(ctx, inbox(), page.NextCursor, 5)
	if e != nil || len(next.Messages) != 1 || next.NextCursor != nil {
		t.Fatalf("next page: %v %v", next, e)
	}
	fc := qmsg.InMessage{Channel: inbox(), ID: m.ID}
	if e = p.flags(ctx, fc, imap.FlagSeen, true); e != nil {
		t.Fatal(e)
	}
	if e = p.flags(ctx, fc, imap.FlagFlagged, true); e != nil {
		t.Fatal(e)
	}
	got, e := p.get(ctx, inbox(), m.ID)
	if e != nil || !got.Email.Read || !got.Email.Starred {
		t.Fatalf("flags: %v %v", got.Email, e)
	}
	if e = p.flags(ctx, fc, imap.FlagSeen, false); e != nil {
		t.Fatal(e)
	}
	if e = p.move(ctx, fc, "Trash"); e != nil {
		t.Fatal(e)
	}
	if _, e = p.get(ctx, inbox(), m.ID); e == nil {
		t.Fatal("moved mail still in inbox")
	}
	trash := inbox()
	trash.Channel = "Trash"
	tp, e := p.history(ctx, trash, nil, 5)
	if e != nil || len(tp.Messages) != 1 || tp.Messages[0].Email.Read {
		t.Fatalf("move: %v %v", tp, e)
	}
	if len(p.find("alice")) != 1 {
		t.Fatalf("address lookup: %v", p.known)
	}
	// Deleted/recreated folder reuses UID 1 but must not retarget an old id.
	old := next.Messages[0].ID
	if e = s.User.Delete("INBOX"); e != nil {
		t.Fatal(e)
	}
	if e = s.User.Create("INBOX", nil); e != nil {
		t.Fatal(e)
	}
	if e = s.Append("INBOX", mails[0]); e != nil {
		t.Fatal(e)
	}
	if _, e = p.get(ctx, inbox(), old); e == nil {
		t.Fatal("stale UIDVALIDITY read a different message")
	}
}
func TestSMTPStartTLSAndPartialResult(t *testing.T) {
	p, s := testProvider(t)
	ctx := context.Background()
	p.cfg.smtpTLS = "starttls"
	p.cfg.redirects["smtp.example.test"] = s.StartTLS
	s.DenySent.Store(true)
	r, e := p.send(ctx, compose{to: []string{fakemail.Address}, subject: "accepted", content: []qmsg.Content{qmsg.ContentText{Text: "Body"}}})
	if e != nil {
		t.Fatal(e)
	}
	if r["accepted"] != qmsg.Boolean(true) || r["sent_copy_saved"] != qmsg.Boolean(false) || r["warning"] == qmsg.Text("") {
		t.Fatalf("partial result: %v", r)
	}
	m, _ := s.Snapshot()
	if len(m) != 1 {
		t.Fatal("message was duplicated")
	}
}
func TestSMTPUnknownDelivery(t *testing.T) {
	p, s := testProvider(t)
	s.DropDATA.Store(true)
	_, e := p.send(context.Background(), compose{to: []string{fakemail.Address}, subject: "unknown", content: []qmsg.Content{qmsg.ContentText{Text: "Body"}}})
	var pe qmsg.ErrProvider
	if !errors.As(e, &pe) || pe.Code != "delivery_unknown" {
		t.Fatalf("expected unknown delivery, got %v", e)
	}
	m, _ := s.Snapshot()
	if len(m) != 1 {
		t.Fatal("fixture did not accept the mail")
	}
}
func TestInputAndMIMELimits(t *testing.T) {
	for _, s := range []string{"alice@example.test\r\nBcc: evil@example.test", "not an address", "a\x00@example.test"} {
		if _, e := parseAddress(s); e == nil {
			t.Fatalf("accepted %q", s)
		}
	}
	_, _, _, e := composeMail(context.Background(), fakemail.Address, compose{to: []string{fakemail.Address}, subject: "hello\r\nBcc: evil@example.test"}, nil)
	if e == nil {
		t.Fatal("header injection accepted")
	}
	_, _, _, e = composeMail(context.Background(), fakemail.Address, compose{to: []string{fakemail.Address}, content: []qmsg.Content{qmsg.ContentText{Text: strings.Repeat("x", maxMail)}}}, nil)
	if e == nil {
		t.Fatal("oversized MIME accepted")
	}
	_, _, _, e = composeMail(context.Background(), fakemail.Address, compose{to: []string{fakemail.Address}, content: []qmsg.Content{qmsg.ContentFile{Media: qmsg.Media{Source: qmsg.SourceURL{URL: "https://example.test/private"}}}}}, nil)
	if e == nil {
		t.Fatal("URL attachment accepted")
	}
	if _, e = parseMail(make([]byte, maxMail+1), inbox(), "1:1", nil); e == nil {
		t.Fatal("oversized incoming mail accepted")
	}
	cfg, e := configuration(map[string]string{"service": "gmail", "address": fakemail.Address})
	if e != nil || cfg.imap != "imap.gmail.com:993" || cfg.smtp != "smtp.gmail.com:465" {
		t.Fatalf("Gmail defaults: %v %v", cfg, e)
	}
}
func TestCancelDial(t *testing.T) {
	p, _ := testProvider(t)
	l, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	defer l.Close()
	go func() {
		c, e := l.Accept()
		if e == nil {
			defer c.Close()
			time.Sleep(300 * time.Millisecond)
		}
	}()
	p.cfg.redirects["imap.example.test"] = l.Addr().String()
	ctx, stop := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer stop()
	start := time.Now()
	_, _, e = p.openIMAP(ctx)
	if e == nil || time.Since(start) > time.Second {
		t.Fatalf("cancel did not stop TLS connect: %v", e)
	}
}

func TestGmailOAuthRefreshAndSASL(t *testing.T) {
	p, s := testProvider(t)
	p.cfg.service = "gmail"
	p.cfg.imap = "imap.gmail.com:993"
	p.cfg.smtp = "smtp.gmail.com:465"
	p.cfg.redirects["imap.gmail.com"] = s.IMAP
	p.cfg.redirects["smtp.gmail.com"] = s.SMTP
	p.cfg.redirects["oauth2.googleapis.com"] = s.OAuth
	p.cred = credential{RefreshToken: "fixture-refresh", ClientID: "fixture-client"}
	ctx := context.Background()
	if e := p.list(ctx); e != nil {
		t.Fatal(e)
	}
	if s.Refreshes.Load() != 1 || p.cred.AccessToken != "fixture-access" {
		t.Fatal("Gmail did not refresh the expired token")
	}
	r, e := p.send(ctx, compose{to: []string{fakemail.Address}, subject: "OAuth mail", content: []qmsg.Content{qmsg.ContentText{Text: "Body"}}})
	if e != nil || r["accepted"] != qmsg.Boolean(true) {
		t.Fatalf("XOAUTH2 SMTP: %v %v", r, e)
	}
	if s.Refreshes.Load() != 1 {
		t.Fatal("valid token refreshed needlessly")
	}
	p.cred.ExpiresAt = 0
	s.RejectRefresh.Store(true)
	if e = p.list(ctx); e == nil {
		t.Fatal("revoked refresh token accepted")
	}
}

func TestMIMENestingCIDAndInvalidText(t *testing.T) {
	raw := "From: bob@example.test\r\nTo: alice@example.test\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:pic\">\r\n--x\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=picture.png\r\nContent-ID: <pic>\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n--x--\r\n"
	m, e := parseMail([]byte(raw), inbox(), "1:1", nil)
	if e != nil {
		t.Fatal(e)
	}
	if len(m.Content) != 1 || m.Email.ContentIDs["0"] != "pic" || m.Email.HTML == nil {
		t.Fatalf("CID association: %#v", m)
	}
	if m.Content[0].(qmsg.ContentImage).Media.Name == nil || *m.Content[0].(qmsg.ContentImage).Media.Name != "picture.png" {
		t.Fatal("inline image name lost")
	}
	body := "Content-Type: text/plain\r\n\r\nBody"
	for i := 0; i < 32; i++ {
		boundary := strings.Repeat("b", i+1)
		body = "Content-Type: multipart/mixed; boundary=" + boundary + "\r\n\r\n--" + boundary + "\r\n" + body + "\r\n--" + boundary + "--\r\n"
	}
	if _, e = parseMail([]byte(body), inbox(), "1:1", nil); e == nil {
		t.Fatal("excess MIME nesting accepted")
	}
	m, e = parseMail([]byte("From: bob@example.test\r\nContent-Type: text/plain\r\n\r\ninvalid \xff"), inbox(), "1:1", nil)
	if e != nil {
		t.Fatal(e)
	}
	if !strings.Contains(m.Content[0].(qmsg.ContentText).Text, "�") {
		t.Fatal("invalid UTF-8 passed to protocol")
	}
}

func TestDefiniteSMTPFailure(t *testing.T) {
	p, _ := testProvider(t)
	p.cred.Password = "wrong"
	_, e := p.send(context.Background(), compose{to: []string{fakemail.Address}, subject: "rejected", content: []qmsg.Content{qmsg.ContentText{Text: "Body"}}})
	var login qmsg.ErrLoginRequired
	if !errors.As(e, &login) {
		t.Fatalf("bad SMTP login: %v", e)
	}
	err := definiteSMTPError(&smtp.SMTPError{Code: 550, Message: "Recipient rejected"})
	pe := err.(qmsg.ErrProvider)
	if pe.Details.(qmsg.Record)["delivery_unknown"] != qmsg.Boolean(false) {
		t.Fatal("definite SMTP rejection became unknown")
	}
}

// Polls must only invalidate folders that changed, so clients do not reload
// every mailbox each minute.
func TestSyncReportsOnlyChangedFolders(t *testing.T) {
	p, s := testProvider(t)
	ctx := context.Background()
	sync := func(want ...string) {
		t.Helper()
		changed, e := p.sync(ctx)
		if e != nil {
			t.Fatal(e)
		}
		if strings.Join(changed, ",") != strings.Join(want, ",") {
			t.Fatalf("changed folders: got %v, want %v", changed, want)
		}
	}
	sync()
	raw := []byte("From: bob@example.test\r\nTo: alice@example.test\r\nSubject: hi\r\n\r\nBody\r\n")
	for i := 0; i < 2; i++ {
		if e := s.Append("INBOX", raw); e != nil {
			t.Fatal(e)
		}
	}
	sync("INBOX")
	sync()

	// Another client reads, then expunges a message.
	c, _, e := p.openIMAP(ctx)
	if e != nil {
		t.Fatal(e)
	}
	defer c.Close()
	if _, e = c.Select("INBOX", nil).Wait(); e != nil {
		t.Fatal(e)
	}
	if e = c.Store(imap.UIDSetNum(1), &imap.StoreFlags{Op: imap.StoreFlagsAdd, Silent: true, Flags: []imap.Flag{imap.FlagSeen}}, nil).Close(); e != nil {
		t.Fatal(e)
	}
	sync("INBOX")
	if e = c.Store(imap.UIDSetNum(1), &imap.StoreFlags{Op: imap.StoreFlagsAdd, Silent: true, Flags: []imap.Flag{imap.FlagDeleted}}, nil).Close(); e != nil {
		t.Fatal(e)
	}
	if e = c.UIDExpunge(imap.UIDSetNum(1)).Close(); e != nil {
		t.Fatal(e)
	}
	sync("INBOX")

	// A failed poll keeps the baseline: what changed meanwhile is reported
	// once the server is reachable again.
	if e = s.Append("Trash", raw); e != nil {
		t.Fatal(e)
	}
	l, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	l.Close()
	p.cfg.redirects["imap.example.test"] = l.Addr().String()
	if _, e = p.sync(ctx); e == nil {
		t.Fatal("sync succeeded without a server")
	}
	p.cfg.redirects["imap.example.test"] = s.IMAP
	sync("Trash")

	// New folders are reported; a recreated one is too, by its UIDVALIDITY.
	if e = s.User.Create("Archive", nil); e != nil {
		t.Fatal(e)
	}
	sync("Archive")
	if e = s.User.Delete("Archive"); e != nil {
		t.Fatal(e)
	}
	if e = s.User.Create("Archive", nil); e != nil {
		t.Fatal(e)
	}
	sync("Archive")
	sync()
}

func TestChangedFoldersCompareModSeq(t *testing.T) {
	old := map[string]folderState{"INBOX": {validity: 1, next: 5, messages: 4, modseq: 10}, "Sent": {validity: 1, next: 2, messages: 1, modseq: 3}}
	if changedFolders(nil, old) != nil {
		t.Fatal("the first sync reported changes")
	}
	now := map[string]folderState{"INBOX": old["INBOX"], "Sent": old["Sent"]}
	starred := now["INBOX"]
	starred.modseq++
	now["INBOX"] = starred
	if got := changedFolders(old, now); len(got) != 1 || got[0] != "INBOX" {
		t.Fatalf("flag-only change: %v", got)
	}
}

func TestSyncRemovesLoadedMailArchivedElsewhere(t *testing.T) {
	p, s := testProvider(t)
	ctx := context.Background()
	for i := 0; i < 3; i++ {
		if e := s.Append("INBOX", []byte("From: bob@example.test\r\nSubject: Mail\r\n\r\nBody")); e != nil {
			t.Fatal(e)
		}
	}
	if _, e := p.sync(ctx); e != nil {
		t.Fatal(e)
	}
	page, e := p.history(ctx, inbox(), nil, 20)
	if e != nil {
		t.Fatal(e)
	}
	old := page.Messages[len(page.Messages)-1].ID
	if !p.loadedMail["INBOX"][old] {
		t.Fatal("loaded identity wasn't recorded")
	}
	c, _, e := p.openIMAP(ctx)
	if e != nil {
		t.Fatal(e)
	}
	defer c.Close()
	selected, e := c.Select("INBOX", nil).Wait()
	if e != nil {
		t.Fatal(e)
	}
	uid, e := parseID(old, selected.UIDValidity)
	if e != nil {
		t.Fatal(e)
	}
	if e = c.Store(imap.UIDSetNum(uid), &imap.StoreFlags{Op: imap.StoreFlagsAdd, Silent: true, Flags: []imap.Flag{imap.FlagDeleted}}, nil).Close(); e != nil {
		t.Fatal(e)
	}
	if e = c.UIDExpunge(imap.UIDSetNum(uid)).Close(); e != nil {
		t.Fatal(e)
	}
	if _, e = p.sync(ctx); e != nil {
		t.Fatal(e)
	}
	if p.loadedMail["INBOX"][old] {
		t.Fatal("archived mail remains loaded in Inbox")
	}
	if len(p.loadedMail["INBOX"]) != 2 {
		t.Fatal("unrelated Inbox mail was removed")
	}
}
