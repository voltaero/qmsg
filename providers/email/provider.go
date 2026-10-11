package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	imap "github.com/emersion/go-imap/v2"
	smtp "github.com/emersion/go-smtp"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

func init() { qmsg.Register(run) }
func main() {}

type provider struct {
	pending       *oauthLogin
	moveSupported bool
	cx            *qmsg.Context
	cfg           config
	cred          credential
	folders       map[string]qmsg.Channel
	states        map[string]folderState
	published     []qmsg.Channel
	reported      qmsg.ProviderStatus
	known         map[string]qmsg.EmailAddress
	ready         bool
}

func (p *provider) storeCredential() error {
	if p.cx == nil {
		return nil
	}
	b, e := json.Marshal(p.cred)
	if e != nil {
		return e
	}
	return p.cx.SecretSet("credentials", b)
}

// status reports s unless it is already the current status, so that each
// poll does not repeat Ready or the same failure.
func (p *provider) status(s qmsg.ProviderStatus) {
	if p.cx != nil && !reflect.DeepEqual(s, p.reported) {
		p.reported = s
		p.cx.Status(s)
	}
}

// gaps asks clients to reload folders whose contents changed.
func (p *provider) gaps(folders []string) {
	if p.cx == nil {
		return
	}
	for _, id := range folders {
		p.cx.Emit(qmsg.Gap{Channel: qmsg.ChannelRef{Organization: qmsg.Str(p.cfg.address), Channel: id}})
	}
}
func (p *provider) loginRequired() {
	p.ready = false
	p.status(qmsg.StatusLoginRequired{Step: qmsg.LoginStep{Message: "Sign in to your mail account.", Function: qmsg.Str("login")}})
}
func run(cx *qmsg.Context) error {
	cfg, e := configuration(cx.Config().Settings)
	if e != nil {
		return e
	}
	p := &provider{cx: cx, cfg: cfg, folders: map[string]qmsg.Channel{}, known: map[string]qmsg.EmailAddress{}}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() {
		select {
		case <-cx.Done():
			cancel()
		case <-ctx.Done():
		}
	}()
	p.status(qmsg.StatusConnecting{})
	b, e := cx.SecretGet("credentials")
	if e != nil {
		return e
	}
	if len(b) > 0 {
		if e = json.Unmarshal(b, &p.cred); e != nil {
			return fmt.Errorf("stored mail credentials are invalid")
		}
		if e = p.list(ctx); e == nil {
			p.ready = true
			if e = p.storeCredential(); e != nil {
				return e
			}
			p.status(qmsg.StatusReady{})
		} else if authenticationError(e) {
			p.loginRequired()
		} else {
			p.status(qmsg.StatusDisconnected{Reason: "Mail connection failed; retrying."})
		}
	} else {
		p.loginRequired()
	}
	// Commands stay cancellable while waiting. One IMAP operation at a time;
	// each uses its own connection, so cancellation cannot corrupt the next one.
	gate := make(chan struct{}, 1)
	workers := make(chan struct{}, 16)
	var wg sync.WaitGroup
	ticker := time.NewTicker(60 * time.Second)
	defer ticker.Stop()
	wg.Add(1)
	go func() {
		defer wg.Done()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				select {
				case gate <- struct{}{}:
					if p.pending != nil && !time.Now().Before(p.pending.deadline) {
						p.pending = nil
						p.loginRequired()
					}
					if p.ready || p.cred.Password != "" || p.cred.RefreshToken != "" {
						pollCtx, stop := context.WithTimeout(ctx, 45*time.Second)
						changed, err := p.sync(pollCtx)
						stop()
						if err == nil {
							p.ready = true
							p.status(qmsg.StatusReady{})
							// A failed poll keeps the old baseline, so changes
							// made while disconnected still show up here.
							p.gaps(changed)
						} else if p.pending != nil && !p.ready {
							// Keep showing the pending sign-in step.
						} else if authenticationError(err) {
							p.loginRequired()
						} else {
							p.status(qmsg.StatusDisconnected{Reason: "Mail sync failed; retrying."})
						}
					}
					<-gate
				default:
				}
			}
		}
	}()
