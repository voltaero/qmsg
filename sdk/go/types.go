package qmsg

// Go mirrors of the qmsg-types protocol. Field and variant order is the wire
// format, so it must follow crates/qmsg-types exactly; the golden fixtures in
// testdata check that. Enums with data are interfaces with one struct per
// variant; enums without are integer constants.

// ABIVersion must equal qmsg_types::ABI_VERSION.
const ABIVersion = 3

const (
	MaxFrame        = 64 << 20
	MaxRead         = 4 << 20
	MaxBlobReads    = 4
	MaxLookups      = 4
	MaxID           = 1024
	TypingTimeoutMs = 10_000
)

type ProviderConfig struct {
	Name     string
	HostURL  string
	Settings map[string]string
}

type ChannelRef struct {
	Organization *string
	Channel      string
}

type Message struct {
	ID        string
	Channel   ChannelRef
	Author    string
	SentAt    uint64
	ReplyTo   *string
	Content   []Content
	EditedAt  *uint64
	Reactions []Reaction
	Nonce     *string
}

type Reaction struct {
	Key   string
	Count uint32
	Me    bool
}

type MessageEvent interface{ isMessageEvent() }

type Received struct{ Message Message }
type Edited struct {
	Channel  ChannelRef
	ID       string
	Content  []Content
	EditedAt uint64
}
type Deleted struct {
	Channel ChannelRef
	ID      string
}
type Reacted struct {
	Channel ChannelRef
	ID      string
	User    string
	Key     string
	Added   bool
}
type Read struct {
	Channel ChannelRef
	User    string
	UpTo    *string
}
type Typing struct {
	Channel ChannelRef
	User    string
}
type Gap struct{ Channel ChannelRef }
type NotSent struct {
	Channel ChannelRef
	Nonce   string
	Error   CommandError
}

func (Received) isMessageEvent() {}
func (Edited) isMessageEvent()   {}
func (Deleted) isMessageEvent()  {}
func (Reacted) isMessageEvent()  {}
func (Read) isMessageEvent()     {}
func (Typing) isMessageEvent()   {}
func (Gap) isMessageEvent()      {}
func (NotSent) isMessageEvent()  {}

type Content interface{ isContent() }

type ContentText struct{ Text string }
type ContentFormatted struct{ Text FormattedText }
type ContentImage struct{ Media Media }
type ContentVideo struct{ Media Media }
type ContentAudio struct{ Media Media }
type ContentFile struct{ Media Media }
type ContentCustom struct {
	Kind string
	Data []byte
}

func (ContentText) isContent()      {}
func (ContentFormatted) isContent() {}
func (ContentImage) isContent()     {}
func (ContentVideo) isContent()     {}
func (ContentAudio) isContent()     {}
func (ContentFile) isContent()      {}
func (ContentCustom) isContent()    {}

type Media struct {
	Name   *string
	Mime   *string
	Size   *uint64
	Source MediaSource
}

type MediaSource interface{ isMediaSource() }

type SourceBytes struct{ Bytes []byte }
type SourceURL struct{ URL string }
type SourceBlob struct{ Blob string }

func (SourceBytes) isMediaSource() {}
func (SourceURL) isMediaSource()   {}
func (SourceBlob) isMediaSource()  {}

// ContentKind is a kind of Content; Custom is set only for KindCustom.
type ContentKind struct {
	Tag    ContentKindTag
	Custom string
}

type ContentKindTag uint32

const (
	KindText ContentKindTag = iota
	KindFormatted
	KindImage
	KindVideo
	KindAudio
	KindFile
	KindCustom
)

type FormattedText struct {
	Text  string
	Spans []Span
}

type Span struct {
	Start uint32
	End   uint32
	Style Style
}

type Style interface{ isStyle() }

