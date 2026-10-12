package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"time"

	imap "github.com/emersion/go-imap/v2"
	"github.com/emersion/go-imap/v2/imapclient"
	"github.com/emersion/go-message"
	gomail "github.com/emersion/go-message/mail"
	"github.com/emersion/go-message/textproto"
	"github.com/emersion/go-sasl"
	smtp "github.com/emersion/go-smtp"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

type credential struct {
	Password     string `json:"password,omitempty"`
	AccessToken  string `json:"access_token,omitempty"`
	RefreshToken string `json:"refresh_token,omitempty"`
	ClientID     string `json:"client_id,omitempty"`
	ClientSecret string `json:"client_secret,omitempty"`
	ExpiresAt    int64  `json:"expires_at,omitempty"`
}
type xoauth struct{ user, token string }

func (x xoauth) Start() (string, []byte, error) {
	return "XOAUTH2", []byte("user=" + x.user + "\x01auth=Bearer " + x.token + "\x01\x01"), nil
}
func (x xoauth) Next([]byte) ([]byte, error) { return []byte{}, nil }
func (p *provider) auth() sasl.Client {
	if p.cfg.service == "gmail" {
		return xoauth{p.cfg.address, p.cred.AccessToken}
	}
	return sasl.NewPlainClient("", p.cfg.address, p.cred.Password)
}
func (p *provider) refresh(ctx context.Context) error {
	if p.cfg.service != "gmail" {
		return nil
	}
	if p.cred.AccessToken != "" && p.cred.ExpiresAt > time.Now().Unix()+60 {
		return nil
	}
	if p.cred.RefreshToken == "" || p.cred.ClientID == "" {
		return qmsg.ErrLoginRequired{}
	}
	form := url.Values{"grant_type": {"refresh_token"}, "refresh_token": {p.cred.RefreshToken}, "client_id": {p.cred.ClientID}}
	if p.cred.ClientSecret != "" {
		form.Set("client_secret", p.cred.ClientSecret)
	}
	req, _ := http.NewRequestWithContext(ctx, "POST", "https://oauth2.googleapis.com/token", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	resp, e := p.cfg.httpClient().Do(req)
	if e != nil {
		return fmt.Errorf("OAuth refresh connection failed")
	}
	defer resp.Body.Close()
	body, e := io.ReadAll(io.LimitReader(resp.Body, 65537))
	if e != nil || len(body) > 65536 {
		return fmt.Errorf("invalid OAuth response")
	}
	var token struct {
		AccessToken  string `json:"access_token"`
		RefreshToken string `json:"refresh_token"`
		TokenType    string `json:"token_type"`
		ExpiresIn    int64  `json:"expires_in"`
		Error        string `json:"error"`
	}
	if json.Unmarshal(body, &token) != nil {
		return fmt.Errorf("invalid OAuth response")
	}
	if resp.StatusCode != 200 {
		if token.Error == "invalid_grant" {
			p.cred.AccessToken = ""
			return qmsg.ErrLoginRequired{}
		}
		return fmt.Errorf("OAuth refresh failed")
	}
	if token.AccessToken == "" || token.ExpiresIn <= 0 || token.ExpiresIn > 86400 || !strings.EqualFold(token.TokenType, "bearer") {
		return fmt.Errorf("invalid OAuth response")
	}
	p.cred.AccessToken = token.AccessToken
	if token.RefreshToken != "" {
		p.cred.RefreshToken = token.RefreshToken
	}
	p.cred.ExpiresAt = time.Now().Unix() + token.ExpiresIn
	if p.ready {
		return p.storeCredential()
	}
	return nil
}
func (p *provider) openIMAP(ctx context.Context) (*imapclient.Client, net.Conn, error) {
	if e := p.refresh(ctx); e != nil {
		return nil, nil, e
	}
	conn, e := p.cfg.connect(ctx, p.cfg.imap, true)
	if e != nil {
		return nil, nil, e
	}
	c := imapclient.New(conn, &imapclient.Options{})
	stop := context.AfterFunc(ctx, func() { conn.Close() })
	defer stop()
	if e = c.WaitGreeting(); e == nil {
		if p.cfg.service == "gmail" {
			e = c.Authenticate(p.auth())
		} else {
			e = c.Login(p.cfg.address, p.cred.Password).Wait()
		}
	}
	if e != nil {
		c.Close()
		if authenticationError(e) {
			return nil, nil, qmsg.ErrLoginRequired{}
		}
		return nil, nil, e
	}
	return c, conn, nil
}
func (p *provider) withMailbox(ctx context.Context, ref qmsg.ChannelRef, fn func(*imapclient.Client, *imap.SelectData) error) error {
	if !p.validRef(ref) {
		return qmsg.ErrUnknownChannel{Channel: ref}
	}
	c, conn, e := p.openIMAP(ctx)
	if e != nil {
		return e
	}
	defer c.Close()
	stop := context.AfterFunc(ctx, func() { conn.Close() })
	defer stop()
	selected, e := c.Select(ref.Channel, nil).Wait()
	if e != nil {
		return e
	}
	if selected.NumMessages > maxMailbox {
		return fmt.Errorf("mailbox exceeds 100000-message limit")
	}
	return fn(c, selected)
}
func (p *provider) list(ctx context.Context) error {
	_, e := p.sync(ctx)
	return e
}

// folderState is what STATUS reports about a mailbox. Any new, expunged or
// re-flagged message changes it: UIDNEXT and MESSAGES cover arrivals and
// deletions, UNSEEN covers read state, and HIGHESTMODSEQ (with CONDSTORE)
// covers every flag change. Without CONDSTORE, star changes made by other
// clients are not detected.
type folderState struct {
	validity, messages, unseen uint32
	next                       imap.UID
	modseq                     uint64
}

func statusState(d *imap.StatusData) folderState {
	s := folderState{validity: d.UIDValidity, next: d.UIDNext, modseq: d.HighestModSeq}
	if d.NumMessages != nil {
		s.messages = *d.NumMessages
	}
	if d.NumUnseen != nil {
		s.unseen = *d.NumUnseen
	}
	return s
}

// changedFolders lists mailboxes whose state differs from the previous sync,
// including ones that are new or whose STATUS failed last time. The first
// sync only records a baseline.
func changedFolders(old, now map[string]folderState) []string {
	if old == nil {
		return nil
	}
	changed := []string{}
	for name, s := range now {
		if prev, ok := old[name]; !ok || prev != s {
			changed = append(changed, name)
		}
	}
	sort.Strings(changed)
	return changed
}

// sync lists folders, publishes the directory when it changed, and returns
// the folders whose contents changed since the last sync.
func (p *provider) sync(ctx context.Context) ([]string, error) {
	c, conn, e := p.openIMAP(ctx)
	if e != nil {
		return nil, e
	}
	defer c.Close()
	stop := context.AfterFunc(ctx, func() { conn.Close() })
	defer stop()
	// Bound the result before publishing the directory.
	cmd := c.List("", "*", &imap.ListOptions{ReturnSpecialUse: c.Caps().Has(imap.CapSpecialUse)})
	defer cmd.Close()
	folders := map[string]qmsg.Channel{}
	for data := cmd.Next(); data != nil; data = cmd.Next() {
		if len(folders) >= 1024 {
			return nil, fmt.Errorf("too many mailboxes")
		}
		attrs := []string{}
		skip := false
		for _, a := range data.Attrs {
			attrs = append(attrs, string(a))
			if a == imap.MailboxAttrNoSelect {
				skip = true
			}
		}
		if skip {
			continue
		}
		if len(data.Mailbox) > qmsg.MaxID {
			return nil, fmt.Errorf("mailbox name exceeds protocol limit")
		}
		ch := qmsg.Channel{ID: data.Mailbox, Name: data.Mailbox, Kind: qmsg.ChannelKind{Tag: qmsg.ChannelCustom, Custom: "mailbox"}, Mailbox: &qmsg.Mailbox{Attributes: attrs}}
		if data.Delim != 0 {
			d := string(data.Delim)
			ch.Mailbox.Delimiter = &d
		}
		folders[ch.ID] = ch
	}
	if e = cmd.Close(); e != nil {
		return nil, e
	}
	channels := []qmsg.Channel{}
	for _, ch := range folders {
		channels = append(channels, ch)
	}
	sort.Slice(channels, func(i, j int) bool { return channels[i].ID < channels[j].ID })
	options := &imap.StatusOptions{NumMessages: true, UIDNext: true, UIDValidity: true, NumUnseen: true, HighestModSeq: c.Caps().Has(imap.CapCondStore)}
	// Pipeline the STATUS commands; Gmail accounts have dozens of labels.
	cmds := make([]*imapclient.StatusCommand, len(channels))
	for i, ch := range channels {
		cmds[i] = c.Status(ch.ID, options)
	}
	states := map[string]folderState{}
	for i, ch := range channels {
		d, e := cmds[i].Wait()
		var ie *imap.Error
		if errors.As(e, &ie) {
			// The folder is listed but refuses STATUS. Leave it without a
			// baseline; it reports a change once STATUS works again.
			continue
		} else if e != nil {
			return nil, e
		}
		states[ch.ID] = statusState(d)
	}
	changed := changedFolders(p.states, states)
	for _, folder := range changed {
		if len(p.loadedMail[folder]) == 0 {
			continue
		}
		selected, err := c.Select(folder, &imap.SelectOptions{ReadOnly: true}).Wait()
		if err != nil {
			return nil, err
		}
		result, err := c.UIDSearch(&imap.SearchCriteria{}, nil).Wait()
		if err != nil {
			return nil, err
		}
		ref := qmsg.ChannelRef{Organization: qmsg.Str(p.cfg.address), Channel: folder}
		for _, id := range p.forgetMissingMail(folder, selected.UIDValidity, result.AllUIDs()) {
			if p.cx != nil {
				p.cx.Emit(qmsg.Deleted{Channel: ref, ID: id})
			}
		}
	}
	p.folders = folders
	p.states = states
	p.moveSupported = c.Caps().Has(imap.CapMove)
	// Polls repeat the same listing; only publish a different one.
	if p.cx != nil && !reflect.DeepEqual(channels, p.published) {
		p.published = channels
		p.cx.Directory(qmsg.OrganizationUpserted{Organization: qmsg.Organization{ID: p.cfg.address, Name: p.cfg.address, Me: qmsg.Str(p.cfg.address), Users: p.users(), Channels: channels}})
	}
	return changed, nil
}
func (p *provider) validRef(ref qmsg.ChannelRef) bool {
	if ref.Organization == nil || *ref.Organization != p.cfg.address {
		return false
	}
	_, ok := p.folders[ref.Channel]
	return ok
}
func mailID(validity uint32, uid imap.UID) string {
	return strconv.FormatUint(uint64(validity), 10) + ":" + strconv.FormatUint(uint64(uid), 10)
}
func parseID(id string, validity uint32) (imap.UID, error) {
	a, b, ok := strings.Cut(id, ":")
	v, e := strconv.ParseUint(a, 10, 32)
	u, ue := strconv.ParseUint(b, 10, 32)
	if !ok || e != nil || ue != nil || uint32(v) != validity || u == 0 {
		return 0, qmsg.ErrUnknownMessage{ID: id}
	}
	return imap.UID(u), nil
}
func fetchMail(c *imapclient.Client, ref qmsg.ChannelRef, validity uint32, uid imap.UID) (qmsg.Message, error) {
	section := &imap.FetchItemBodySection{Peek: true, Partial: &imap.SectionPartial{Offset: 0, Size: maxMail + 1}}
	rows, e := c.Fetch(imap.UIDSetNum(uid), &imap.FetchOptions{UID: true, Flags: true, InternalDate: true, BodySection: []*imap.FetchItemBodySection{section}}).Collect()
	if e != nil {
		return qmsg.Message{}, e
	}
	for _, r := range rows {
		if r.UID == uid {
			flags := []string{}
			for _, f := range r.Flags {
				flags = append(flags, string(f))
			}
			m, e := parseMail(r.FindBodySection(section), ref, mailID(validity, uid), flags)
			if e != nil {
				return m, qmsg.ErrProvider{Code: "mail_format", Message: e.Error()}
			}
			if !r.InternalDate.IsZero() {
				m.SentAt = uint64(r.InternalDate.UnixMilli())
			}
			return m, nil
		}
	}
	return qmsg.Message{}, qmsg.ErrUnknownMessage{ID: mailID(validity, uid)}
}
func (p *provider) history(ctx context.Context, ref qmsg.ChannelRef, cursor *string, limit uint32) (qmsg.HistoryPage, error) {
	page := qmsg.HistoryPage{}
	if !p.validRef(ref) {
		return page, qmsg.ErrUnknownChannel{Channel: ref}
	}
	n := min(int(limit), maxPage)
	if n == 0 {
		return page, nil
	}
	e := p.withMailbox(ctx, ref, func(c *imapclient.Client, s *imap.SelectData) error {
		var before imap.UID
		if cursor != nil {
			var e error
			before, e = parseID(*cursor, s.UIDValidity)
			if e != nil {
				return e
			}
		}
		criteria := &imap.SearchCriteria{}
		data, e := c.UIDSearch(criteria, nil).Wait()
		if e != nil {
			return e
		}
		uids := data.AllUIDs()
		if len(uids) > maxMailbox {
			return fmt.Errorf("mailbox exceeds UID limit")
		}
		if cursor == nil {
			for _, id := range p.forgetMissingMail(ref.Channel, s.UIDValidity, uids) {
				if p.cx != nil {
					p.cx.Emit(qmsg.Deleted{Channel: ref, ID: id})
				}
			}
		}
		uids, e = p.orderedMail(c, ref.Channel, s.UIDValidity, uids)
		if e != nil {
			return e
		}
		if before != 0 {
			index := -1
			for i, uid := range uids {
				if uid == before {
					index = i
					break
				}
			}
			if index < 0 {
				return qmsg.ErrUnknownMessage{ID: *cursor}
			}
			uids = uids[:index]
		}
		start := max(0, len(uids)-n)
		if start == len(uids) {
			return nil
		}
		byUID, err := fetchMailHeaders(c, ref, s.UIDValidity, uids[start:])
		if err != nil {
			return err
		}
		for i := len(uids) - 1; i >= start; i-- {
			if m, ok := byUID[uids[i]]; ok {
				page.Messages = append(page.Messages, m)
				p.know(m)
				p.rememberMail(ref.Channel, m.ID)
			}
		}
		if start > 0 && len(page.Messages) > 0 {
			page.NextCursor = qmsg.Str(page.Messages[len(page.Messages)-1].ID)
		}
		return nil
	})
	return page, e
}

type mailOrder struct {
	folder       string
	validity     uint32
	ids, ordered []imap.UID
}

// A UID records insertion order, not arrival date: moving an old message
// back into Inbox can give it a new UID. Cache one folder's date order so
// reopening and paging don't rescan its metadata.
func (p *provider) orderedMail(c *imapclient.Client, folder string, validity uint32, ids []imap.UID) ([]imap.UID, error) {
	sort.Slice(ids, func(i, j int) bool { return ids[i] < ids[j] })
	if cache := p.mailOrder; cache != nil && cache.folder == folder && cache.validity == validity && reflect.DeepEqual(cache.ids, ids) {
		return cache.ordered, nil
	}
	if len(ids) == 0 {
		return ids, nil
	}
	dates := map[imap.UID]time.Time{}
	cmd := c.Fetch(imap.UIDSetNum(ids...), &imap.FetchOptions{UID: true, InternalDate: true})
	defer cmd.Close()
	for row := cmd.Next(); row != nil; row = cmd.Next() {
		data, err := row.Collect()
		if err != nil {
			return nil, err
		}
		dates[data.UID] = data.InternalDate
	}
	if err := cmd.Close(); err != nil {
		return nil, err
	}
	ordered := append([]imap.UID(nil), ids...)
	sort.Slice(ordered, func(i, j int) bool {
		a, b := dates[ordered[i]], dates[ordered[j]]
		if a.Equal(b) {
			return ordered[i] < ordered[j]
		}
		return a.Before(b)
	})
	p.mailOrder = &mailOrder{folder: folder, validity: validity, ids: ids, ordered: ordered}
	return ordered, nil
}
func (p *provider) get(ctx context.Context, ref qmsg.ChannelRef, id string) (qmsg.Message, error) {
	var m qmsg.Message
	e := p.withMailbox(ctx, ref, func(c *imapclient.Client, s *imap.SelectData) error {
		uid, e := parseID(id, s.UIDValidity)
		if e != nil {
			return e
		}
		m, e = fetchMail(c, ref, s.UIDValidity, uid)
		return e
	})
	if e == nil {
		p.know(m)
		p.rememberMail(ref.Channel, m.ID)
	}
	return m, e
}
func (p *provider) flags(ctx context.Context, fc qmsg.InMessage, flag imap.Flag, add bool) error {
	return p.withMailbox(ctx, fc.Channel, func(c *imapclient.Client, s *imap.SelectData) error {
		uid, e := parseID(fc.ID, s.UIDValidity)
		if e != nil {
			return e
		}
		rows, e := fetchMailHeaders(c, fc.Channel, s.UIDValidity, []imap.UID{uid})
		if e != nil {
			return e
		}
		if _, found := rows[uid]; !found {
			return qmsg.ErrUnknownMessage{ID: fc.ID}
		}
		op := imap.StoreFlagsDel
		if add {
			op = imap.StoreFlagsAdd
		}
		if e = c.Store(imap.UIDSetNum(uid), &imap.StoreFlags{Op: op, Silent: true, Flags: []imap.Flag{flag}}, nil).Close(); e != nil {
			return e
		}
		rows, e = fetchMailHeaders(c, fc.Channel, s.UIDValidity, []imap.UID{uid})
		m, found := rows[uid]
		if e == nil && !found {
			return qmsg.ErrUnknownMessage{ID: fc.ID}
		}
		if e == nil && p.cx != nil {
			p.cx.Emit(qmsg.Received{Message: m})
		}
		return e
	})
}
func (p *provider) move(ctx context.Context, fc qmsg.InMessage, target string) error {
	if _, ok := p.folders[target]; !ok {
		return qmsg.ErrUnknownChannel{Channel: qmsg.ChannelRef{Organization: qmsg.Str(p.cfg.address), Channel: target}}
	}
	if target == fc.Channel.Channel {
		return fmt.Errorf("choose a different mailbox")
	}
	return p.withMailbox(ctx, fc.Channel, func(c *imapclient.Client, s *imap.SelectData) error {
		uid, e := parseID(fc.ID, s.UIDValidity)
		if e != nil {
			return e
		}
		if !c.Caps().Has(imap.CapMove) {
			return qmsg.ErrUnsupported{}
		}
		if _, e = fetchMail(c, fc.Channel, s.UIDValidity, uid); e != nil {
			return e
		}
		_, e = c.Move(imap.UIDSetNum(uid), target).Wait()
		if e == nil && p.cx != nil {
			p.cx.Emit(qmsg.Deleted{Channel: fc.Channel, ID: fc.ID})
			p.cx.Emit(qmsg.Gap{Channel: qmsg.ChannelRef{Organization: qmsg.Str(p.cfg.address), Channel: target}})
		}
		return e
	})
}
func (p *provider) smtpSend(ctx context.Context, raw []byte, envelope []string) (bool, error) {
	if e := p.refresh(ctx); e != nil {
		return false, e
	}
	conn, e := p.cfg.connect(ctx, p.cfg.smtp, p.cfg.smtpTLS == "tls")
	if e != nil {
		return false, e
	}
	defer conn.Close()
	stop := context.AfterFunc(ctx, func() { conn.Close() })
	defer stop()
	var client *smtp.Client
	if p.cfg.smtpTLS == "starttls" {
		client, e = smtp.NewClientStartTLS(conn, p.cfg.tlsFor(p.cfg.smtp))
		if e != nil {
			return false, e
		}
	} else {
		client = smtp.NewClient(conn)
	}
	defer client.Close()
	client.CommandTimeout = 30 * time.Second
	client.SubmissionTimeout = 45 * time.Second
	if e = client.Auth(p.auth()); e != nil {
		return false, e
	}
	a, _ := parseAddress(p.cfg.address)
	if e = client.Mail(a.Address, nil); e != nil {
		return false, e
	}
	for _, to := range envelope {
		if e = client.Rcpt(to, nil); e != nil {
			return false, e
		}
	}
	writer, e := client.Data()
	if e != nil {
		return false, e
	}
	if _, e = writer.Write(raw); e != nil {
		return true, e
	}
	e = writer.Close()
	return true, e
}

// SMTP acceptance and saving a Sent copy are separate outcomes.
func (p *provider) send(ctx context.Context, c compose) (qmsg.Record, error) {
	var read func(context.Context, string, uint64, uint32) ([]byte, error)
	if p.cx != nil {
		read = p.cx.ReadBlob
	}
	raw, id, envelope, e := composeMail(ctx, p.cfg.address, c, read)
	if e != nil {
		return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "mail", Message: e.Error()}}}
	}
	submitted, e := p.smtpSend(ctx, raw, envelope)
	if e != nil {
		var se *smtp.SMTPError
		if submitted && !errors.As(e, &se) {
			return nil, qmsg.ErrProvider{Code: "delivery_unknown", Message: "SMTP connection ended after DATA started. Check Sent before retrying."}
		}
		if authenticationError(e) {
			return nil, qmsg.ErrLoginRequired{}
		}
		var ce qmsg.CommandError
		if errors.As(e, &ce) {
			return nil, ce
		}
		return nil, definiteSMTPError(e)
	}
	saved := p.cfg.service == "gmail"
	var warning string
	if !saved {
		sent := p.specialFolder("\\Sent", p.cfg.sentMailbox)
		if sent != "" {
			client, conn, err := p.openIMAP(ctx)
			if err == nil {
				stop := context.AfterFunc(ctx, func() { conn.Close() })
				cmd := client.Append(sent, int64(len(raw)), &imap.AppendOptions{Flags: []imap.Flag{imap.FlagSeen}})
				_, err = cmd.Write(raw)
				if err == nil {
					err = cmd.Close()
				}
				if err == nil {
					_, err = cmd.Wait()
				}
				stop()
				client.Close()
			}
			saved = err == nil
		}
		if !saved {
			warning = "SMTP accepted the mail, but no Sent copy was saved. Do not resend."
		}
	}
	result := qmsg.Record{"accepted": qmsg.Boolean(true), "message_id": qmsg.Text(id), "sent_copy_saved": qmsg.Boolean(saved), "warning": qmsg.Text(warning)}
	return result, nil
}