commands:
	for {
		cmd, requestCtx, e := cx.NextCommand(ctx)
		if e != nil {
			break
		}
		if _, ok := cmd.(qmsg.CmdShutdown); ok {
			cancel()
			break
		}
		select {
		case workers <- struct{}{}:
		case <-ctx.Done():
			break commands
		}
		wg.Add(1)
		go func(c qmsg.Command, rctx context.Context) {
			defer wg.Done()
			defer func() { <-workers }()
			req, has := qmsg.RequestOf(c)
			if !has {
				return
			}
			select {
			case gate <- struct{}{}:
			case <-rctx.Done():
				cx.Reply(req, nil, qmsg.ErrCancelled{})
				return
			case <-ctx.Done():
				return
			}
			defer func() { <-gate }()
			rctx, stop := context.WithCancel(rctx)
			defer stop()
			done := context.AfterFunc(ctx, stop)
			defer done()
			reply, err := p.handle(rctx, c)
			if err != nil {
				if authenticationError(err) {
					p.loginRequired()
				}
				cx.Reply(req, nil, commandError(rctx, err))
			} else {
				cx.Reply(req, reply, nil)
			}
		}(cmd, requestCtx)
	}
	cancel()
	wg.Wait()
	return nil
}
func input(id, label string, t qmsg.ValueType, required bool) qmsg.FunctionInput {
	return qmsg.FunctionInput{ID: id, Label: label, ValueType: t, Required: required, Hint: qmsg.HintPlain{}}
}
func action(id, label string, tag qmsg.ActionTag, result qmsg.ValueType, inputs ...qmsg.FunctionInput) qmsg.Function {
	return qmsg.Function{ID: id, Label: label, Kind: qmsg.KindAction{Action: qmsg.ActionKind{Tag: tag, Custom: id}, Result: result}, Inputs: inputs}
}

var sentType = qmsg.TypeRecord{Fields: []qmsg.Field{{ID: "accepted", Label: "Accepted by SMTP", ValueType: qmsg.TypeBoolean{}, Required: true}, {ID: "message_id", Label: "RFC Message-ID", ValueType: qmsg.TypeText{}, Required: true}, {ID: "sent_copy_saved", Label: "Sent copy saved", ValueType: qmsg.TypeBoolean{}, Required: true}, {ID: "warning", Label: "Warning", ValueType: qmsg.TypeText{}, Required: true}}}