type StyleBold struct{}
type StyleItalic struct{}
type StyleUnderline struct{}
type StyleStrikethrough struct{}
type StyleCode struct{}
type StyleCodeBlock struct{ Language *string }
type StyleQuote struct{}
type StyleSpoiler struct{}
type StyleLink struct{ URL string }
type StyleMention struct{ Mention Mention }

func (StyleBold) isStyle()          {}
func (StyleItalic) isStyle()        {}
func (StyleUnderline) isStyle()     {}
func (StyleStrikethrough) isStyle() {}
func (StyleCode) isStyle()          {}
func (StyleCodeBlock) isStyle()     {}
func (StyleQuote) isStyle()         {}
func (StyleSpoiler) isStyle()       {}
func (StyleLink) isStyle()          {}
func (StyleMention) isStyle()       {}

type Mention interface{ isMention() }

type MentionUser struct{ ID string }
type MentionChannel struct{ ID string }
type MentionRole struct{ ID string }
type MentionEveryone struct{}

func (MentionUser) isMention()     {}
func (MentionChannel) isMention()  {}
func (MentionRole) isMention()     {}
func (MentionEveryone) isMention() {}

type Organization struct {
	ID       string
	Name     string
	Me       *string
	Users    []User
	Channels []Channel
}

type User struct {
	ID   string
	Name string
}

type Channel struct {
	ID              string
	Name            string
	Kind            ChannelKind
	Parent          *string
	Position        *uint32
	Members         *[]string
	AcceptedContent []ContentRule
	Limits          MessageLimits
}

// ChannelKind is a kind of channel; Custom is set only for ChannelCustom.
type ChannelKind struct {
	Tag    ChannelKindTag
	Custom string
}

type ChannelKindTag uint32

const (
	ChannelText ChannelKindTag = iota
	ChannelVoice
	ChannelVideo
	ChannelAnnouncement
	ChannelForum
	ChannelThread
	ChannelCategory
	ChannelDirect
	ChannelGroup
	ChannelCustom
)

type ContentRule struct {
	Kind      ContentKind
	MaxSize   *uint64
	TextUnit  TextUnit
	MimeTypes []string
}

type TextUnit uint32

const (
	UnitChars TextUnit = iota
	UnitUTF16
	UnitBytes
)

type MessageLimits struct {
	MaxAttachments *uint32
	MaxTotalSize   *uint64
}

type Violation interface{ isViolation() }

type ViolationEmpty struct{}
type ViolationUnsupported struct {
	Part uint32
	Kind ContentKind
}
type ViolationUnsupportedMime struct {
	Part uint32
	Mime *string
}
type ViolationTextTooLong struct{ Length, Max uint64 }
type ViolationInvalidFormatting struct{ Part uint32 }
type ViolationTooLarge struct {
	Part      uint32
	Size, Max uint64
}
type ViolationTooManyAttachments struct{ Count, Max uint32 }
type ViolationTotalTooLarge struct{ Size, Max uint64 }

func (ViolationEmpty) isViolation()              {}
func (ViolationUnsupported) isViolation()        {}
func (ViolationUnsupportedMime) isViolation()    {}
func (ViolationTextTooLong) isViolation()        {}
func (ViolationInvalidFormatting) isViolation()  {}
func (ViolationTooLarge) isViolation()           {}
func (ViolationTooManyAttachments) isViolation() {}
func (ViolationTotalTooLarge) isViolation()      {}

type DirectoryUpdate interface{ isDirectoryUpdate() }

type OrganizationUpserted struct{ Organization Organization }
type OrganizationUpdated struct{ ID, Name string }
type OrganizationRemoved struct{ ID string }
type UserUpserted struct {
	Organization *string
	User         User
}
type UserRemoved struct {
	Organization *string
	ID           string
}
type ChannelUpserted struct {
	Organization *string
	Channel      Channel
}
type ChannelRemoved struct{ Channel ChannelRef }
type Me struct {
	Organization *string
	ID           *string
}
type PresenceChanged struct {
	Organization *string
	User         string
	Presence     Presence
}
type DirectoryReset struct{}