func mailSize(m qmsg.Message) int {
	n := 0
	if m.Email != nil {
		n += len(m.Email.Subject)
		if m.Email.HTML != nil {
			n += len(*m.Email.HTML)
		}
	}
	for _, c := range m.Content {
		switch c := c.(type) {
		case qmsg.ContentText:
			n += len(c.Text)
		case qmsg.ContentFile:
			if s, ok := c.Media.Source.(qmsg.SourceBytes); ok {
				n += len(s.Bytes)
			}
		case qmsg.ContentImage:
			if s, ok := c.Media.Source.(qmsg.SourceBytes); ok {
				n += len(s.Bytes)
			}
		}
	}
	return n
}

func (p *provider) users() []qmsg.User {
	users := []qmsg.User{{ID: p.cfg.address, Name: p.cfg.address, Username: qmsg.Str(p.cfg.address)}}
	for _, a := range p.known {
		if a.Address != p.cfg.address {
			users = append(users, qmsg.User{ID: a.Address, Name: displayName(a), Username: qmsg.Str(a.Address)})
		}
	}
	sort.Slice(users, func(i, j int) bool { return users[i].ID < users[j].ID })
	return users
}

func definiteSMTPError(e error) qmsg.CommandError {
	code := "smtp_failed"
	message := e.Error()
	var se *smtp.SMTPError
	if errors.As(e, &se) {
		code = "smtp_" + strconv.Itoa(se.Code)
		message = se.Message
	}
	return qmsg.ErrProvider{Code: code, Message: validText(message), Details: qmsg.Record{"delivery_unknown": qmsg.Boolean(false)}}
}

