package qmsg

import (
	"fmt"
	"sort"
)

// EncodeProviderMessage encodes what a provider sends.
func EncodeProviderMessage(m ProviderMessage) []byte {
	return encode(func(e *encoder) { e.providerMessage(m) })
}

// DecodeHostMessage decodes what the orchestrator sends.
func DecodeHostMessage(b []byte) (HostMessage, error) {
	return decode(b, (*decoder).hostMessage)
}

// DecodeProviderConfig decodes the config passed to run.
func DecodeProviderConfig(b []byte) (ProviderConfig, error) {
	return decode(b, func(d *decoder) ProviderConfig {
		return ProviderConfig{Name: d.str(), HostURL: d.str(), Settings: d.strMap()}
	})
}

// The other directions, for tests and tools that play the orchestrator.

func EncodeHostMessage(m HostMessage) []byte {
	return encode(func(e *encoder) { e.hostMessage(m) })
}

func DecodeProviderMessage(b []byte) (ProviderMessage, error) {
	return decode(b, (*decoder).providerMessage)
}

func EncodeProviderConfig(c ProviderConfig) []byte {
	return encode(func(e *encoder) { e.str(c.Name); e.str(c.HostURL); e.strMap(c.Settings) })
}

func bad(kind string, v any) string { return fmt.Sprintf("qmsg: cannot encode %s %T", kind, v) }

// Encoding.

func (e *encoder) channelRef(c ChannelRef) { e.optStr(c.Organization); e.str(c.Channel) }

func (e *encoder) message(m Message) {
	e.str(m.ID)
	e.channelRef(m.Channel)
	e.str(m.Author)
	e.u64(m.SentAt)
	e.optStr(m.ReplyTo)
	e.contents(m.Content)
	e.optU64(m.EditedAt)
	e.varint(uint64(len(m.Reactions)))
	for _, r := range m.Reactions {
		e.str(r.Key)
		e.u32(r.Count)
		e.boolean(r.Me)
	}
	e.optStr(m.Nonce)
	e.boolean(m.Email != nil)
	if m.Email != nil {
		e.email(*m.Email)
	}
}