func (OrganizationUpserted) isDirectoryUpdate() {}
func (OrganizationUpdated) isDirectoryUpdate()  {}
func (OrganizationRemoved) isDirectoryUpdate()  {}
func (UserUpserted) isDirectoryUpdate()         {}
func (UserRemoved) isDirectoryUpdate()          {}
func (ChannelUpserted) isDirectoryUpdate()      {}
func (ChannelRemoved) isDirectoryUpdate()       {}
func (Me) isDirectoryUpdate()                   {}
func (PresenceChanged) isDirectoryUpdate()      {}
func (DirectoryReset) isDirectoryUpdate()       {}

type Presence uint32

const (
	Online Presence = iota
	Idle
	Busy
	Offline
)

type ProviderStatus interface{ isProviderStatus() }

type StatusConnecting struct{}
type StatusLoginRequired struct{ Step LoginStep }
type StatusSyncing struct{}
type StatusReady struct{}
type StatusDisconnected struct{ Reason string }
type StatusFailed struct{ Reason string }

func (StatusConnecting) isProviderStatus()    {}
func (StatusLoginRequired) isProviderStatus() {}
func (StatusSyncing) isProviderStatus()       {}
func (StatusReady) isProviderStatus()         {}
func (StatusDisconnected) isProviderStatus()  {}
func (StatusFailed) isProviderStatus()        {}

type LoginStep struct {
	Message  string
	Function *string
	Link     *string
	QRCode   *string
}

type Command interface{ isCommand() }

type CmdFunctions struct {
	Request uint64
	Context FunctionContext
}
type CmdCall struct {
	Request uint64
	Call    FunctionCall
}
type CmdVerify struct {
	Request      uint64
	Verification VerificationRequest
}
type CmdComplete struct {
	Request    uint64
	Completion CompletionRequest
}
type CmdSend struct {
	Request uint64
	Channel ChannelRef
	ReplyTo *string
	Content []Content
	Nonce   *string
}
type CmdHistory struct {
	Request uint64
	Channel ChannelRef
	Cursor  *string
	Limit   uint32
}
type CmdGetMessage struct {
	Request uint64
	Channel ChannelRef
	ID      string
}
type CmdOpenChannel struct {
	Request      uint64
	Organization *string
	Members      []string
}
type CmdReadBlob struct {
	Request uint64
	Blob    string
	Offset  uint64
	Len     uint32
}
type CmdReleaseBlob struct{ Blob string }
type CmdCancel struct{ Request uint64 }
type CmdShutdown struct{}

func (CmdFunctions) isCommand()   {}
func (CmdCall) isCommand()        {}
func (CmdVerify) isCommand()      {}
func (CmdComplete) isCommand()    {}
func (CmdSend) isCommand()        {}
func (CmdHistory) isCommand()     {}
func (CmdGetMessage) isCommand()  {}
func (CmdOpenChannel) isCommand() {}
func (CmdReadBlob) isCommand()    {}
func (CmdReleaseBlob) isCommand() {}
func (CmdCancel) isCommand()      {}
func (CmdShutdown) isCommand()    {}

// RequestOf returns the request id a command is answered by, if any.
func RequestOf(c Command) (uint64, bool) {
	switch c := c.(type) {
	case CmdFunctions:
		return c.Request, true
	case CmdCall:
		return c.Request, true
	case CmdVerify:
		return c.Request, true
	case CmdComplete:
		return c.Request, true
	case CmdSend:
		return c.Request, true
	case CmdHistory:
		return c.Request, true
	case CmdGetMessage:
		return c.Request, true
	case CmdOpenChannel:
		return c.Request, true
	case CmdReadBlob:
		return c.Request, true
	}
	return 0, false
}

type HistoryPage struct {
	Messages   []Message
	NextCursor *string
}

