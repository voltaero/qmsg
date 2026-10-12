package main

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"strings"
	"testing"

	imap "github.com/emersion/go-imap/v2"
	"github.com/voltaero/qmsg/providers/email/internal/fakemail"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

// gmailTabsProvider is a Gmail account on the fixture server whose tab
// search reports promo as Promotions mail.
func gmailTabsProvider(t *testing.T) (*provider, *fakemail.Server, map[imap.UID]bool) {
	t.Helper()
	p, s := testProvider(t)
	p.cfg.gmailTabs = true
	promo := map[imap.UID]bool{}
	p.searchTabs = func(ctx context.Context) (uint32, map[string][]imap.UID, error) {
		c, _, err := p.openIMAP(ctx)
		if err != nil {
			return 0, nil, err
		}
		defer c.Close()
		selected, err := c.Select("INBOX", &imap.SelectOptions{ReadOnly: true}).Wait()
		if err != nil {
			return 0, nil, err
		}
		found := map[string][]imap.UID{}
		all, err := c.UIDSearch(&imap.SearchCriteria{}, nil).Wait()
		if err != nil {
			return 0, nil, err
		}
		for _, uid := range all.AllUIDs() {
			if !promo[uid] {
				found["INBOX"] = append(found["INBOX"], uid)
			}
		}

		for uid := range promo {
			found["INBOX/Promotions"] = append(found["INBOX/Promotions"], uid)
		}
		return selected.UIDValidity, found, nil
	}
	if _, err := p.sync(context.Background()); err != nil {
		t.Fatal(err)
	}
	return p, s, promo
}

func subjects(page qmsg.HistoryPage) string {
	out := []string{}
	for _, m := range page.Messages {
		out = append(out, m.Email.Subject)
	}
	return strings.Join(out, ",")
}

