package main

import (
	"context"
	"errors"
	"net/http"
	"sort"
	"strconv"
	"strings"

	"github.com/diamondburned/arikawa/v3/api"
	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/arikawa/v3/utils/httputil"
	"github.com/diamondburned/ningen/v3"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

// Function ids.
const (
	fnLogin        = "login"
	fnLogout       = "logout"
	fnOpenDM       = "open_dm"
	fnFindUsers    = "find_users"
	fnVerifyUser   = "verify_user"
	fnCompleteUser = "complete_user"
	fnSend         = "send"
	fnTyping       = "typing"
	fnEdit         = "edit"
	fnDelete       = "delete"
	fnReact        = "react"
	fnUnreact      = "unreact"
	fnMarkRead     = "mark_read"
)

// Standard actions, with the inputs of their contracts.

func action(id, label string, kind qmsg.ActionTag, result qmsg.ValueType, inputs ...qmsg.FunctionInput) qmsg.Function {
	return qmsg.Function{ID: id, Label: label, Kind: qmsg.KindAction{Action: qmsg.ActionKind{Tag: kind}, Result: result}, Inputs: inputs}
}

func input(id, label string, t qmsg.ValueType, required bool) qmsg.FunctionInput {
	return qmsg.FunctionInput{ID: id, Label: label, ValueType: t, Required: required, Hint: qmsg.HintPlain{}}
}

var userRecord = qmsg.TypeRecord{Fields: []qmsg.Field{
	{ID: "id", Label: "Id", ValueType: qmsg.TypeText{}, Required: true},
	{ID: "name", Label: "Name", ValueType: qmsg.TypeText{}, Required: true},
}}

func accountFunctions() []qmsg.Function {
	members := input(qmsg.InputMembers, "Members", qmsg.TypeList{Item: qmsg.TypeText{}}, true)
	members.Verification, members.Completion = qmsg.Str(fnVerifyUser), qmsg.Str(fnCompleteUser)
	query := input("query", "Name", qmsg.TypeText{}, true)
	query.Completion = qmsg.Str(fnCompleteUser)
	return []qmsg.Function{
		action(fnLogout, "Log out", qmsg.ActionLogout, qmsg.TypeNull{}),
		action(fnOpenDM, "Message users", qmsg.ActionOpenChannel, qmsg.TypeChannel{}, members),
		{ID: fnFindUsers, Label: "Find users", Description: "Users this account knows, by name.",
			Kind: qmsg.KindLookup{Result: qmsg.TypeList{Item: userRecord}}, Inputs: []qmsg.FunctionInput{query}},
		{ID: fnVerifyUser, Label: "Check user", Kind: qmsg.KindVerification{Type: qmsg.TypeText{}}},
		{ID: fnCompleteUser, Label: "Suggest users", Kind: qmsg.KindCompletion{Type: qmsg.TypeText{}}},
	}
}

func (p *provider) handle(ctx, reqCtx context.Context, c qmsg.Command) {
	reqCtx, cancel := context.WithCancel(reqCtx)
	defer cancel()
	go func() {
		select {
		case <-ctx.Done():
			cancel()
		case <-reqCtx.Done():
		}
	}()
	request, _ := qmsg.RequestOf(c)
	var r qmsg.Reply
	var err qmsg.CommandError
	switch c := c.(type) {
	case qmsg.CmdFunctions:
		r, err = p.functions(reqCtx, c.Context)
	case qmsg.CmdCall:
		r, err = p.call(reqCtx, c.Call)
	case qmsg.CmdVerify:
		r, err = p.verify(c.Verification)
	case qmsg.CmdComplete:
		r, err = p.complete(c.Completion)
	case qmsg.CmdSend:
		var id string
		id, err = p.send(reqCtx, c.Channel, c.Content, c.ReplyTo, c.Nonce)
		r = qmsg.ReplySent{ID: id}
	case qmsg.CmdHistory:
		r, err = p.history(reqCtx, c.Channel, c.Cursor, c.Limit)
	case qmsg.CmdGetMessage:
		r, err = p.getMessage(reqCtx, c.Channel, c.ID)
	case qmsg.CmdOpenChannel:
		var ch qmsg.ChannelRef
		ch, err = p.openChannel(reqCtx, c.Organization, c.Members)
		r = qmsg.ReplyOpened{Channel: ch}
	case qmsg.CmdReadBlob:
		err = qmsg.ErrUnknownBlob{ID: c.Blob}
	default:
		// ReleaseBlob: this provider shares no blobs.
		return
	}
	// Work that finished is answered as done: cancelling doesn't undo it.
	if err != nil {
		r = nil
	}
	p.cx.Reply(request, r, err)
}

func (p *provider) functions(ctx context.Context, fc qmsg.FunctionContext) (qmsg.Reply, qmsg.CommandError) {
	st, err := p.session()
	if err != nil {
		// Nothing works without a session but signing in.
		if _, ok := fc.(qmsg.InProvider); ok && !p.hasToken() {
			return qmsg.ReplyFunctions{Functions: []qmsg.Function{
				action(fnLogin, "Sign in", qmsg.ActionLogin, qmsg.TypeNull{}),
			}}, nil
		}
		return qmsg.ReplyFunctions{}, nil
	}
	var fs []qmsg.Function
	switch fc := fc.(type) {
	case qmsg.InProvider:
		fs = accountFunctions()
	case qmsg.InOrganization:
		if _, err := p.guild(st, fc.ID); err != nil {
			return nil, err
		}
		// Direct messages belong to no organization, so only lookups.
		for _, f := range accountFunctions() {
			if _, ok := f.Kind.(qmsg.KindAction); !ok {
				fs = append(fs, f)
			}
		}
	case qmsg.InChannel:
		ch, err := p.channel(st, fc.Channel)
		if err != nil {
			return nil, err
		}
		if p.canSend(st, ch) {
			fs = append(fs,
				action(fnSend, "Send", qmsg.ActionSendMessage, qmsg.TypeText{},
					input(qmsg.InputContent, "Content", qmsg.TypeContent{}, true),
					input(qmsg.InputReplyTo, "Reply to", qmsg.TypeText{}, false),
					input(qmsg.InputNonce, "Nonce", qmsg.TypeText{}, false)),
				action(fnTyping, "Typing", qmsg.ActionTyping, qmsg.TypeNull{}))
		}
	case qmsg.InMessage:
		ch, err := p.channel(st, fc.Channel)
		if err != nil {
			return nil, err
		}
		m, err := p.message(ctx, st, ch, fc.ID)
		if err != nil {
			return nil, err
		}
		own := m.Author.ID == p.me
		if own && p.canSend(st, ch) {
			fs = append(fs, action(fnEdit, "Edit", qmsg.ActionEditMessage, qmsg.TypeNull{},
				input(qmsg.InputContent, "Content", qmsg.TypeContent{}, true)))
		}
		if own || p.can(st, ch, discord.PermissionManageMessages) {
			fs = append(fs, action(fnDelete, "Delete", qmsg.ActionDeleteMessage, qmsg.TypeNull{}))
		}
		if p.can(st, ch, discord.PermissionAddReactions|discord.PermissionReadMessageHistory) {
			fs = append(fs,
				action(fnReact, "React", qmsg.ActionAddReaction, qmsg.TypeNull{}, input(qmsg.InputReaction, "Reaction", qmsg.TypeText{}, true)),
				action(fnUnreact, "Remove reaction", qmsg.ActionRemoveReaction, qmsg.TypeNull{}, input(qmsg.InputReaction, "Reaction", qmsg.TypeText{}, true)))
		}
		fs = append(fs, action(fnMarkRead, "Mark read", qmsg.ActionMarkRead, qmsg.TypeNull{}))
	}
	return qmsg.ReplyFunctions{Functions: fs}, nil
}

// call runs a function, checking again everything Functions promised.
func (p *provider) call(ctx context.Context, call qmsg.FunctionCall) (qmsg.Reply, qmsg.CommandError) {
	if call.Function == fnLogin {
		// Offered only while logged out, as Functions says.
		_, inProvider := call.Context.(qmsg.InProvider)
		if _, err := p.session(); err == nil || !inProvider || p.hasToken() {
			return nil, qmsg.ErrUnknownFunction{ID: call.Function}
		}
		if issues := checkInputs(qmsg.Function{}, call.Arguments); len(issues) > 0 {
			return nil, qmsg.ErrInvalidInput{Issues: issues}
		}
		if err := p.login(ctx); err != nil {
			return nil, err
		}
		return qmsg.ReplyValue{Value: qmsg.Null{}}, nil
	}
	if call.Function == fnLogout {
		if _, ok := call.Context.(qmsg.InProvider); !ok {
			return nil, qmsg.ErrUnknownFunction{ID: call.Function}
		}
		if err := p.logout("Logged out. Sign in to Discord to connect again."); err != nil {
			return nil, qmsg.ErrFailed{Reason: err.Error()}
		}
		return qmsg.ReplyValue{Value: qmsg.Null{}}, nil
	}
	st, err := p.session()
	if err != nil {
		return nil, err
	}
	// The function must be offered here now.
	fs, err := p.functions(ctx, call.Context)
	if err != nil {
		return nil, err
	}
	var fn *qmsg.Function
	for _, f := range fs.(qmsg.ReplyFunctions).Functions {
		if f.ID == call.Function {
			fn = &f
			break
		}
	}
	if fn == nil {
		return nil, qmsg.ErrUnknownFunction{ID: call.Function}
	}
	if issues := checkInputs(*fn, call.Arguments); len(issues) > 0 {
		return nil, qmsg.ErrInvalidInput{Issues: issues}
	}
	args := call.Arguments
	text := func(k string) *string {
		if v, ok := args[k].(qmsg.Text); ok {
			s := string(v)
			return &s
		}
		return nil
	}
	null := qmsg.ReplyValue{Value: qmsg.Null{}}
	switch call.Function {
	case fnFindUsers:
		var users qmsg.List
		for _, u := range p.findUsers(st, *text("query")) {
			users = append(users, qmsg.Record{"id": qmsg.Text(u.ID), "name": qmsg.Text(u.Name)})
		}
		return qmsg.ReplyValue{Value: users}, nil
	case fnOpenDM:
		var members []string
		for _, m := range args[qmsg.InputMembers].(qmsg.List) {
			members = append(members, string(m.(qmsg.Text)))
		}
		ref, err := p.openChannel(ctx, nil, members)
		if err != nil {
			return nil, err
		}
		return qmsg.ReplyValue{Value: qmsg.ChannelValue(ref)}, nil
	}
	switch fc := call.Context.(type) {
	case qmsg.InChannel:
		switch call.Function {
		case fnSend:
			id, err := p.send(ctx, fc.Channel, []qmsg.Content(args[qmsg.InputContent].(qmsg.ContentValue)), text(qmsg.InputReplyTo), text(qmsg.InputNonce))
			if err != nil {
				return nil, err
			}
			return qmsg.ReplyValue{Value: qmsg.Text(id)}, nil
		case fnTyping:
			ch, _ := p.channel(st, fc.Channel)
			if err := client(st, ctx).Typing(ch.ID); err != nil {
				return nil, commandError(err, qmsg.ErrUnknownChannel{Channel: fc.Channel})
			}
			return null, nil
		}
	case qmsg.InMessage:
		ch, _ := p.channel(st, fc.Channel)
		id, perr := discord.ParseSnowflake(fc.ID)
		if perr != nil {
			return nil, qmsg.ErrUnknownMessage{ID: fc.ID}
		}
		msg := discord.MessageID(id)
		unknown := qmsg.ErrUnknownMessage{ID: fc.ID}
		var apiErr error
		var event qmsg.MessageEvent
		switch call.Function {
		case fnEdit:
			body, err := sendable([]qmsg.Content(args[qmsg.InputContent].(qmsg.ContentValue)))
			if err != nil {
				return nil, err
			}
			if ctx.Err() != nil {
				return nil, qmsg.ErrCancelled{}
			}
			var edited *discord.Message
			edited, apiErr = client(st, ctx).EditText(ch.ID, msg, body)
			if apiErr == nil {
				edited.GuildID = ch.GuildID
				st.Cabinet.MessageSet(edited, false)
				event = qmsg.Edited{Channel: fc.Channel, ID: fc.ID, Content: contents(*edited), EditedAt: millis(edited.EditedTimestamp)}
			}
		case fnDelete:
			if ctx.Err() != nil {
				return nil, qmsg.ErrCancelled{}
			}
			apiErr = client(st, ctx).DeleteMessage(ch.ID, msg, "")
			if apiErr == nil {
				st.Cabinet.MessageRemove(ch.ID, msg)
			}
			event = qmsg.Deleted{Channel: fc.Channel, ID: fc.ID}
		case fnReact:
			apiErr = client(st, ctx).React(ch.ID, msg, discord.APIEmoji(*text(qmsg.InputReaction)))
			event = qmsg.Reacted{Channel: fc.Channel, ID: fc.ID, User: p.me.String(), Key: *text(qmsg.InputReaction), Added: true}
		case fnUnreact:
			apiErr = client(st, ctx).Unreact(ch.ID, msg, discord.APIEmoji(*text(qmsg.InputReaction)))
			event = qmsg.Reacted{Channel: fc.Channel, ID: fc.ID, User: p.me.String(), Key: *text(qmsg.InputReaction), Added: false}
		case fnMarkRead:
			apiErr = client(st, ctx).Ack(ch.ID, msg, &api.Ack{})
			event = qmsg.Read{Channel: fc.Channel, User: p.me.String(), UpTo: qmsg.Str(fc.ID)}
		}
		if apiErr != nil {
			return nil, commandError(apiErr, unknown)
		}
		if event != nil {
			p.emit(event)
		}
		return null, nil
	}
	return nil, qmsg.ErrUnknownFunction{ID: call.Function}
}

// checkInputs checks required, unknown and wrongly typed inputs, as the
// orchestrator's Function::check does, for the types used here.
func checkInputs(f qmsg.Function, args qmsg.Arguments) []qmsg.InputIssue {
	var issues []qmsg.InputIssue
	declared := map[string]qmsg.FunctionInput{}
	for _, in := range f.Inputs {
		declared[in.ID] = in
		if _, ok := args[in.ID]; !ok && in.Required {
			issues = append(issues, qmsg.InputIssue{Input: qmsg.Str(in.ID), Code: "required", Message: in.Label + " is required"})
		}
	}
	for _, k := range sortedArgs(args) {
		in, ok := declared[k]
		if !ok {
			issues = append(issues, qmsg.InputIssue{Input: qmsg.Str(k), Code: "unknown", Message: "no input " + k})
		} else if !fits(in.ValueType, args[k]) {
			issues = append(issues, qmsg.InputIssue{Input: qmsg.Str(k), Code: "type", Message: in.Label + " has the wrong type"})
		}
	}
	return issues
}

func sortedArgs(a qmsg.Arguments) []string {
	keys := make([]string, 0, len(a))
	for k := range a {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
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
		for _, item := range l {
			if !fits(t.Item, item) {
				return false
			}
		}
		return true
	}
	return false
}

func (p *provider) guild(st *ningen.State, id string) (*discord.Guild, qmsg.CommandError) {
	g, err := discord.ParseSnowflake(id)
	if err != nil {
		return nil, qmsg.ErrUnknownOrganization{ID: id}
	}
	guild, gerr := st.Cabinet.Guild(discord.GuildID(g))
	if gerr != nil {
		return nil, qmsg.ErrUnknownOrganization{ID: id}
	}
	return guild, nil
}

// channel resolves ref to a channel the account can see.
func (p *provider) channel(st *ningen.State, ref qmsg.ChannelRef) (discord.Channel, qmsg.CommandError) {
	unknown := qmsg.ErrUnknownChannel{Channel: ref}
	id, err := discord.ParseSnowflake(ref.Channel)
	if err != nil {
		return discord.Channel{}, unknown
	}
	ch, cerr := st.Cabinet.Channel(discord.ChannelID(id))
	if cerr != nil {
		return discord.Channel{}, unknown
	}
	org := ""
	if ref.Organization != nil {
		org = *ref.Organization
	}
	want := ""
	if ch.GuildID.IsValid() {
		want = ch.GuildID.String()
	}
	if org != want || !p.can(st, *ch, discord.PermissionViewChannel) {
		return discord.Channel{}, unknown
	}
	return *ch, nil
}

func (p *provider) message(ctx context.Context, st *ningen.State, ch discord.Channel, id string) (*discord.Message, qmsg.CommandError) {
	unknown := qmsg.ErrUnknownMessage{ID: id}
	s, err := discord.ParseSnowflake(id)
	if err != nil {
		return nil, unknown
	}
	if m, err := st.Cabinet.Message(ch.ID, discord.MessageID(s)); err == nil {
		return m, nil
	}
	if !p.can(st, ch, discord.PermissionReadMessageHistory) {
		return nil, qmsg.ErrForbidden{Reason: "no access to this channel's history"}
	}
	// Discord's single-message endpoint only allows bot accounts. User
	// accounts read history around the id; a deleted id can return a
	// neighbouring message, so never assume the first result is the target.
	messages, err := client(st, ctx).MessagesAround(ch.ID, discord.MessageID(s), 1)
	if err != nil {
		return nil, commandError(err, unknown)
	}
	for _, message := range messages {
		if message.ID == discord.MessageID(s) {
			return &message, nil
		}
	}
	return nil, unknown
}

func (p *provider) send(ctx context.Context, ref qmsg.ChannelRef, content []qmsg.Content, replyTo, nonce *string) (string, qmsg.CommandError) {
	id, err, submitted := p.trySend(ctx, ref, content, replyTo, nonce)
	if err != nil && nonce != nil && !submitted {
		p.cx.Emit(qmsg.NotSent{Channel: ref, Nonce: *nonce, Error: err})
	}
	return id, err
}

// The third result means the REST call started. Once it starts, a timeout
// or cancellation cannot prove that Discord did not accept the message.
func (p *provider) trySend(ctx context.Context, ref qmsg.ChannelRef, content []qmsg.Content, replyTo, nonce *string) (string, qmsg.CommandError, bool) {
	if ctx.Err() != nil {
		return "", qmsg.ErrCancelled{}, false
	}
	st, err := p.session()
	if err != nil {
		return "", err, false
	}
	ch, err := p.channel(st, ref)
	if err != nil {
		return "", err, false
	}
	if !p.canSend(st, ch) {
		return "", qmsg.ErrForbidden{Reason: "cannot send messages here"}, false
	}
	text, files, err := p.sendParts(ctx, content)
	if err != nil {
		return "", err, false
	}
	data := api.SendMessageData{Content: text, Files: files, AllowedMentions: &api.AllowedMentions{Parse: []api.AllowedMentionType{api.AllowUserMention, api.AllowRoleMention, api.AllowEveryoneMention}}}
	if nonce != nil {
		data.Nonce = *nonce
	}
	if replyTo != nil {
		r, perr := discord.ParseSnowflake(*replyTo)
		if perr != nil {
			return "", qmsg.ErrUnknownMessage{ID: *replyTo}, false
		}
		data.Reference = &discord.MessageReference{MessageID: discord.MessageID(r), ChannelID: ch.ID, GuildID: ch.GuildID}
	}
	// The last moment a cancel still stops the send.
	if ctx.Err() != nil {
		return "", qmsg.ErrCancelled{}, false
	}
	m, serr := client(st, ctx).SendMessageComplex(ch.ID, data)
	if serr != nil {
		var response *httputil.HTTPError
		submitted := !(errors.As(serr, &response) && response.Status >= 400 && response.Status < 500 && response.Status != http.StatusRequestTimeout)
		return "", commandError(serr, qmsg.ErrUnknownChannel{Channel: ref}), submitted
	}
	m.GuildID = ch.GuildID
	st.Cabinet.MessageSet(m, false)
	p.knowMessageUsers(*m)
	p.emit(qmsg.Received{Message: message(*m)})
	return m.ID.String(), nil, true
}

// maxPage is the most messages Discord returns in one request.
const maxPage = 100

func (p *provider) history(ctx context.Context, ref qmsg.ChannelRef, cursor *string, limit uint32) (qmsg.Reply, qmsg.CommandError) {
	st, err := p.session()
	if err != nil {
		return nil, err
	}
	ch, err := p.channel(st, ref)
	if err != nil {
		return nil, err
	}
	if !holdsMessages(ch.Type) {
		return nil, qmsg.ErrUnsupported{}
	}
	if !p.can(st, ch, discord.PermissionReadMessageHistory) {
		return nil, qmsg.ErrForbidden{Reason: "no access to this channel's history"}
	}
	var before discord.MessageID
	if cursor != nil {
		s, perr := discord.ParseSnowflake(*cursor)
		if perr != nil {
			return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "cursor", Message: "invalid cursor"}}}
		}
		before = discord.MessageID(s)
	}
	n := min(limit, maxPage)
	if n == 0 {
		return qmsg.ReplyHistory{Page: qmsg.HistoryPage{NextCursor: cursor}}, nil
	}
	msgs, herr := client(st, ctx).MessagesBefore(ch.ID, before, uint(n))
	if herr != nil {
		return nil, commandError(herr, qmsg.ErrUnknownChannel{Channel: ref})
	}
	page := qmsg.HistoryPage{}
	for _, m := range msgs {
		m.GuildID = ch.GuildID
		p.knowMessageUsers(m)
		page.Messages = append(page.Messages, message(m))
	}
	if len(msgs) == int(n) {
		page.NextCursor = qmsg.Str(msgs[len(msgs)-1].ID.String())
	}
	return qmsg.ReplyHistory{Page: page}, nil
}