type Reply interface{ isReply() }

type ReplyFunctions struct{ Functions []Function }
type ReplyValue struct{ Value Value }
type ReplyVerified struct{ Verification Verification }
type ReplyCompleted struct{ Page CompletionPage }
type ReplySent struct{ ID string }
type ReplyOpened struct{ Channel ChannelRef }
type ReplyHistory struct{ Page HistoryPage }
type ReplyMessage struct{ Message Message }
type ReplyBlob struct{ Bytes []byte }

func (ReplyFunctions) isReply() {}
func (ReplyValue) isReply()     {}
func (ReplyVerified) isReply()  {}
func (ReplyCompleted) isReply() {}
func (ReplySent) isReply()      {}
func (ReplyOpened) isReply()    {}
func (ReplyHistory) isReply()   {}
func (ReplyMessage) isReply()   {}
func (ReplyBlob) isReply()      {}

// CommandError is the error half of a reply. It is also a Go error.
type CommandError interface {
	error
	isCommandError()
}

type ErrUnsupported struct{}
type ErrUnknownFunction struct{ ID string }
type ErrInvalidInput struct{ Issues []InputIssue }
type ErrProvider struct {
	Code    string
	Message string
	Details Value // nil for none
}
type ErrUnknownOrganization struct{ ID string }
type ErrUnknownChannel struct{ Channel ChannelRef }
type ErrUnknownUser struct{ ID string }
type ErrUnknownMessage struct{ ID string }
type ErrUnknownBlob struct{ ID string }
type ErrRejected struct{ Violation Violation }
type ErrRateLimited struct{ RetryAfterMs *uint64 }
type ErrForbidden struct{ Reason string }
type ErrLoginRequired struct{}
type ErrCancelled struct{}
type ErrFailed struct{ Reason string }

func (ErrUnsupported) isCommandError()         {}
func (ErrUnknownFunction) isCommandError()     {}
func (ErrInvalidInput) isCommandError()        {}
func (ErrProvider) isCommandError()            {}
func (ErrUnknownOrganization) isCommandError() {}
func (ErrUnknownChannel) isCommandError()      {}
func (ErrUnknownUser) isCommandError()         {}
func (ErrUnknownMessage) isCommandError()      {}
func (ErrUnknownBlob) isCommandError()         {}
func (ErrRejected) isCommandError()            {}
func (ErrRateLimited) isCommandError()         {}
func (ErrForbidden) isCommandError()           {}
func (ErrLoginRequired) isCommandError()       {}
func (ErrCancelled) isCommandError()           {}
func (ErrFailed) isCommandError()              {}

func (ErrUnsupported) Error() string           { return "unsupported" }
func (e ErrUnknownFunction) Error() string     { return "unknown function " + e.ID }
func (ErrInvalidInput) Error() string          { return "invalid input" }
func (e ErrProvider) Error() string            { return e.Code + ": " + e.Message }
func (e ErrUnknownOrganization) Error() string { return "unknown organization " + e.ID }
func (e ErrUnknownChannel) Error() string      { return "unknown channel " + e.Channel.Channel }
func (e ErrUnknownUser) Error() string         { return "unknown user " + e.ID }
func (e ErrUnknownMessage) Error() string      { return "unknown message " + e.ID }
func (e ErrUnknownBlob) Error() string         { return "unknown blob " + e.ID }
func (ErrRejected) Error() string              { return "rejected by the channel's limits" }
func (ErrRateLimited) Error() string           { return "rate limited" }
func (e ErrForbidden) Error() string           { return "forbidden: " + e.Reason }
func (ErrLoginRequired) Error() string         { return "login required" }
func (ErrCancelled) Error() string             { return "cancelled" }
func (e ErrFailed) Error() string              { return e.Reason }

type LogLevel uint32

const (
	LogError LogLevel = iota
	LogWarn
	LogInfo
	LogDebug
	LogTrace
)