// Only keep identities the client has loaded, so external moves can remove
// old rows without downloading the mailbox again.
func (p *provider) rememberMail(folder, id string) {
	if p.loadedMail == nil {
		p.loadedMail = map[string]map[string]bool{}
	}
	if p.loadedMail[folder] == nil {
		p.loadedMail[folder] = map[string]bool{}
	}
	p.loadedMail[folder][id] = true
}
func (p *provider) forgetMissingMail(folder string, validity uint32, uids []imap.UID) []string {
	present := map[string]bool{}
	for _, uid := range uids {
		present[mailID(validity, uid)] = true
	}
	gone := []string{}
	for id := range p.loadedMail[folder] {
		if !present[id] {
			gone = append(gone, id)
			delete(p.loadedMail[folder], id)
		}
	}
	sort.Strings(gone)
	return gone
}

func fetchMailHeaders(c *imapclient.Client, ref qmsg.ChannelRef, validity uint32, uids []imap.UID) (map[imap.UID]qmsg.Message, error) {
	section := &imap.FetchItemBodySection{Specifier: imap.PartSpecifierHeader, Peek: true, Partial: &imap.SectionPartial{Offset: 0, Size: 64 << 10}}
	set := imap.UIDSetNum(uids...)
	rows, err := c.Fetch(set, &imap.FetchOptions{UID: true, Flags: true, InternalDate: true, BodySection: []*imap.FetchItemBodySection{section}}).Collect()
	if err != nil {
		return nil, err
	}
	byUID := map[imap.UID]qmsg.Message{}
	for _, row := range rows {
		header, err := textproto.ReadHeader(bufio.NewReader(bytes.NewReader(row.FindBodySection(section))))
		if err != nil {
			return nil, fmt.Errorf("cannot read mail headers: %w", err)
		}
		flags := []string{}
		for _, flag := range row.Flags {
			flags = append(flags, string(flag))
		}
		m := mailHeader(gomail.Header{Header: message.Header{Header: header}}, ref, mailID(validity, row.UID), flags)
		m.Content = []qmsg.Content{qmsg.ContentCustom{Kind: "email-summary"}}
		if !row.InternalDate.IsZero() {
			m.SentAt = uint64(row.InternalDate.UnixMilli())
		}
		byUID[row.UID] = m
	}
	return byUID, nil
}