func (p *provider) getMessage(ctx context.Context, ref qmsg.ChannelRef, id string) (qmsg.Reply, qmsg.CommandError) {
	st, err := p.session()
	if err != nil {
		return nil, err
	}
	ch, err := p.channel(st, ref)
	if err != nil {
		return nil, err
	}
	m, err := p.message(ctx, st, ch, id)
	if err != nil {
		return nil, err
	}
	m.GuildID = ch.GuildID
	p.knowMessageUsers(*m)
	return qmsg.ReplyMessage{Message: message(*m)}, nil
}

// openChannel opens a direct message with one user, or a group with more.
func (p *provider) openChannel(ctx context.Context, org *string, members []string) (qmsg.ChannelRef, qmsg.CommandError) {
	st, err := p.session()
	if err != nil {
		return qmsg.ChannelRef{}, err
	}
	if org != nil {
		return qmsg.ChannelRef{}, qmsg.ErrUnsupported{}
	}
	var ids []discord.UserID
	var issues []qmsg.InputIssue
	for _, m := range members {
		if v := p.verifyUser(st, m); v != nil {
			issues = append(issues, *v)
			continue
		}
		s, _ := discord.ParseSnowflake(m)
		if discord.UserID(s) != p.me {
			ids = append(ids, discord.UserID(s))
		}
	}
	if len(issues) > 0 {
		return qmsg.ChannelRef{}, qmsg.ErrInvalidInput{Issues: issues}
	}
	if len(ids) == 0 {
		return qmsg.ChannelRef{}, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Input: qmsg.Str(qmsg.InputMembers), Code: "empty", Message: "name at least one other user"}}}
	}
	if ctx.Err() != nil {
		return qmsg.ChannelRef{}, qmsg.ErrCancelled{}
	}
	var ch discord.Channel
	var cerr error
	if len(ids) == 1 {
		var c *discord.Channel
		c, cerr = client(st, ctx).CreatePrivateChannel(ids[0])
		if c != nil {
			ch = *c
		}
	} else {
		// Group direct messages take a recipient list on the same endpoint.
		cerr = client(st, ctx).RequestJSON(&ch, "POST", api.EndpointMe+"/channels",
			httputil.WithJSONBody(map[string]any{"recipients": ids}))
	}
	if cerr != nil {
		return qmsg.ChannelRef{}, commandError(cerr, nil)
	}
	ch = resolveDMRecipients(st, ch)
	for _, u := range ch.DMRecipients {
		p.knowUser(nil, u)
	}
	p.cx.Directory(qmsg.ChannelUpserted{Channel: channel(ch, p.me, true)})
	return channelRef(0, ch.ID), nil
}