type ProviderMessage interface{ isProviderMessage() }

type PmLog struct {
	Level   LogLevel
	Message string
}
type PmMessage struct{ Event MessageEvent }
type PmStatus struct{ Status ProviderStatus }
type PmSecretGet struct{ Key string }
type PmSecretSet struct {
	Key   string
	Value []byte
}
type PmSecretDelete struct{ Key string }
type PmDirectory struct{ Update DirectoryUpdate }

// PmReply answers a request: Err is set on failure, otherwise Reply.
type PmReply struct {
	Request uint64
	Reply   Reply
	Err     CommandError
}
type PmReadBlob struct {
	Blob   string
	Offset uint64
	Len    uint32
}
type PmLookup struct{ Name string }

// PmBrowserLogin asks the host to sign in with its browser recipe for
// Provider; answered with HmBrowserLogin. See Context.BrowserLogin.
type PmBrowserLogin struct {
	Request  uint64
	Provider string
}
type PmCancelBrowserLogin struct{ Request uint64 }

func (PmLog) isProviderMessage()                {}
func (PmMessage) isProviderMessage()            {}
func (PmStatus) isProviderMessage()             {}
func (PmSecretGet) isProviderMessage()          {}
func (PmSecretSet) isProviderMessage()          {}
func (PmSecretDelete) isProviderMessage()       {}
func (PmDirectory) isProviderMessage()          {}
func (PmReply) isProviderMessage()              {}
func (PmReadBlob) isProviderMessage()           {}
func (PmLookup) isProviderMessage()             {}
func (PmBrowserLogin) isProviderMessage()       {}
func (PmCancelBrowserLogin) isProviderMessage() {}

type HostMessage interface{ isHostMessage() }

type HmCommand struct{ Command Command }

// HmSecret answers PmSecretGet: Err is set when the secret can't be read,
// otherwise Value is nil when it was never set.
type HmSecret struct {
	Key   string
	Value []byte
	Found bool
	Err   *string
}
type HmSecretStored struct {
	Key string
	Err *string
}
type HmBlob struct {
	Blob   string
	Offset uint64
	Bytes  []byte
	Err    *string
}
type HmAddresses struct {
	Name      string
	Addresses []SocketAddr
	Err       *string
}

// HmBrowserLogin answers PmBrowserLogin: Err is set on failure, otherwise
// Token holds the session token.
type HmBrowserLogin struct {
	Request uint64
	Token   string
	Err     *string
}

// SocketAddr is a std::net::SocketAddr: IP is 4 or 16 bytes.
type SocketAddr struct {
	IP   []byte
	Port uint16
}

func (HmCommand) isHostMessage()      {}
func (HmSecret) isHostMessage()       {}
func (HmSecretStored) isHostMessage() {}
func (HmBlob) isHostMessage()         {}
func (HmAddresses) isHostMessage()    {}
func (HmBrowserLogin) isHostMessage() {}

// Arguments are named function inputs.
type Arguments = map[string]Value

type Value interface{ isValue() }

type Null struct{}
type Boolean bool
type Integer int64
type Text string
type Bytes []byte
type List []Value
type Record Arguments
type ContentValue []Content
type ChannelValue ChannelRef
type MessageValue Message

func (Null) isValue()         {}
func (Boolean) isValue()      {}
func (Integer) isValue()      {}
func (Text) isValue()         {}
func (Bytes) isValue()        {}
func (List) isValue()         {}
func (Record) isValue()       {}
func (ContentValue) isValue() {}
func (ChannelValue) isValue() {}
func (MessageValue) isValue() {}

type ValueType interface{ isValueType() }

type TypeAny struct{}
type TypeNull struct{}
type TypeBoolean struct{}
type TypeInteger struct{}
type TypeText struct{}
type TypeBytes struct{}
type TypeList struct{ Item ValueType }
type TypeRecord struct{ Fields []Field }
type TypeContent struct{}
type TypeChannel struct{}
type TypeMessage struct{}