func (p *provider) functions(fc qmsg.FunctionContext) ([]qmsg.Function, error) {
	if p.pending != nil && !time.Now().Before(p.pending.deadline) {
		p.pending = nil
		p.loginRequired()
	}
	if !p.ready {
		if _, ok := fc.(qmsg.InProvider); !ok {
			return nil, nil
		}
		var inputs []qmsg.FunctionInput
		if p.cfg.service != "gmail" {
			secret := input("password", "Password", qmsg.TypeText{}, true)
			secret.Hint = qmsg.HintSecret{}
			inputs = append(inputs, secret)
		} else if p.cfg.oauthRedirect == "" {
			inputs = append(inputs, input("redirect_uri", "Client callback address", qmsg.TypeText{}, true))
		}
		result := qmsg.ValueType(qmsg.TypeNull{})
		if p.cfg.service == "gmail" {
			result = oauthResultType
		}
		label := "Sign in"
		if p.cfg.service == "gmail" && p.cfg.oauthRedirect != "" {
			label = "Sign in with Google"
		}
		fs := []qmsg.Function{action("login", label, qmsg.ActionLogin, result, inputs...)}
		if p.pending != nil {
			code := input("code", "Authorization code", qmsg.TypeText{}, true)
			code.Hint = qmsg.HintSecret{}
			fs = append(fs, action("finish_login", "Finish sign-in", qmsg.ActionLogin, qmsg.TypeNull{}, code, input("state", "Login state", qmsg.TypeText{}, true)), action("cancel_login", "Cancel sign-in", qmsg.ActionLogin, qmsg.TypeNull{}))
		}
		if p.cred.Password != "" || p.cred.RefreshToken != "" {
			fs = append(fs, action("logout", "Log out", qmsg.ActionLogout, qmsg.TypeNull{}))
		}
		return fs, nil
	}
	composeFn := func() qmsg.Function {
		to := input("to", "To", qmsg.TypeList{Item: qmsg.TypeText{}}, true)
		to.Verification = qmsg.Str("verify_address")
		to.Completion = qmsg.Str("complete_address")
		cc := to
		cc.ID = "cc"
		cc.Label = "Cc"
		cc.Required = false
		bcc := cc
		bcc.ID = "bcc"
		bcc.Label = "Bcc"
		html := input("html", "HTML body", qmsg.TypeText{}, false)
		html.Hint = qmsg.HintMultiline{}
		return action("compose", "Send email", qmsg.ActionCustom, sentType, to, cc, bcc, input("subject", "Subject", qmsg.TypeText{}, true), input("content", "Body and attachments", qmsg.TypeContent{}, true), html)
	}
	fs := []qmsg.Function{{ID: "complete_mailbox", Label: "Suggest mailboxes", Kind: qmsg.KindCompletion{Type: qmsg.TypeText{}}}, {ID: "verify_address", Label: "Check email address", Kind: qmsg.KindVerification{Type: qmsg.TypeText{}}}, {ID: "complete_address", Label: "Suggest email addresses", Kind: qmsg.KindCompletion{Type: qmsg.TypeText{}}}, {ID: "find_addresses", Label: "Find known addresses", Kind: qmsg.KindLookup{Result: qmsg.TypeList{Item: qmsg.TypeText{}}}, Inputs: []qmsg.FunctionInput{input("query", "Query", qmsg.TypeText{}, true)}}}
	switch fc := fc.(type) {
	case qmsg.InProvider:
		fs = append(fs, composeFn(), action("logout", "Log out", qmsg.ActionLogout, qmsg.TypeNull{}))
	case qmsg.InOrganization:
		if fc.ID != p.cfg.address {
			return nil, qmsg.ErrUnknownOrganization{ID: fc.ID}
		}
		fs = append(fs, composeFn())
	case qmsg.InChannel:
		if !p.validRef(fc.Channel) {
			return nil, qmsg.ErrUnknownChannel{Channel: fc.Channel}
		}
		fs = append(fs, composeFn())
	case qmsg.InMessage:
		if !p.validRef(fc.Channel) {
			return nil, qmsg.ErrUnknownChannel{Channel: fc.Channel}
		}
		for _, id := range []string{"mark_read", "mark_unread", "star", "unstar"} {
			fs = append(fs, action(id, strings.ReplaceAll(id, "_", " "), qmsg.ActionCustom, qmsg.TypeNull{}))
		}
		if p.moveSupported {
			mailbox := input("mailbox", "Mailbox", qmsg.TypeText{}, true)
			mailbox.Completion = qmsg.Str("complete_mailbox")
			fs = append(fs, action("move", "Move to folder", qmsg.ActionCustom, qmsg.TypeNull{}, mailbox))
			if trash := p.specialFolder("\\Trash", p.cfg.trashMailbox); trash != "" && trash != fc.Channel.Channel {
				fs = append(fs, action("trash", "Move to trash", qmsg.ActionDeleteMessage, qmsg.TypeNull{}))
			}
		}
		fs = append(fs, action("reply", "Reply", qmsg.ActionCustom, sentType, input("content", "Body and attachments", qmsg.TypeContent{}, true), input("html", "HTML body", qmsg.TypeText{}, false)))
	}
	return fs, nil
}
func checkInputs(fn qmsg.Function, args qmsg.Arguments) error {
	known := map[string]qmsg.FunctionInput{}
	for _, in := range fn.Inputs {
		known[in.ID] = in
		if _, ok := args[in.ID]; !ok && in.Required {
			return fmt.Errorf("missing input %s", in.ID)
		}
	}
	for k, v := range args {
		in, ok := known[k]
		if !ok || !fits(in.ValueType, v) {
			return fmt.Errorf("invalid input %s", k)
		}
	}
	return nil
}
func fits(t qmsg.ValueType, v qmsg.Value) bool {
	switch t := t.(type) {
	case qmsg.TypeText:
		_, ok := v.(qmsg.Text)
		return ok
	case qmsg.TypeContent:
		_, ok := v.(qmsg.ContentValue)
		return ok
	case qmsg.TypeList:
		l, ok := v.(qmsg.List)
		if !ok {
			return false
		}
		for _, i := range l {
			if !fits(t.Item, i) {
				return false
			}
		}
		return true
	}
	return false
}
func text(args qmsg.Arguments, k string) string { s, _ := args[k].(qmsg.Text); return string(s) }
func list(args qmsg.Arguments, k string) []string {
	vs, _ := args[k].(qmsg.List)
	out := []string{}
	for _, v := range vs {
		s, _ := v.(qmsg.Text)
		out = append(out, string(s))
	}
	return out
}
func (p *provider) call(ctx context.Context, call qmsg.FunctionCall) (qmsg.Reply, error) {
	fs, e := p.functions(call.Context)
	if e != nil {
		return nil, e
	}
	var found *qmsg.Function
	for _, f := range fs {
		if f.ID == call.Function {
			copy := f
			found = &copy
			break
		}
	}
	if found == nil {
		return nil, qmsg.ErrUnknownFunction{ID: call.Function}
	}
	if e = checkInputs(*found, call.Arguments); e != nil {
		return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "input", Message: e.Error()}}}
	}
	null := qmsg.ReplyValue{Value: qmsg.Null{}}
	switch call.Function {
	case "cancel_login":
		p.pending = nil
		p.loginRequired()
		return null, nil
	case "login", "finish_login":
		if p.cfg.service == "gmail" && call.Function == "login" {
			return p.beginOAuth(text(call.Arguments, "redirect_uri"))
		}
		old := p.cred
		if p.cfg.service == "gmail" {
			cred, err := p.finishOAuth(ctx, text(call.Arguments, "code"), text(call.Arguments, "state"))
			if err != nil {
				return nil, err
			}
			p.cred = cred
		} else {
			p.cred = credential{Password: text(call.Arguments, "password")}
		}
		changed, e := p.sync(ctx)
		if e != nil {
			p.cred = old
			return nil, e
		}
		if e = p.storeCredential(); e != nil {
			p.cred = old
			return nil, e
		}
		p.ready = true
		p.status(qmsg.StatusReady{})
		// Signing in again after a revoked login keeps the old baseline.
		p.gaps(changed)
		return null, nil
	case "logout":
		if e = p.cx.SecretDelete("credentials"); e != nil {
			return nil, e
		}
		p.cred = credential{}
		p.folders = map[string]qmsg.Channel{}
		p.states = nil
		p.published = nil
		p.known = map[string]qmsg.EmailAddress{}
		p.cx.Directory(qmsg.OrganizationRemoved{ID: p.cfg.address})
		p.loginRequired()
		return null, nil
	case "find_addresses":
		values := qmsg.List{}
		for _, a := range p.find(text(call.Arguments, "query")) {
			values = append(values, qmsg.Text(a.Address))
		}
		return qmsg.ReplyValue{Value: values}, nil
	case "compose", "reply":
		c := compose{to: list(call.Arguments, "to"), cc: list(call.Arguments, "cc"), bcc: list(call.Arguments, "bcc"), subject: text(call.Arguments, "subject"), html: text(call.Arguments, "html")}
		cv, _ := call.Arguments["content"].(qmsg.ContentValue)
		c.content = []qmsg.Content(cv)
		if call.Function == "reply" {
			fc := call.Context.(qmsg.InMessage)
			m, e := p.get(ctx, fc.Channel, fc.ID)
			if e != nil {
				return nil, e
			}
			if m.Email == nil {
				return nil, qmsg.ErrUnsupported{}
			}
			as := m.Email.ReplyTo
			if m.Author == p.cfg.address && len(m.Email.To) > 0 {
				as = m.Email.To
			} else if len(as) == 0 {
				as = m.Email.From
			}
			for _, a := range as {
				c.to = append(c.to, a.Address)
			}
			c.subject = m.Email.Subject
			if !strings.HasPrefix(strings.ToLower(c.subject), "re:") {
				c.subject = "Re: " + c.subject
			}
			c.inReplyTo = m.Email.MessageID
			c.references = append([]string{}, m.Email.References...)
			if c.inReplyTo != nil {
				c.references = append(c.references, *c.inReplyTo)
			}
		}
		result, e := p.send(ctx, c)
		if e != nil {
			return nil, e
		}
		return qmsg.ReplyValue{Value: result}, nil
	}
	fc, ok := call.Context.(qmsg.InMessage)
	if !ok {
		return nil, qmsg.ErrUnknownFunction{ID: call.Function}
	}
	switch call.Function {
	case "mark_read":
		e = p.flags(ctx, fc, imap.FlagSeen, true)
	case "mark_unread":
		e = p.flags(ctx, fc, imap.FlagSeen, false)
	case "star":
		e = p.flags(ctx, fc, imap.FlagFlagged, true)
	case "unstar":
		e = p.flags(ctx, fc, imap.FlagFlagged, false)
	case "trash":
		e = p.move(ctx, fc, p.specialFolder("\\Trash", p.cfg.trashMailbox))
	case "move":
		e = p.move(ctx, fc, text(call.Arguments, "mailbox"))
	default:
		return nil, qmsg.ErrUnsupported{}
	}
	return null, e
}
func (p *provider) handle(ctx context.Context, cmd qmsg.Command) (qmsg.Reply, error) {
	switch c := cmd.(type) {
	case qmsg.CmdFunctions:
		fs, e := p.functions(c.Context)
		return qmsg.ReplyFunctions{Functions: fs}, e
	case qmsg.CmdCall:
		return p.call(ctx, c.Call)
	}
	if !p.ready {
		return nil, qmsg.ErrLoginRequired{}
	}
	switch c := cmd.(type) {
	case qmsg.CmdHistory:
		page, e := p.history(ctx, c.Channel, c.Cursor, c.Limit)
		return qmsg.ReplyHistory{Page: page}, e
	case qmsg.CmdGetMessage:
		m, e := p.get(ctx, c.Channel, c.ID)
		return qmsg.ReplyMessage{Message: m}, e
	case qmsg.CmdVerify:
		if c.Verification.Call.Function != "verify_address" {
			return nil, qmsg.ErrUnknownFunction{ID: c.Verification.Call.Function}
		}
		s, ok := c.Verification.Value.(qmsg.Text)
		_, e := parseAddress(string(s))
		if !ok || e != nil {
			return qmsg.ReplyVerified{Verification: qmsg.Invalid{Issues: []qmsg.InputIssue{{Code: "address", Message: "Enter a valid email address."}}}}, nil
		}
		return qmsg.ReplyVerified{Verification: qmsg.Valid{}}, nil
	case qmsg.CmdComplete:
		return p.complete(c.Completion)
	case qmsg.CmdReadBlob:
		return nil, qmsg.ErrUnknownBlob{ID: c.Blob}
	default:
		return nil, qmsg.ErrUnsupported{}
	}
}
func (p *provider) know(m qmsg.Message) {
	if m.Email == nil {
		return
	}
	for _, as := range [][]qmsg.EmailAddress{m.Email.From, m.Email.To, m.Email.Cc, m.Email.ReplyTo} {
		for _, a := range as {
			if len(p.known) < 2000 && len(a.Address) <= qmsg.MaxID {
				if a.Name != nil && len(*a.Name) > 256 {
					name := string([]rune(*a.Name)[:min(len([]rune(*a.Name)), 128)])
					a.Name = &name
				}
				old, exists := p.known[a.Address]
				p.known[a.Address] = a
				if p.cx != nil && (!exists || displayName(old) != displayName(a)) {
					p.cx.Directory(qmsg.UserUpserted{Organization: qmsg.Str(p.cfg.address), User: qmsg.User{ID: a.Address, Name: displayName(a), Username: qmsg.Str(a.Address)}})
				}
			}
		}
	}
}
func (p *provider) find(query string) []qmsg.EmailAddress {
	q := strings.ToLower(query)
	out := []qmsg.EmailAddress{}
	for _, a := range p.known {
		name := ""
		if a.Name != nil {
			name = *a.Name
		}
		if strings.Contains(strings.ToLower(a.Address+" "+name), q) {
			out = append(out, a)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Address < out[j].Address })
	return out
}
func (p *provider) complete(c qmsg.CompletionRequest) (qmsg.Reply, error) {
	if c.Call.Function != "complete_address" && c.Call.Function != "complete_mailbox" {
		return nil, qmsg.ErrUnknownFunction{ID: c.Call.Function}
	}
	start := 0
	if c.Cursor != nil {
		n, e := strconv.Atoi(*c.Cursor)
		if e != nil || n < 0 {
			return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "cursor", Message: "Invalid cursor"}}}
		}
		start = n
	}
	as := p.find(c.Query)
	if c.Call.Function == "complete_mailbox" {
		as = nil
		for name := range p.folders {
			if strings.Contains(strings.ToLower(name), strings.ToLower(c.Query)) {
				as = append(as, qmsg.EmailAddress{Address: name})
			}
		}
		sort.Slice(as, func(i, j int) bool { return as[i].Address < as[j].Address })
	}
	start = min(start, len(as))
	end := start + min(int(c.Limit), 100, len(as)-start)
	page := qmsg.CompletionPage{}
	for _, a := range as[start:end] {
		label := a.Address
		if a.Name != nil {
			label = *a.Name
		}
		page.Items = append(page.Items, qmsg.CompletionItem{Label: label, Description: qmsg.Str(a.Address), Value: qmsg.Text(a.Address)})
	}
	if end < len(as) && end > start {
		page.NextCursor = qmsg.Str(strconv.Itoa(end))
	}
	return qmsg.ReplyCompleted{Page: page}, nil
}