type knownUser struct{ ID, Name, Username string }

// knownUsers lists the users this session knows: direct message recipients
// guild members, and users from Discord's deduplicated READY cache, once each.
func (p *provider) knownUsers(st *ningen.State) []knownUser {
	seen := map[discord.UserID]int{}
	var out []knownUser
	add := func(u discord.User, name string) {
		if u.ID == p.me || !u.ID.IsValid() {
			return
		}
		if index, ok := seen[u.ID]; ok {
			if out[index].Username == "" {
				out[index].Username = u.Username
			}
			if out[index].Name == "" || out[index].Name == u.ID.String() {
				out[index].Name = name
			}
			return
		}
		seen[u.ID] = len(out)
		out = append(out, knownUser{u.ID.String(), name, u.Username})
	}
	if dms, err := st.Cabinet.PrivateChannels(); err == nil {
		for _, ch := range dms {
			for _, u := range ch.DMRecipients {
				add(u, userName(u))
			}
		}
	}
	if guilds, err := st.Cabinet.Guilds(); err == nil {
		for _, g := range guilds {
			if members, err := st.Cabinet.Members(g.ID); err == nil {
				for _, m := range members {
					add(m.User, memberName(m))
				}
			}
		}
	}
	// With DedupeUserObjects, friends and some DM recipients are stored
	// in ningen's global user cache rather than as full channel/member users.
	st.PresenceStore.Each(0, func(presence *discord.Presence) bool {
		add(presence.User, userName(presence.User))
		return false
	})
	sort.Slice(out, func(i, j int) bool { return strings.ToLower(out[i].Name) < strings.ToLower(out[j].Name) })
	return out
}