func TestGmailInboxShowsPrimaryAndTabsShowTheRest(t *testing.T) {
	p, s, promo := gmailTabsProvider(t)
	ctx := context.Background()
	for _, subject := range []string{"Hello", "Vote now"} {
		if err := s.Append("INBOX", []byte("From: bob@example.test\r\nSubject: "+subject+"\r\n\r\nBody")); err != nil {
			t.Fatal(err)
		}
	}
	promo[2] = true
	if _, ok := p.folders["INBOX/Promotions"]; !ok {
		t.Fatal("Gmail tabs are not listed")
	}
	page, err := p.history(ctx, inbox(), nil, 20)
	if err != nil {
		t.Fatal(err)
	}
	if got := subjects(page); got != "Hello" {
		t.Fatalf("Inbox shows %q, want only Primary mail", got)
	}
	promotions := qmsg.ChannelRef{Organization: qmsg.Str(fakemail.Address), Channel: "INBOX/Promotions"}
	page, err = p.history(ctx, promotions, nil, 20)
	if err != nil {
		t.Fatal(err)
	}
	if got := subjects(page); got != "Vote now" {
		t.Fatalf("Promotions shows %q", got)
	}
	// Tab mail is INBOX mail: its actions select INBOX.
	if err = p.flags(ctx, qmsg.InMessage{Channel: promotions, ID: page.Messages[0].ID}, imap.FlagSeen, true); err != nil {
		t.Fatal(err)
	}
	if err = p.move(ctx, qmsg.InMessage{Channel: inbox(), ID: page.Messages[0].ID}, "INBOX/Social"); err == nil {
		t.Fatal("moved mail into a virtual tab")
	}
	if err = p.move(ctx, qmsg.InMessage{Channel: promotions, ID: page.Messages[0].ID}, "INBOX"); err == nil {
		t.Fatal("moved Promotions mail into its own mailbox")
	}

	// New mail changes every tab, and mail moved between tabs leaves Primary.
	promo[1] = true
	if err = s.Append("INBOX", []byte("From: bob@example.test\r\nSubject: Later\r\n\r\nBody")); err != nil {
		t.Fatal(err)
	}
	changed, err := p.sync(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Join(changed, ","); got != "INBOX,INBOX/Forums,INBOX/Promotions,INBOX/Social,INBOX/Updates" {
		t.Fatalf("changed: %s", got)
	}
	if len(p.loadedMail["INBOX"]) != 0 {
		t.Fatal("mail moved to Promotions stayed in Primary")
	}
	page, err = p.history(ctx, inbox(), nil, 20)
	if err != nil {
		t.Fatal(err)
	}
	if got := subjects(page); got != "Later" {
		t.Fatalf("Inbox shows %q", got)
	}
}

func TestGmailTabsCanBeTurnedOff(t *testing.T) {
	p, s, promo := gmailTabsProvider(t)
	p.cfg.gmailTabs = false
	if err := s.Append("INBOX", []byte("From: bob@example.test\r\nSubject: Vote now\r\n\r\nBody")); err != nil {
		t.Fatal(err)
	}
	promo[1] = true
	if _, err := p.sync(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, ok := p.folders["INBOX/Promotions"]; ok {
		t.Fatal("tabs listed while turned off")
	}
	page, err := p.history(context.Background(), inbox(), nil, 20)
	if err != nil || subjects(page) != "Vote now" {
		t.Fatalf("Inbox without tabs: %q %v", subjects(page), err)
	}
	if _, err := configuration(map[string]string{"service": "gmail", "address": "a@gmail.com", "gmail_tabs": "maybe"}); err == nil {
		t.Fatal("invalid gmail_tabs accepted")
	}
}

func TestSearchTabsSpeaksGmailIMAP(t *testing.T) {
	client, server := net.Pipe()
	defer client.Close()
	commands := make(chan []string, 1)
	go func() {
		defer server.Close()
		r := bufio.NewReader(server)
		seen := []string{}
		fmt.Fprint(server, "* OK Gimap ready\r\n")
		for {
			line, err := r.ReadString('\n')
			if err != nil {
				commands <- seen
				return
			}
			tag, command, _ := strings.Cut(strings.TrimRight(line, "\r\n"), " ")
			seen = append(seen, command)
			switch {
			case strings.HasPrefix(command, "AUTHENTICATE"):
				fmt.Fprintf(server, "* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\n%s OK alice authenticated\r\n", tag)
			case command == "EXAMINE INBOX":
				fmt.Fprintf(server, "* FLAGS (\\Seen)\r\n* OK [UIDVALIDITY 7] UIDs valid.\r\n* 4 EXISTS\r\n%s OK [READ-ONLY] INBOX selected.\r\n", tag)
			case strings.Contains(command, "category:promotions"):
				fmt.Fprintf(server, "* SEARCH 3 9\r\n%s OK SEARCH completed\r\n", tag)
			case strings.HasPrefix(command, "UID SEARCH"):
				fmt.Fprintf(server, "* SEARCH\r\n%s OK SEARCH completed\r\n", tag)
			}
		}
	}()
	session := &tabSession{conn: client, r: bufio.NewReader(client), token: "token"}
	if err := session.login("alice@gmail.com"); err != nil {
		t.Fatal(err)
	}
	validity, found, err := session.search()
	if err != nil {
		t.Fatal(err)
	}
	// A reused connection examines INBOX again.
	if _, _, err = session.search(); err != nil {
		t.Fatal(err)
	}
	client.Close()
	if validity != 7 || fmt.Sprint(found) != "map[INBOX/Promotions:[3 9]]" {
		t.Fatalf("got %d %v", validity, found)
	}
	seen := <-commands
	if len(seen) != 13 || seen[2] != `UID SEARCH X-GM-RAW "category:primary"` || seen[3] != `UID SEARCH X-GM-RAW "category:promotions"` || seen[7] != "EXAMINE INBOX" {
		t.Fatalf("commands: %q", seen)
	}
}

func TestSearchTabsReportsRejectedTokens(t *testing.T) {
	client, server := net.Pipe()
	defer client.Close()
	go func() {
		defer server.Close()
		r := bufio.NewReader(server)
		fmt.Fprint(server, "* OK Gimap ready\r\n")
		r.ReadString('\n')
		fmt.Fprint(server, "+ eyJzdGF0dXMiOiI0MDAifQ==\r\n")
		r.ReadString('\n')
		fmt.Fprint(server, "t1 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)\r\n")
	}()
	session := &tabSession{conn: client, r: bufio.NewReader(client), token: "expired"}
	if err := session.login("alice@gmail.com"); !authenticationError(err) {
		t.Fatalf("got %v, want a login request", err)
	}
}

func TestPrimaryCanOverlapAnotherGmailCategory(t *testing.T) {
	p, _, _ := gmailTabsProvider(t)
	p.inboxTabs = nil
	p.searchTabs = func(context.Context) (uint32, map[string][]imap.UID, error) {
		return 7, map[string][]imap.UID{"INBOX": {2}, "INBOX/Updates": {2}, "INBOX/Promotions": {3}}, nil
	}
	got, err := p.inTab(context.Background(), &imap.SelectData{UIDValidity: 7}, 0, "INBOX", []imap.UID{1, 2, 3})
	if err != nil || fmt.Sprint(got) != "[2]" {
		t.Fatalf("Primary UIDs = %v, %v; want Spotify UID 2 only", got, err)
	}
}
