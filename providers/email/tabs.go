package main

import (
	"bufio"
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net"
	"strconv"
	"strings"
	"time"

	imap "github.com/emersion/go-imap/v2"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

// Gmail's IMAP INBOX holds every inbox tab, while Gmail itself shows only
// Primary there. With tabs on, INBOX is Primary and each other tab is a
// virtual mailbox over INBOX. The tabs come from X-GM-RAW searches, which
// go-imap cannot send, so they run on a short connection of their own.
var gmailTabs = []struct{ id, query string }{
	{"INBOX/Promotions", "category:promotions"},
	{"INBOX/Social", "category:social"},
	{"INBOX/Updates", "category:updates"},
	{"INBOX/Forums", "category:forums"},
}

// tabAttribute marks the virtual tab mailboxes for clients.
const tabAttribute = "$GmailTab"

// tabState is the INBOX state a tab listing was taken at.
type tabState struct {
	validity, messages uint32
	next               imap.UID
	modseq             uint64
}

type inboxTabs struct {
	state tabState
	at    time.Time
	// tab is each UID's tab; Primary mail has none.
	tab map[imap.UID]string
}

// tabsOn reports whether INBOX is split into tabs; only Gmail sets gmailTabs.
func (p *provider) tabsOn() bool { return p.cfg.gmailTabs }

// mailbox returns the IMAP mailbox behind a channel and, for INBOX views,
// the tab it shows: "" for every message, "INBOX" for Primary.
func (p *provider) mailbox(channel string) (string, string) {
	if !p.tabsOn() {
		return channel, ""
	}
	if channel == "INBOX" {
		return "INBOX", "INBOX"
	}
	for _, t := range gmailTabs {
		if t.id == channel {
			return "INBOX", t.id
		}
	}
	return channel, ""
}

func tabChannels() []qmsg.Channel {
	channels := []qmsg.Channel{}
	for _, t := range gmailTabs {
		d := "/"
		channels = append(channels, qmsg.Channel{ID: t.id, Name: t.id, Kind: qmsg.ChannelKind{Tag: qmsg.ChannelCustom, Custom: "mailbox"}, Mailbox: &qmsg.Mailbox{Attributes: []string{"\\HasNoChildren", tabAttribute}, Delimiter: &d}})
	}
	return channels
}

// inTab keeps the UIDs of INBOX that belong to tab.
func (p *provider) inTab(ctx context.Context, s *imap.SelectData, modseq uint64, tab string, uids []imap.UID) ([]imap.UID, error) {
	if tab == "" {
		return uids, nil
	}
	tabs, err := p.tabs(ctx, s, modseq)
	if err != nil {
		return nil, err
	}
	if tab == "INBOX" {
		tab = ""
	}
	out := []imap.UID{}
	for _, uid := range uids {
		if tabs[uid] == tab {
			out = append(out, uid)
		}
	}
	return out, nil
}

// tabs sorts INBOX into tabs, reusing the last listing while INBOX is
// unchanged. modseq is INBOX's HIGHESTMODSEQ from STATUS when SELECT has
// none. Without either, a tab move changes nothing we can see, so the
// listing then also expires after a minute.
func (p *provider) tabs(ctx context.Context, s *imap.SelectData, modseq uint64) (map[imap.UID]string, error) {
	state := tabState{validity: s.UIDValidity, messages: s.NumMessages, next: s.UIDNext, modseq: s.HighestModSeq}
	if state.modseq == 0 {
		state.modseq = modseq
	}
	if c := p.inboxTabs; c != nil && c.state == state && (state.modseq != 0 || time.Since(c.at) < time.Minute) {
		return c.tab, nil
	}
	search := p.searchTabs
	if search == nil {
		search = p.gmailSearch
	}
	validity, found, err := search(ctx)
	if err != nil {
		return nil, err
	}
	if validity != s.UIDValidity {
		return nil, fmt.Errorf("Inbox changed while loading tabs; try again")
	}
	tab := map[imap.UID]string{}
	for _, t := range gmailTabs {
		for _, uid := range found[t.id] {
			tab[uid] = t.id
		}
	}
	p.inboxTabs = &inboxTabs{state: state, at: time.Now(), tab: tab}
	return tab, nil
}

// tabSession is a connection kept for tab searches, since signing in to
// Gmail on a new connection can take many seconds.
type tabSession struct {
	conn  net.Conn
	r     *bufio.Reader
	n     int
	token string
}

func (p *provider) closeTabs() {
	if p.tabSession != nil {
		p.tabSession.conn.Close()
		p.tabSession = nil
	}
}

// gmailSearch lists each tab's INBOX UIDs, reusing the last connection
// while its token is current and it still answers.
func (p *provider) gmailSearch(ctx context.Context) (uint32, map[string][]imap.UID, error) {
	if err := p.refresh(ctx); err != nil {
		p.closeTabs()
		return 0, nil, err
	}
	if s := p.tabSession; s != nil && s.token != p.cred.AccessToken {
		p.closeTabs()
	}
	reused := p.tabSession != nil
	if !reused {
		conn, err := p.cfg.connect(ctx, p.cfg.imap, true)
		if err != nil {
			return 0, nil, err
		}
		s := &tabSession{conn: conn, r: bufio.NewReaderSize(conn, 64<<10), token: p.cred.AccessToken}
		stop := context.AfterFunc(ctx, func() { conn.Close() })
		conn.SetDeadline(time.Now().Add(45 * time.Second))
		err = s.login(p.cfg.address)
		stop()
		if err != nil {
			conn.Close()
			return 0, nil, err
		}
		p.tabSession = s
	}
	s := p.tabSession
	stop := context.AfterFunc(ctx, func() { s.conn.Close() })
	s.conn.SetDeadline(time.Now().Add(45 * time.Second))
	validity, found, err := s.search()
	stop()
	if err != nil {
		p.closeTabs()
		var ie *imap.Error
		if reused && ctx.Err() == nil && !errors.As(err, &ie) {
			// The server may have dropped an idle connection.
			return p.gmailSearch(ctx)
		}
		return 0, nil, err
	}
	s.conn.SetDeadline(time.Time{})
	return validity, found, nil
}

// run sends one command and reads its responses, passing untagged ones to
// untagged. This is just enough IMAP for XOAUTH2, EXAMINE and SEARCH.
func (s *tabSession) run(command string, untagged func(string) error) error {
	s.n++
	tag := "t" + strconv.Itoa(s.n)
	if _, err := io.WriteString(s.conn, tag+" "+command+"\r\n"); err != nil {
		return err
	}
	for {
		line, err := readLine(s.r)
		if err != nil {
			return err
		}
		switch {
		case strings.HasPrefix(line, "+"):
			// XOAUTH2 sends its error as a challenge; an empty reply
			// gets the tagged failure.
			if _, err := io.WriteString(s.conn, "\r\n"); err != nil {
				return err
			}
		case strings.HasPrefix(line, "* "):
			if untagged != nil {
				if err := untagged(line[2:]); err != nil {
					return err
				}
			}
		case strings.HasPrefix(line, tag+" "):
			status, text, _ := strings.Cut(line[len(tag)+1:], " ")
			if !strings.EqualFold(status, "OK") {
				return &imap.Error{Type: imap.StatusResponseType(strings.ToUpper(status)), Text: text}
			}
			return nil
		}
	}
}

func (s *tabSession) login(user string) error {
	if _, err := readLine(s.r); err != nil {
		return err
	}
	auth := base64.StdEncoding.EncodeToString([]byte("user=" + user + "\x01auth=Bearer " + s.token + "\x01\x01"))
	if err := s.run("AUTHENTICATE XOAUTH2 "+auth, nil); err != nil {
		var ie *imap.Error
		if errors.As(err, &ie) {
			return qmsg.ErrLoginRequired{}
		}
		return err
	}
	return nil
}

// search examines INBOX again, so its UIDVALIDITY is current, and runs one
// X-GM-RAW search per tab.
func (s *tabSession) search() (uint32, map[string][]imap.UID, error) {
	var validity uint32
	err := s.run("EXAMINE INBOX", func(line string) error {
		if _, rest, ok := strings.Cut(line, "[UIDVALIDITY "); ok {
			v, _, _ := strings.Cut(rest, "]")
			n, err := strconv.ParseUint(v, 10, 32)
			if err != nil {
				return fmt.Errorf("invalid UIDVALIDITY")
			}
			validity = uint32(n)
		}
		return nil
	})
	if err != nil {
		return 0, nil, err
	}
	found := map[string][]imap.UID{}
	for _, t := range gmailTabs {
		err := s.run(`UID SEARCH X-GM-RAW "`+t.query+`"`, func(line string) error {
			fields := strings.Fields(line)
			if len(fields) == 0 || !strings.EqualFold(fields[0], "SEARCH") {
				return nil
			}
			for _, f := range fields[1:] {
				uid, err := strconv.ParseUint(f, 10, 32)
				if err != nil || uid == 0 {
					return fmt.Errorf("invalid SEARCH response")
				}
				if len(found[t.id]) >= maxMailbox {
					return fmt.Errorf("mailbox exceeds UID limit")
				}
				found[t.id] = append(found[t.id], imap.UID(uid))
			}
			return nil
		})
		if err != nil {
			return 0, nil, err
		}
	}
	return validity, found, nil
}

// readLine reads one CRLF line of at most 2 MiB, enough for a search
// result at the mailbox limit. These responses carry no literals.
func readLine(r *bufio.Reader) (string, error) {
	var b strings.Builder
	for {
		chunk, err := r.ReadSlice('\n')
		if b.Len()+len(chunk) > 2<<20 {
			return "", fmt.Errorf("IMAP response line too long")
		}
		b.Write(chunk)
		if err == bufio.ErrBufferFull {
			continue
		}
		if err != nil {
			return "", err
		}
		line := strings.TrimRight(b.String(), "\r\n")
		if strings.HasSuffix(line, "}") {
			return "", fmt.Errorf("unexpected IMAP literal")
		}
		return line, nil
	}
}

// isTab reports whether a channel is a virtual tab mailbox.
func (p *provider) isTab(channel string) bool {
	mailbox, tab := p.mailbox(channel)
	return tab != "" && mailbox != channel
}