func (e *encoder) email(m Email) {
	e.str(m.Subject)
	for _, list := range [][]EmailAddress{m.From, m.To, m.Cc, m.Bcc, m.ReplyTo} {
		e.varint(uint64(len(list)))
		for _, a := range list {
			e.str(a.Address)
			e.optStr(a.Name)
		}
	}
	e.optStr(m.MessageID)
	e.optStr(m.InReplyTo)
	e.varint(uint64(len(m.References)))
	for _, r := range m.References {
		e.str(r)
	}
	e.optStr(m.HTML)
	keys := make([]string, 0, len(m.ContentIDs))
	for k := range m.ContentIDs {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	e.varint(uint64(len(keys)))
	for _, k := range keys {
		e.str(k)
		e.str(m.ContentIDs[k])
	}
	e.boolean(m.Read)
	e.boolean(m.Starred)
	e.boolean(m.Draft)
}
func (d *decoder) optEmail() *Email {
	if !d.option() {
		return nil
	}
	m := &Email{Subject: d.str()}
	for _, list := range []*[]EmailAddress{&m.From, &m.To, &m.Cc, &m.Bcc, &m.ReplyTo} {
		*list = seq(d, func() EmailAddress { return EmailAddress{d.str(), d.optStr()} })
	}
	m.MessageID = d.optStr()
	m.InReplyTo = d.optStr()
	m.References = seq(d, d.str)
	m.HTML = d.optStr()
	n := d.count()
	if n > 0 {
		m.ContentIDs = map[string]string{}
	}
	for i := 0; i < n; i++ {
		k := d.str()
		m.ContentIDs[k] = d.str()
	}
	m.Read = d.boolean()
	m.Starred = d.boolean()
	m.Draft = d.boolean()
	return m
}

func (e *encoder) messageEvent(v MessageEvent) {
	switch v := v.(type) {
	case Received:
		e.tag(0)
		e.message(v.Message)
	case Edited:
		e.tag(1)
		e.channelRef(v.Channel)
		e.str(v.ID)
		e.contents(v.Content)
		e.u64(v.EditedAt)
	case Deleted:
		e.tag(2)
		e.channelRef(v.Channel)
		e.str(v.ID)
	case Reacted:
		e.tag(3)
		e.channelRef(v.Channel)
		e.str(v.ID)
		e.str(v.User)
		e.str(v.Key)
		e.boolean(v.Added)
	case Read:
		e.tag(4)
		e.channelRef(v.Channel)
		e.str(v.User)
		e.optStr(v.UpTo)
	case Typing:
		e.tag(5)
		e.channelRef(v.Channel)
		e.str(v.User)
	case Gap:
		e.tag(6)
		e.channelRef(v.Channel)
	case NotSent:
		e.tag(7)
		e.channelRef(v.Channel)
		e.str(v.Nonce)
		e.commandError(v.Error)
	default:
		panic(bad("message event", v))
	}
}

func (e *encoder) contents(c []Content) {
	e.varint(uint64(len(c)))
	for _, c := range c {
		e.content(c)
	}
}

func (e *encoder) content(c Content) {
	switch c := c.(type) {
	case ContentText:
		e.tag(0)
		e.str(c.Text)
	case ContentFormatted:
		e.tag(1)
		e.formatted(c.Text)
	case ContentImage:
		e.tag(2)
		e.media(c.Media)
	case ContentVideo:
		e.tag(3)
		e.media(c.Media)
	case ContentAudio:
		e.tag(4)
		e.media(c.Media)
	case ContentFile:
		e.tag(5)
		e.media(c.Media)
	case ContentCustom:
		e.tag(6)
		e.str(c.Kind)
		e.bytes(c.Data)
	default:
		panic(bad("content", c))
	}
}

func (e *encoder) media(m Media) {
	e.optStr(m.Name)
	e.optStr(m.Mime)
	e.optU64(m.Size)
	switch s := m.Source.(type) {
	case SourceBytes:
		e.tag(0)
		e.bytes(s.Bytes)
	case SourceURL:
		e.tag(1)
		e.str(s.URL)
	case SourceBlob:
		e.tag(2)
		e.str(s.Blob)
	default:
		panic(bad("media source", s))
	}
	e.optU32(m.Metadata.Width)
	e.optU32(m.Metadata.Height)
	e.optU64(m.Metadata.DurationMs)
	e.optU64(m.Metadata.ExpiresAtMs)
	e.optStr(m.Metadata.PreviewURL)
	e.optStr(m.Metadata.Description)
	e.boolean(m.Metadata.Spoiler)
}
func (e *encoder) optMedia(m *Media) {
	e.boolean(m != nil)
	if m != nil {
		e.media(*m)
	}
}

func (e *encoder) contentKind(k ContentKind) {
	e.tag(uint32(k.Tag))
	if k.Tag == KindCustom {
		e.str(k.Custom)
	}
}

func (e *encoder) formatted(f FormattedText) {
	e.str(f.Text)
	e.varint(uint64(len(f.Spans)))
	for _, s := range f.Spans {
		e.u32(s.Start)
		e.u32(s.End)
		e.style(s.Style)
	}
}

func (e *encoder) style(s Style) {
	switch s := s.(type) {
	case StyleBold:
		e.tag(0)
	case StyleItalic:
		e.tag(1)
	case StyleUnderline:
		e.tag(2)
	case StyleStrikethrough:
		e.tag(3)
	case StyleCode:
		e.tag(4)
	case StyleCodeBlock:
		e.tag(5)
		e.optStr(s.Language)
	case StyleQuote:
		e.tag(6)
	case StyleSpoiler:
		e.tag(7)
	case StyleLink:
		e.tag(8)
		e.str(s.URL)
	case StyleMention:
		e.tag(9)
		switch m := s.Mention.(type) {
		case MentionUser:
			e.tag(0)
			e.str(m.ID)
		case MentionChannel:
			e.tag(1)
			e.str(m.ID)
		case MentionRole:
			e.tag(2)
			e.str(m.ID)
		case MentionEveryone:
			e.tag(3)
		default:
			panic(bad("mention", m))
		}
	default:
		panic(bad("style", s))
	}
}

func (e *encoder) user(u User) {
	e.str(u.ID)
	e.str(u.Name)
	e.optStr(u.Username)
	e.optMedia(u.Avatar)
}

func (e *encoder) channel(c Channel) {
	e.str(c.ID)
	e.str(c.Name)
	e.optStr(c.Topic)
	e.optMedia(c.Icon)
	e.tag(uint32(c.Kind.Tag))
	if c.Kind.Tag == ChannelCustom {
		e.str(c.Kind.Custom)
	}
	e.optStr(c.Parent)
	e.optU32(c.Position)
	e.boolean(c.Members != nil)
	if c.Members != nil {
		e.varint(uint64(len(*c.Members)))
		for _, m := range *c.Members {
			e.str(m)
		}
	}
	e.varint(uint64(len(c.AcceptedContent)))
	for _, r := range c.AcceptedContent {
		e.contentKind(r.Kind)
		e.optU64(r.MaxSize)
		e.tag(uint32(r.TextUnit))
		e.varint(uint64(len(r.MimeTypes)))
		for _, m := range r.MimeTypes {
			e.str(m)
		}
	}
	e.optU32(c.Limits.MaxAttachments)
	e.optU64(c.Limits.MaxTotalSize)
	e.boolean(c.Mailbox != nil)
	if c.Mailbox != nil {
		e.optStr(c.Mailbox.Delimiter)
		e.varint(uint64(len(c.Mailbox.Attributes)))
		for _, a := range c.Mailbox.Attributes {
			e.str(a)
		}
	}
}

func (e *encoder) organization(o Organization) {
	e.str(o.ID)
	e.str(o.Name)
	e.optMedia(o.Icon)
	e.optStr(o.Me)
	e.varint(uint64(len(o.Users)))
	for _, u := range o.Users {
		e.user(u)
	}
	e.varint(uint64(len(o.Channels)))
	for _, c := range o.Channels {
		e.channel(c)
	}
}

func (e *encoder) violation(v Violation) {
	switch v := v.(type) {
	case ViolationEmpty:
		e.tag(0)
	case ViolationUnsupported:
		e.tag(1)
		e.u32(v.Part)
		e.contentKind(v.Kind)
	case ViolationUnsupportedMime:
		e.tag(2)
		e.u32(v.Part)
		e.optStr(v.Mime)
	case ViolationTextTooLong:
		e.tag(3)
		e.u64(v.Length)
		e.u64(v.Max)
	case ViolationInvalidFormatting:
		e.tag(4)
		e.u32(v.Part)
	case ViolationTooLarge:
		e.tag(5)
		e.u32(v.Part)
		e.u64(v.Size)
		e.u64(v.Max)
	case ViolationTooManyAttachments:
		e.tag(6)
		e.u32(v.Count)
		e.u32(v.Max)
	case ViolationTotalTooLarge:
		e.tag(7)
		e.u64(v.Size)
		e.u64(v.Max)
	default:
		panic(bad("violation", v))
	}
}

func (e *encoder) directoryUpdate(u DirectoryUpdate) {
	switch u := u.(type) {
	case OrganizationUpserted:
		e.tag(0)
		e.organization(u.Organization)
	case OrganizationUpdated:
		e.tag(1)
		e.str(u.ID)
		e.str(u.Name)
		e.optMedia(u.Icon)
	case OrganizationRemoved:
		e.tag(2)
		e.str(u.ID)
	case UserUpserted:
		e.tag(3)
		e.optStr(u.Organization)
		e.user(u.User)
	case UserRemoved:
		e.tag(4)
		e.optStr(u.Organization)
		e.str(u.ID)
	case ChannelUpserted:
		e.tag(5)
		e.optStr(u.Organization)
		e.channel(u.Channel)
	case ChannelRemoved:
		e.tag(6)
		e.channelRef(u.Channel)
	case Me:
		e.tag(7)
		e.optStr(u.Organization)
		e.optStr(u.ID)
	case PresenceChanged:
		e.tag(8)
		e.optStr(u.Organization)
		e.str(u.User)
		e.tag(uint32(u.Presence))
	case DirectoryReset:
		e.tag(9)
	default:
		panic(bad("directory update", u))
	}
}

func (e *encoder) status(s ProviderStatus) {
	switch s := s.(type) {
	case StatusConnecting:
		e.tag(0)
	case StatusLoginRequired:
		e.tag(1)
		e.str(s.Step.Message)
		e.optStr(s.Step.Function)
		e.optStr(s.Step.Link)
		e.optStr(s.Step.QRCode)
	case StatusSyncing:
		e.tag(2)
	case StatusReady:
		e.tag(3)
	case StatusDisconnected:
		e.tag(4)
		e.str(s.Reason)
	case StatusFailed:
		e.tag(5)
		e.str(s.Reason)
	default:
		panic(bad("status", s))
	}
}

func (e *encoder) command(c Command) {
	switch c := c.(type) {
	case CmdFunctions:
		e.tag(0)
		e.u64(c.Request)
		e.functionContext(c.Context)
	case CmdCall:
		e.tag(1)
		e.u64(c.Request)
		e.functionCall(c.Call)
	case CmdVerify:
		e.tag(2)
		e.u64(c.Request)
		e.functionCall(c.Verification.Call)
		e.inputRef(c.Verification.Input)
		e.value(c.Verification.Value)
	case CmdComplete:
		e.tag(3)
		e.u64(c.Request)
		e.functionCall(c.Completion.Call)
		e.inputRef(c.Completion.Input)
		e.str(c.Completion.Query)
		e.optStr(c.Completion.Cursor)
		e.u32(c.Completion.Limit)
	case CmdSend:
		e.tag(4)
		e.u64(c.Request)
		e.channelRef(c.Channel)
		e.optStr(c.ReplyTo)
		e.contents(c.Content)
		e.optStr(c.Nonce)
	case CmdHistory:
		e.tag(5)
		e.u64(c.Request)
		e.channelRef(c.Channel)
		e.optStr(c.Cursor)
		e.u32(c.Limit)
	case CmdGetMessage:
		e.tag(6)
		e.u64(c.Request)
		e.channelRef(c.Channel)
		e.str(c.ID)
	case CmdOpenChannel:
		e.tag(7)
		e.u64(c.Request)
		e.optStr(c.Organization)
		e.varint(uint64(len(c.Members)))
		for _, m := range c.Members {
			e.str(m)
		}
	case CmdReadBlob:
		e.tag(8)
		e.u64(c.Request)
		e.str(c.Blob)
		e.u64(c.Offset)
		e.u32(c.Len)
	case CmdReleaseBlob:
		e.tag(9)
		e.str(c.Blob)
	case CmdCancel:
		e.tag(10)
		e.u64(c.Request)
	case CmdShutdown:
		e.tag(11)
	default:
		panic(bad("command", c))
	}
}

func (e *encoder) inputRef(r *FunctionInputRef) {
	e.boolean(r != nil)
	if r != nil {
		e.str(r.Function)
		e.str(r.Input)
	}
}

func (e *encoder) reply(r Reply) {
	switch r := r.(type) {
	case ReplyFunctions:
		e.tag(0)
		e.varint(uint64(len(r.Functions)))
		for _, f := range r.Functions {
			e.function(f)
		}
	case ReplyValue:
		e.tag(1)
		e.value(r.Value)
	case ReplyVerified:
		e.tag(2)
		e.verification(r.Verification)
	case ReplyCompleted:
		e.tag(3)
		e.varint(uint64(len(r.Page.Items)))
		for _, it := range r.Page.Items {
			e.str(it.Label)
			e.optStr(it.Description)
			e.value(it.Value)
		}
		e.optStr(r.Page.NextCursor)
	case ReplySent:
		e.tag(4)
		e.str(r.ID)
	case ReplyOpened:
		e.tag(5)
		e.channelRef(r.Channel)
	case ReplyHistory:
		e.tag(6)
		e.varint(uint64(len(r.Page.Messages)))
		for _, m := range r.Page.Messages {
			e.message(m)
		}
		e.optStr(r.Page.NextCursor)
	case ReplyMessage:
		e.tag(7)
		e.message(r.Message)
	case ReplyBlob:
		e.tag(8)
		e.bytes(r.Bytes)
	default:
		panic(bad("reply", r))
	}
}

func (e *encoder) issues(is []InputIssue) {
	e.varint(uint64(len(is)))
	for _, i := range is {
		e.optStr(i.Input)
		e.str(i.Code)
		e.str(i.Message)
	}
}

func (e *encoder) verification(v Verification) {
	switch v := v.(type) {
	case Valid:
		e.tag(0)
	case Invalid:
		e.tag(1)
		e.issues(v.Issues)
	default:
		panic(bad("verification", v))
	}
}

func (e *encoder) commandError(c CommandError) {
	switch c := c.(type) {
	case ErrUnsupported:
		e.tag(0)
	case ErrUnknownFunction:
		e.tag(1)
		e.str(c.ID)
	case ErrInvalidInput:
		e.tag(2)
		e.issues(c.Issues)
	case ErrProvider:
		e.tag(3)
		e.str(c.Code)
		e.str(c.Message)
		e.boolean(c.Details != nil)
		if c.Details != nil {
			e.value(c.Details)
		}
	case ErrUnknownOrganization:
		e.tag(4)
		e.str(c.ID)
	case ErrUnknownChannel:
		e.tag(5)
		e.channelRef(c.Channel)
	case ErrUnknownUser:
		e.tag(6)
		e.str(c.ID)
	case ErrUnknownMessage:
		e.tag(7)
		e.str(c.ID)
	case ErrUnknownBlob:
		e.tag(8)
		e.str(c.ID)
	case ErrRejected:
		e.tag(9)
		e.violation(c.Violation)
	case ErrRateLimited:
		e.tag(10)
		e.optU64(c.RetryAfterMs)
	case ErrForbidden:
		e.tag(11)
		e.str(c.Reason)
	case ErrLoginRequired:
		e.tag(12)
	case ErrCancelled:
		e.tag(13)
	case ErrFailed:
		e.tag(14)
		e.str(c.Reason)
	default:
		panic(bad("command error", c))
	}
}

func (e *encoder) providerMessage(m ProviderMessage) {
	switch m := m.(type) {
	case PmLog:
		e.tag(0)
		e.tag(uint32(m.Level))
		e.str(m.Message)
	case PmMessage:
		e.tag(1)
		e.messageEvent(m.Event)
	case PmStatus:
		e.tag(2)
		e.status(m.Status)
	case PmSecretGet:
		e.tag(3)
		e.str(m.Key)
	case PmSecretSet:
		e.tag(4)
		e.str(m.Key)
		e.bytes(m.Value)
	case PmSecretDelete:
		e.tag(5)
		e.str(m.Key)
	case PmDirectory:
		e.tag(6)
		e.directoryUpdate(m.Update)
	case PmReply:
		e.tag(7)
		e.u64(m.Request)
		if m.Err != nil {
			e.tag(1)
			e.commandError(m.Err)
		} else {
			e.tag(0)
			e.reply(m.Reply)
		}
	case PmReadBlob:
		e.tag(8)
		e.str(m.Blob)
		e.u64(m.Offset)
		e.u32(m.Len)
	case PmLookup:
		e.tag(9)
		e.str(m.Name)
	case PmBrowserLogin:
		e.tag(10)
		e.u64(m.Request)
		e.str(m.Provider)
	case PmCancelBrowserLogin:
		e.tag(11)
		e.u64(m.Request)
	default:
		panic(bad("provider message", m))
	}
}

func (e *encoder) result(err *string) {
	if err != nil {
		e.tag(1)
		e.str(*err)
	} else {
		e.tag(0)
	}
}

func (e *encoder) hostMessage(m HostMessage) {
	switch m := m.(type) {
	case HmCommand:
		e.tag(0)
		e.command(m.Command)
	case HmSecret:
		e.tag(1)
		e.str(m.Key)
		e.result(m.Err)
		if m.Err == nil {
			e.boolean(m.Found)
			if m.Found {
				e.bytes(m.Value)
			}
		}
	case HmSecretStored:
		e.tag(2)
		e.str(m.Key)
		e.result(m.Err)
	case HmBlob:
		e.tag(3)
		e.str(m.Blob)
		e.u64(m.Offset)
		e.result(m.Err)
		if m.Err == nil {
			e.bytes(m.Bytes)
		}
	case HmAddresses:
		e.tag(4)
		e.str(m.Name)
		e.result(m.Err)
		if m.Err == nil {
			e.varint(uint64(len(m.Addresses)))
			for _, a := range m.Addresses {
				if len(a.IP) == 4 {
					e.tag(0)
				} else {
					e.tag(1)
				}
				e.b = append(e.b, a.IP...)
				e.varint(uint64(a.Port))
			}
		}
	case HmBrowserLogin:
		e.tag(5)
		e.u64(m.Request)
		e.result(m.Err)
		if m.Err == nil {
			e.str(m.Token)
		}
	default:
		panic(bad("host message", m))
	}
}

func (e *encoder) value(v Value) {
	switch v := v.(type) {
	case Null:
		e.tag(0)
	case Boolean:
		e.tag(1)
		e.boolean(bool(v))
	case Integer:
		e.tag(2)
		e.i64(int64(v))
	case Text:
		e.tag(3)
		e.str(string(v))
	case Bytes:
		e.tag(4)
		e.bytes(v)
	case List:
		e.tag(5)
		e.varint(uint64(len(v)))
		for _, v := range v {
			e.value(v)
		}
	case Record:
		e.tag(6)
		e.arguments(Arguments(v))
	case ContentValue:
		e.tag(7)
		e.contents(v)
	case ChannelValue:
		e.tag(8)
		e.channelRef(ChannelRef(v))
	case MessageValue:
		e.tag(9)
		e.message(Message(v))
	default:
		panic(bad("value", v))
	}
}

func (e *encoder) arguments(a Arguments) {
	e.varint(uint64(len(a)))
	for _, k := range sortedKeys(a) {
		e.str(k)
		e.value(a[k])
	}
}

func (e *encoder) valueType(t ValueType) {
	switch t := t.(type) {
	case TypeAny:
		e.tag(0)
	case TypeNull:
		e.tag(1)
	case TypeBoolean:
		e.tag(2)
	case TypeInteger:
		e.tag(3)
	case TypeText:
		e.tag(4)
	case TypeBytes:
		e.tag(5)
	case TypeList:
		e.tag(6)
		e.valueType(t.Item)
	case TypeRecord:
		e.tag(7)
		e.varint(uint64(len(t.Fields)))
		for _, f := range t.Fields {
			e.str(f.ID)
			e.str(f.Label)
			e.valueType(f.ValueType)
			e.boolean(f.Required)
		}
	case TypeContent:
		e.tag(8)
	case TypeChannel:
		e.tag(9)
	case TypeMessage:
		e.tag(10)
	default:
		panic(bad("value type", t))
	}
}

func (e *encoder) functionContext(c FunctionContext) {
	switch c := c.(type) {
	case InProvider:
		e.tag(0)
	case InOrganization:
		e.tag(1)
		e.str(c.ID)
	case InChannel:
		e.tag(2)
		e.channelRef(c.Channel)
	case InMessage:
		e.tag(3)
		e.channelRef(c.Channel)
		e.str(c.ID)
	default:
		panic(bad("function context", c))
	}
}

func (e *encoder) functionCall(c FunctionCall) {
	e.str(c.Function)
	e.functionContext(c.Context)
	e.arguments(c.Arguments)
}

func (e *encoder) function(f Function) {
	e.str(f.ID)
	e.str(f.Label)
	e.str(f.Description)
	switch k := f.Kind.(type) {
	case KindAction:
		e.tag(0)
		e.tag(uint32(k.Action.Tag))
		if k.Action.Tag == ActionCustom {
			e.str(k.Action.Custom)
		}
		e.valueType(k.Result)
	case KindLookup:
		e.tag(1)
		e.valueType(k.Result)
	case KindVerification:
		e.tag(2)
		e.valueType(k.Type)
	case KindCompletion:
		e.tag(3)
		e.valueType(k.Type)
	default:
		panic(bad("function kind", k))
	}
	e.varint(uint64(len(f.Inputs)))
	for _, in := range f.Inputs {
		e.str(in.ID)
		e.str(in.Label)
		e.str(in.Description)
		e.valueType(in.ValueType)
		e.boolean(in.Required)
		switch h := in.Hint.(type) {
		case nil, HintPlain:
			e.tag(0)
		case HintSecret:
			e.tag(1)
		case HintMultiline:
			e.tag(2)
		case HintChoices:
			e.tag(3)
			e.varint(uint64(len(h.Choices)))
			for _, c := range h.Choices {
				e.str(c.Label)
				e.value(c.Value)
			}
		default:
			panic(bad("input hint", h))
		}
		e.optStr(in.Verification)
		e.optStr(in.Completion)
	}
}

// Decoding.

func seq[T any](d *decoder, f func() T) []T {
	n := d.count()
	out := make([]T, 0, n)
	for i := 0; i < n; i++ {
		out = append(out, f())
	}
	return out
}

func (d *decoder) channelRef() ChannelRef {
	return ChannelRef{Organization: d.optStr(), Channel: d.str()}
}

func (d *decoder) message() Message {
	return Message{
		ID:       d.str(),
		Channel:  d.channelRef(),
		Author:   d.str(),
		SentAt:   d.u64(),
		ReplyTo:  d.optStr(),
		Content:  d.contents(),
		EditedAt: d.optU64(),
		Reactions: seq(d, func() Reaction {
			return Reaction{Key: d.str(), Count: d.u32(), Me: d.boolean()}
		}),
		Nonce: d.optStr(),
		Email: d.optEmail(),
	}
}

func (d *decoder) messageEvent() MessageEvent {
	switch t := d.tag(); t {
	case 0:
		return Received{d.message()}
	case 1:
		return Edited{d.channelRef(), d.str(), d.contents(), d.u64()}
	case 2:
		return Deleted{d.channelRef(), d.str()}
	case 3:
		return Reacted{d.channelRef(), d.str(), d.str(), d.str(), d.boolean()}
	case 4:
		return Read{d.channelRef(), d.str(), d.optStr()}
	case 5:
		return Typing{d.channelRef(), d.str()}
	case 6:
		return Gap{d.channelRef()}
	case 7:
		return NotSent{d.channelRef(), d.str(), d.commandError()}
	default:
		d.badTag("MessageEvent", t)
		return nil
	}
}

func (d *decoder) contents() []Content { return seq(d, d.content) }

func (d *decoder) content() Content {
	switch t := d.tag(); t {
	case 0:
		return ContentText{d.str()}
	case 1:
		return ContentFormatted{d.formatted()}
	case 2:
		return ContentImage{d.media()}
	case 3:
		return ContentVideo{d.media()}
	case 4:
		return ContentAudio{d.media()}
	case 5:
		return ContentFile{d.media()}
	case 6:
		return ContentCustom{d.str(), d.bytes()}
	default:
		d.badTag("Content", t)
		return nil
	}
}

func (d *decoder) media() Media {
	m := Media{Name: d.optStr(), Mime: d.optStr(), Size: d.optU64()}
	switch t := d.tag(); t {
	case 0:
		m.Source = SourceBytes{d.bytes()}
	case 1:
		m.Source = SourceURL{d.str()}
	case 2:
		m.Source = SourceBlob{d.str()}
	default:
		d.badTag("MediaSource", t)
	}
	m.Metadata = MediaMetadata{d.optU32(), d.optU32(), d.optU64(), d.optU64(), d.optStr(), d.optStr(), d.boolean()}
	return m
}

func (d *decoder) optMedia() *Media {
	if d.option() {
		m := d.media()
		return &m
	}
	return nil
}

func (d *decoder) contentKind() ContentKind {
	t := ContentKindTag(d.tag())
	switch {
	case t == KindCustom:
		return ContentKind{t, d.str()}
	case t > KindCustom:
		d.badTag("ContentKind", uint32(t))
	}
	return ContentKind{Tag: t}
}

func (d *decoder) formatted() FormattedText {
	return FormattedText{d.str(), seq(d, func() Span { return Span{d.u32(), d.u32(), d.style()} })}
}

func (d *decoder) style() Style {
	switch t := d.tag(); t {
	case 0:
		return StyleBold{}
	case 1:
		return StyleItalic{}
	case 2:
		return StyleUnderline{}
	case 3:
		return StyleStrikethrough{}
	case 4:
		return StyleCode{}
	case 5:
		return StyleCodeBlock{d.optStr()}
	case 6:
		return StyleQuote{}
	case 7:
		return StyleSpoiler{}
	case 8:
		return StyleLink{d.str()}
	case 9:
		switch m := d.tag(); m {
		case 0:
			return StyleMention{MentionUser{d.str()}}
		case 1:
			return StyleMention{MentionChannel{d.str()}}
		case 2:
			return StyleMention{MentionRole{d.str()}}
		case 3:
			return StyleMention{MentionEveryone{}}
		default:
			d.badTag("Mention", m)
		}
	default:
		d.badTag("Style", t)
	}
	return nil
}

func (d *decoder) user() User {
	return User{ID: d.str(), Name: d.str(), Username: d.optStr(), Avatar: d.optMedia()}
}

func (d *decoder) channel() Channel {
	c := Channel{ID: d.str(), Name: d.str(), Topic: d.optStr(), Icon: d.optMedia()}
	c.Kind.Tag = ChannelKindTag(d.tag())
	switch {
	case c.Kind.Tag == ChannelCustom:
		c.Kind.Custom = d.str()
	case c.Kind.Tag > ChannelCustom:
		d.badTag("ChannelKind", uint32(c.Kind.Tag))
	}
	c.Parent = d.optStr()
	c.Position = d.optU32()
	if d.option() {
		m := seq(d, d.str)
		c.Members = &m
	}
	c.AcceptedContent = seq(d, func() ContentRule {
		r := ContentRule{Kind: d.contentKind(), MaxSize: d.optU64(), TextUnit: TextUnit(d.tag())}
		if r.TextUnit > UnitBytes {
			d.badTag("TextUnit", uint32(r.TextUnit))
		}
		r.MimeTypes = seq(d, d.str)
		return r
	})
	c.Limits = MessageLimits{d.optU32(), d.optU64()}
	if d.option() {
		c.Mailbox = &Mailbox{d.optStr(), seq(d, d.str)}
	}
	return c
}

func (d *decoder) organization() Organization {
	return Organization{ID: d.str(), Name: d.str(), Icon: d.optMedia(), Me: d.optStr(), Users: seq(d, d.user), Channels: seq(d, d.channel)}
}

func (d *decoder) violation() Violation {
	switch t := d.tag(); t {
	case 0:
		return ViolationEmpty{}
	case 1:
		return ViolationUnsupported{d.u32(), d.contentKind()}
	case 2:
		return ViolationUnsupportedMime{d.u32(), d.optStr()}
	case 3:
		return ViolationTextTooLong{d.u64(), d.u64()}
	case 4:
		return ViolationInvalidFormatting{d.u32()}
	case 5:
		return ViolationTooLarge{d.u32(), d.u64(), d.u64()}
	case 6:
		return ViolationTooManyAttachments{d.u32(), d.u32()}
	case 7:
		return ViolationTotalTooLarge{d.u64(), d.u64()}
	default:
		d.badTag("Violation", t)
		return nil
	}
}

func (d *decoder) directoryUpdate() DirectoryUpdate {
	switch t := d.tag(); t {
	case 0:
		return OrganizationUpserted{d.organization()}
	case 1:
		return OrganizationUpdated{d.str(), d.str(), d.optMedia()}
	case 2:
		return OrganizationRemoved{d.str()}
	case 3:
		return UserUpserted{d.optStr(), d.user()}
	case 4:
		return UserRemoved{d.optStr(), d.str()}
	case 5:
		return ChannelUpserted{d.optStr(), d.channel()}
	case 6:
		return ChannelRemoved{d.channelRef()}
	case 7:
		return Me{d.optStr(), d.optStr()}
	case 8:
		u := PresenceChanged{Organization: d.optStr(), User: d.str(), Presence: Presence(d.tag())}
		if u.Presence > Offline {
			d.badTag("Presence", uint32(u.Presence))
		}
		return u
	case 9:
		return DirectoryReset{}
	default:
		d.badTag("DirectoryUpdate", t)
		return nil
	}
}

func (d *decoder) status() ProviderStatus {
	switch t := d.tag(); t {
	case 0:
		return StatusConnecting{}
	case 1:
		return StatusLoginRequired{LoginStep{d.str(), d.optStr(), d.optStr(), d.optStr()}}
	case 2:
		return StatusSyncing{}
	case 3:
		return StatusReady{}
	case 4:
		return StatusDisconnected{d.str()}
	case 5:
		return StatusFailed{d.str()}
	default:
		d.badTag("ProviderStatus", t)
		return nil
	}
}

func (d *decoder) inputRef() *FunctionInputRef {
	if !d.option() {
		return nil
	}
	return &FunctionInputRef{d.str(), d.str()}
}

func (d *decoder) command() Command {
	switch t := d.tag(); t {
	case 0:
		return CmdFunctions{d.u64(), d.functionContext()}
	case 1:
		return CmdCall{d.u64(), d.functionCall()}
	case 2:
		return CmdVerify{d.u64(), VerificationRequest{d.functionCall(), d.inputRef(), d.value()}}
	case 3:
		return CmdComplete{d.u64(), CompletionRequest{d.functionCall(), d.inputRef(), d.str(), d.optStr(), d.u32()}}
	case 4:
		return CmdSend{d.u64(), d.channelRef(), d.optStr(), d.contents(), d.optStr()}
	case 5:
		return CmdHistory{d.u64(), d.channelRef(), d.optStr(), d.u32()}
	case 6:
		return CmdGetMessage{d.u64(), d.channelRef(), d.str()}
	case 7:
		return CmdOpenChannel{d.u64(), d.optStr(), seq(d, d.str)}
	case 8:
		return CmdReadBlob{d.u64(), d.str(), d.u64(), d.u32()}
	case 9:
		return CmdReleaseBlob{d.str()}
	case 10:
		return CmdCancel{d.u64()}
	case 11:
		return CmdShutdown{}
	default:
		d.badTag("Command", t)
		return nil
	}
}

func (d *decoder) issues() []InputIssue {
	return seq(d, func() InputIssue { return InputIssue{d.optStr(), d.str(), d.str()} })
}

func (d *decoder) verification() Verification {
	switch t := d.tag(); t {
	case 0:
		return Valid{}
	case 1:
		return Invalid{d.issues()}
	default:
		d.badTag("Verification", t)
		return nil
	}
}

func (d *decoder) reply() Reply {
	switch t := d.tag(); t {
	case 0:
		return ReplyFunctions{seq(d, d.function)}
	case 1:
		return ReplyValue{d.value()}
	case 2:
		return ReplyVerified{d.verification()}
	case 3:
		return ReplyCompleted{CompletionPage{
			seq(d, func() CompletionItem { return CompletionItem{d.str(), d.optStr(), d.value()} }),
			d.optStr(),
		}}
	case 4:
		return ReplySent{d.str()}
	case 5:
		return ReplyOpened{d.channelRef()}
	case 6:
		return ReplyHistory{HistoryPage{seq(d, d.message), d.optStr()}}
	case 7:
		return ReplyMessage{d.message()}
	case 8:
		return ReplyBlob{d.bytes()}
	default:
		d.badTag("Reply", t)
		return nil
	}
}

func (d *decoder) commandError() CommandError {
	switch t := d.tag(); t {
	case 0:
		return ErrUnsupported{}
	case 1:
		return ErrUnknownFunction{d.str()}
	case 2:
		return ErrInvalidInput{d.issues()}
	case 3:
		e := ErrProvider{Code: d.str(), Message: d.str()}
		if d.option() {
			e.Details = d.value()
		}
		return e
	case 4:
		return ErrUnknownOrganization{d.str()}
	case 5:
		return ErrUnknownChannel{d.channelRef()}
	case 6:
		return ErrUnknownUser{d.str()}
	case 7:
		return ErrUnknownMessage{d.str()}
	case 8:
		return ErrUnknownBlob{d.str()}
	case 9:
		return ErrRejected{d.violation()}
	case 10:
		return ErrRateLimited{d.optU64()}
	case 11:
		return ErrForbidden{d.str()}
	case 12:
		return ErrLoginRequired{}
	case 13:
		return ErrCancelled{}
	case 14:
		return ErrFailed{d.str()}
	default:
		d.badTag("CommandError", t)
		return nil
	}
}

func (d *decoder) providerMessage() ProviderMessage {
	switch t := d.tag(); t {
	case 0:
		m := PmLog{Level: LogLevel(d.tag())}
		if m.Level > LogTrace {
			d.badTag("LogLevel", uint32(m.Level))
		}
		m.Message = d.str()
		return m
	case 1:
		return PmMessage{d.messageEvent()}
	case 2:
		return PmStatus{d.status()}
	case 3:
		return PmSecretGet{d.str()}
	case 4:
		return PmSecretSet{d.str(), d.bytes()}
	case 5:
		return PmSecretDelete{d.str()}
	case 6:
		return PmDirectory{d.directoryUpdate()}
	case 7:
		m := PmReply{Request: d.u64()}
		switch r := d.tag(); r {
		case 0:
			m.Reply = d.reply()
		case 1:
			m.Err = d.commandError()
		default:
			d.badTag("Result", r)
		}
		return m
	case 8:
		return PmReadBlob{d.str(), d.u64(), d.u32()}
	case 9:
		return PmLookup{d.str()}
	case 10:
		return PmBrowserLogin{d.u64(), d.str()}
	case 11:
		return PmCancelBrowserLogin{d.u64()}
	default:
		d.badTag("ProviderMessage", t)
		return nil
	}
}

// result reads a Result's tag, and the error if it failed.
func (d *decoder) result() *string {
	switch t := d.tag(); t {
	case 0:
		return nil
	case 1:
		s := d.str()
		return &s
	default:
		d.badTag("Result", t)
		return nil
	}
}

func (d *decoder) hostMessage() HostMessage {
	switch t := d.tag(); t {
	case 0:
		return HmCommand{d.command()}
	case 1:
		m := HmSecret{Key: d.str(), Err: d.result()}
		if m.Err == nil && d.option() {
			m.Found, m.Value = true, d.bytes()
		}
		return m
	case 2:
		return HmSecretStored{d.str(), d.result()}
	case 3:
		m := HmBlob{Blob: d.str(), Offset: d.u64(), Err: d.result()}
		if m.Err == nil {
			m.Bytes = d.bytes()
		}
		return m
	case 4:
		m := HmAddresses{Name: d.str(), Err: d.result()}
		if m.Err == nil {
			m.Addresses = seq(d, func() SocketAddr {
				var ip []byte
				switch v := d.tag(); v {
				case 0:
					ip = append([]byte{}, d.raw(4)...)
				case 1:
					ip = append([]byte{}, d.raw(16)...)
				default:
					d.badTag("SocketAddr", v)
				}
				return SocketAddr{ip, d.u16()}
			})
		}
		return m
	case 5:
		m := HmBrowserLogin{Request: d.u64(), Err: d.result()}
		if m.Err == nil {
			m.Token = d.str()
		}
		return m
	default:
		d.badTag("HostMessage", t)
		return nil
	}
}

func (d *decoder) value() Value {
	switch t := d.tag(); t {
	case 0:
		return Null{}
	case 1:
		return Boolean(d.boolean())
	case 2:
		return Integer(d.i64())
	case 3:
		return Text(d.str())
	case 4:
		return Bytes(d.bytes())
	case 5:
		return List(seq(d, d.value))
	case 6:
		return Record(d.arguments())
	case 7:
		return ContentValue(d.contents())
	case 8:
		return ChannelValue(d.channelRef())
	case 9:
		return MessageValue(d.message())
	default:
		d.badTag("Value", t)
		return nil
	}
}

func (d *decoder) arguments() Arguments {
	n := d.count()
	a := make(Arguments, n)
	for i := 0; i < n; i++ {
		k := d.str()
		a[k] = d.value()
	}
	return a
}

func (d *decoder) valueType() ValueType {
	switch t := d.tag(); t {
	case 0:
		return TypeAny{}
	case 1:
		return TypeNull{}
	case 2:
		return TypeBoolean{}
	case 3:
		return TypeInteger{}
	case 4:
		return TypeText{}
	case 5:
		return TypeBytes{}
	case 6:
		return TypeList{d.valueType()}
	case 7:
		return TypeRecord{seq(d, func() Field { return Field{d.str(), d.str(), d.valueType(), d.boolean()} })}
	case 8:
		return TypeContent{}
	case 9:
		return TypeChannel{}
	case 10:
		return TypeMessage{}
	default:
		d.badTag("ValueType", t)
		return nil
	}
}

func (d *decoder) functionContext() FunctionContext {
	switch t := d.tag(); t {
	case 0:
		return InProvider{}
	case 1:
		return InOrganization{d.str()}
	case 2:
		return InChannel{d.channelRef()}
	case 3:
		return InMessage{d.channelRef(), d.str()}
	default:
		d.badTag("FunctionContext", t)
		return nil
	}
}

func (d *decoder) functionCall() FunctionCall {
	return FunctionCall{d.str(), d.functionContext(), d.arguments()}
}

func (d *decoder) function() Function {
	f := Function{ID: d.str(), Label: d.str(), Description: d.str()}
	switch t := d.tag(); t {
	case 0:
		a := ActionKind{Tag: ActionTag(d.tag())}
		switch {
		case a.Tag == ActionCustom:
			a.Custom = d.str()
		case a.Tag > ActionCustom:
			d.badTag("ActionKind", uint32(a.Tag))
		}
		f.Kind = KindAction{a, d.valueType()}
	case 1:
		f.Kind = KindLookup{d.valueType()}
	case 2:
		f.Kind = KindVerification{d.valueType()}
	case 3:
		f.Kind = KindCompletion{d.valueType()}
	default:
		d.badTag("FunctionKind", t)
	}
	f.Inputs = seq(d, func() FunctionInput {
		in := FunctionInput{ID: d.str(), Label: d.str(), Description: d.str(), ValueType: d.valueType(), Required: d.boolean()}
		switch h := d.tag(); h {
		case 0:
			in.Hint = HintPlain{}
		case 1:
			in.Hint = HintSecret{}
		case 2:
			in.Hint = HintMultiline{}
		case 3:
			in.Hint = HintChoices{seq(d, func() Choice { return Choice{d.str(), d.value()} })}
		default:
			d.badTag("InputHint", h)
		}
		in.Verification = d.optStr()
		in.Completion = d.optStr()
		return in
	})
	return f
}