func authenticationError(e error) bool {
	var login qmsg.ErrLoginRequired
	if errors.As(e, &login) {
		return true
	}
	var ie *imap.Error
	if errors.As(e, &ie) && ie.Code == imap.ResponseCodeAuthenticationFailed {
		return true
	}
	var se *smtp.SMTPError
	return errors.As(e, &se) && se.Code == 535
}
func commandError(ctx context.Context, e error) qmsg.CommandError {
	var ce qmsg.CommandError
	if errors.As(e, &ce) {
		return ce
	}
	if ctx.Err() != nil {
		return qmsg.ErrCancelled{}
	}
	var se *smtp.SMTPError
	if errors.As(e, &se) {
		return qmsg.ErrProvider{Code: "smtp_" + strconv.Itoa(se.Code), Message: se.Message}
	}
	var ie *imap.Error
	if errors.As(e, &ie) {
		return qmsg.ErrProvider{Code: "imap_" + string(ie.Code), Message: ie.Text}
	}
	s := e.Error()
	if len(s) > 512 {
		s = string([]rune(s)[:min(len([]rune(s)), 256)])
	}
	return qmsg.ErrProvider{Code: "mail_error", Message: s}
}

func displayName(a qmsg.EmailAddress) string {
	if a.Name != nil && *a.Name != "" {
		return *a.Name
	}
	return a.Address
}
func (p *provider) specialFolder(attr, configured string) string {
	if configured != "" {
		if _, ok := p.folders[configured]; ok {
			return configured
		}
		return ""
	}
	for name, ch := range p.folders {
		if ch.Mailbox != nil {
			for _, a := range ch.Mailbox.Attributes {
				if strings.EqualFold(a, attr) {
					return name
				}
			}
		}
	}
	return ""
}