func (p *provider) findUsers(st *ningen.State, query string) []knownUser {
	q := strings.ToLower(strings.TrimSpace(query))
	var out []knownUser
	for _, u := range p.knownUsers(st) {
		if q == "" || strings.Contains(strings.ToLower(u.Name), q) || strings.Contains(strings.ToLower(u.Username), q) || u.ID == q {
			out = append(out, u)
		}
	}
	return out
}

func (p *provider) verifyUser(st *ningen.State, id string) *qmsg.InputIssue {
	in := qmsg.Str(qmsg.InputMembers)
	s, err := discord.ParseSnowflake(id)
	if err != nil || !s.IsValid() {
		return &qmsg.InputIssue{Input: in, Code: "invalid_id", Message: id + " is not a Discord user id"}
	}
	for _, u := range p.knownUsers(st) {
		if u.ID == id {
			return nil
		}
	}
	if discord.UserID(s) == p.me {
		return nil
	}
	return &qmsg.InputIssue{Input: in, Code: "unknown_user", Message: "no known user " + id}
}

func (p *provider) verify(v qmsg.VerificationRequest) (qmsg.Reply, qmsg.CommandError) {
	if v.Call.Function != fnVerifyUser {
		return nil, qmsg.ErrUnknownFunction{ID: v.Call.Function}
	}
	st, err := p.session()
	if err != nil {
		return nil, err
	}
	text, ok := v.Value.(qmsg.Text)
	if !ok {
		return qmsg.ReplyVerified{Verification: qmsg.Invalid{Issues: []qmsg.InputIssue{{Code: "type", Message: "expected a user id"}}}}, nil
	}
	if issue := p.verifyUser(st, string(text)); issue != nil {
		if v.Input == nil {
			issue.Input = nil
		} else {
			issue.Input = qmsg.Str(v.Input.Input)
		}
		return qmsg.ReplyVerified{Verification: qmsg.Invalid{Issues: []qmsg.InputIssue{*issue}}}, nil
	}
	return qmsg.ReplyVerified{Verification: qmsg.Valid{}}, nil
}

func (p *provider) complete(c qmsg.CompletionRequest) (qmsg.Reply, qmsg.CommandError) {
	if c.Call.Function != fnCompleteUser {
		return nil, qmsg.ErrUnknownFunction{ID: c.Call.Function}
	}
	st, err := p.session()
	if err != nil {
		return nil, err
	}
	start := 0
	if c.Cursor != nil {
		n, perr := strconv.Atoi(*c.Cursor)
		if perr != nil || n < 0 {
			return nil, qmsg.ErrInvalidInput{Issues: []qmsg.InputIssue{{Code: "cursor", Message: "invalid cursor"}}}
		}
		start = n
	}
	users := p.findUsers(st, c.Query)
	page := qmsg.CompletionPage{}
	end := min(start+int(c.Limit), len(users))
	for i := start; i < end; i++ {
		u := users[i]
		page.Items = append(page.Items, qmsg.CompletionItem{Label: u.Name, Description: qmsg.Str("@" + u.Username), Value: qmsg.Text(u.ID)})
	}
	if end < len(users) && c.Limit > 0 {
		page.NextCursor = qmsg.Str(strconv.Itoa(end))
	}
	return qmsg.ReplyCompleted{Page: page}, nil
}