func (TypeAny) isValueType()     {}
func (TypeNull) isValueType()    {}
func (TypeBoolean) isValueType() {}
func (TypeInteger) isValueType() {}
func (TypeText) isValueType()    {}
func (TypeBytes) isValueType()   {}
func (TypeList) isValueType()    {}
func (TypeRecord) isValueType()  {}
func (TypeContent) isValueType() {}
func (TypeChannel) isValueType() {}
func (TypeMessage) isValueType() {}

type Field struct {
	ID        string
	Label     string
	ValueType ValueType
	Required  bool
}

type FunctionContext interface{ isFunctionContext() }

type InProvider struct{}
type InOrganization struct{ ID string }
type InChannel struct{ Channel ChannelRef }
type InMessage struct {
	Channel ChannelRef
	ID      string
}

func (InProvider) isFunctionContext()     {}
func (InOrganization) isFunctionContext() {}
func (InChannel) isFunctionContext()      {}
func (InMessage) isFunctionContext()      {}

// ActionKind is a standard action; Custom is set only for ActionCustom.
type ActionKind struct {
	Tag    ActionTag
	Custom string
}

type ActionTag uint32

const (
	ActionSendMessage ActionTag = iota
	ActionEditMessage
	ActionDeleteMessage
	ActionOpenChannel
	ActionAddReaction
	ActionRemoveReaction
	ActionMarkRead
	ActionMarkUnread
	ActionTyping
	ActionLogin
	ActionLogout
	ActionCustom
)

type FunctionKind interface{ isFunctionKind() }

type KindAction struct {
	Action ActionKind
	Result ValueType
}
type KindLookup struct{ Result ValueType }
type KindVerification struct{ Type ValueType }
type KindCompletion struct{ Type ValueType }

func (KindAction) isFunctionKind()       {}
func (KindLookup) isFunctionKind()       {}
func (KindVerification) isFunctionKind() {}
func (KindCompletion) isFunctionKind()   {}

type InputHint interface{ isInputHint() }

type HintPlain struct{}
type HintSecret struct{}
type HintMultiline struct{}
type HintChoices struct{ Choices []Choice }

func (HintPlain) isInputHint()     {}
func (HintSecret) isInputHint()    {}
func (HintMultiline) isInputHint() {}
func (HintChoices) isInputHint()   {}

type Choice struct {
	Label string
	Value Value
}

type FunctionInput struct {
	ID           string
	Label        string
	Description  string
	ValueType    ValueType
	Required     bool
	Hint         InputHint
	Verification *string
	Completion   *string
}

type Function struct {
	ID          string
	Label       string
	Description string
	Kind        FunctionKind
	Inputs      []FunctionInput
}

type FunctionCall struct {
	Function  string
	Context   FunctionContext
	Arguments Arguments
}

type FunctionInputRef struct {
	Function string
	Input    string
}

type VerificationRequest struct {
	Call  FunctionCall
	Input *FunctionInputRef
	Value Value
}

type CompletionRequest struct {
	Call   FunctionCall
	Input  *FunctionInputRef
	Query  string
	Cursor *string
	Limit  uint32
}

type InputIssue struct {
	Input   *string
	Code    string
	Message string
}

type Verification interface{ isVerification() }

type Valid struct{}
type Invalid struct{ Issues []InputIssue }

func (Valid) isVerification()   {}
func (Invalid) isVerification() {}

type CompletionItem struct {
	Label       string
	Description *string
	Value       Value
}

type CompletionPage struct {
	Items      []CompletionItem
	NextCursor *string
}

// Standard input ids, as in qmsg_types::inputs.
const (
	InputContent  = "content"
	InputReplyTo  = "reply_to"
	InputNonce    = "nonce"
	InputMembers  = "members"
	InputReaction = "reaction"
)
