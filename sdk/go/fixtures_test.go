package qmsg

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// These build the same values as crates/qmsg-types/tests/go_fixtures.rs,
// whose golden encodings are in testdata/fixtures.

func guildChannel() ChannelRef { return ChannelRef{Str("g"), "c"} }
func dmRef() ChannelRef        { return ChannelRef{nil, "dm"} }

func fixMedia(s MediaSource) Media {
	return Media{Name: Str("a.png"), Mime: Str("image/png"), Size: U64(1 << 40), Source: s, Metadata: MediaMetadata{Width: U32(640), Height: U32(480), DurationMs: U64(1234), ExpiresAtMs: U64(1_700_000_000_000), PreviewURL: Str("https://cdn/preview"), Description: Str("an image"), Spoiler: true}}
}

func allContent() []Content {
	styles := []Style{
		StyleBold{}, StyleItalic{}, StyleUnderline{}, StyleStrikethrough{}, StyleCode{},
		StyleCodeBlock{Str("rs")}, StyleCodeBlock{nil}, StyleQuote{}, StyleSpoiler{},
		StyleLink{"https://x"},
		StyleMention{MentionUser{"u"}}, StyleMention{MentionChannel{"c"}},
		StyleMention{MentionRole{"r"}}, StyleMention{MentionEveryone{}},
	}
	var spans []Span
	for i, s := range styles {
		spans = append(spans, Span{uint32(i), 200 + uint32(i), s})
	}
	return []Content{
		ContentText{"héllo 😀"},
		ContentFormatted{FormattedText{"styled", spans}},
		ContentImage{fixMedia(SourceBytes{[]byte{0, 1, 255}})},
		ContentVideo{Media{Source: SourceURL{"https://cdn/v"}}},
		ContentAudio{fixMedia(SourceBlob{"blob-1"})},
		ContentFile{fixMedia(SourceURL{"https://cdn/f"})},
		ContentCustom{"sticker", bytes.Repeat([]byte{9}, 130)},
	}
}

func fixMessage() Message {
	return Message{
		ID: "m1", Channel: guildChannel(), Author: "u1", SentAt: 1_700_000_000_000,
		ReplyTo: Str("m0"), Content: allContent(), EditedAt: U64(1_700_000_000_500),
		Reactions: []Reaction{{"👍", 3, true}, {"custom:1", 70000, false}},
		Nonce:     Str("n1"),
		Email:     &Email{Subject: "Mail 😀", From: []EmailAddress{{"a@example.test", Str("Alice")}}, To: []EmailAddress{{"b@example.test", nil}}, MessageID: Str("m@example.test"), InReplyTo: Str("old@example.test"), References: []string{"old@example.test"}, HTML: Str("<p>Hello</p>"), ContentIDs: map[string]string{"2": "image@example.test"}, Read: true, Draft: true},
	}
}

func plainMessage() Message {
	return Message{ID: "m2", Channel: dmRef(), Author: "u2"}
}

func allValues() Arguments {
	return Arguments{
		"a": Null{}, "b": Boolean(true), "c": Integer(-5),
		"d": Integer(-1 << 63), "d2": Integer(1<<63 - 1), "e": Text("é"),
		"f": Bytes{0, 255}, "g": List{Integer(300), Boolean(false)},
		"h": Record{"x": Null{}}, "i": ContentValue(allContent()),
		"j": ChannelValue(dmRef()), "k": MessageValue(fixMessage()), "z": Record{},
	}
}

func allValueTypes() []ValueType {
	return []ValueType{
		TypeAny{}, TypeNull{}, TypeBoolean{}, TypeInteger{}, TypeText{}, TypeBytes{},
		TypeList{TypeList{TypeText{}}},
		TypeRecord{[]Field{{"id", "Id", TypeText{}, true}, {"n", "N", TypeRecord{}, false}}},
		TypeContent{}, TypeChannel{}, TypeMessage{},
	}
}

func fixFunctions() []Function {
	var fs []Function
	for i := ActionSendMessage; i <= ActionCustom; i++ {
		a := ActionKind{Tag: i}
		if i == ActionCustom {
			a.Custom = "pin"
		}
		fs = append(fs, Function{ID: fmt.Sprintf("a%d", i), Label: "Action", Kind: KindAction{a, TypeNull{}}})
	}
	hints := []InputHint{HintPlain{}, HintSecret{}, HintMultiline{}, HintChoices{[]Choice{{"One", Integer(1)}, {"Two", Text("2")}}}}
	var inputs []FunctionInput
	for i, t := range allValueTypes() {
		in := FunctionInput{ID: fmt.Sprintf("i%d", i), Label: "Input", Description: "d", ValueType: t, Required: i%2 == 0, Hint: hints[i%len(hints)]}
		if i%3 == 0 {
			in.Verification = Str("verify")
		}
		if i%3 == 1 {
			in.Completion = Str("complete")
		}
		inputs = append(inputs, in)
	}
	return append(fs,
		Function{"inputs", "Inputs", "every hint and type", KindLookup{TypeList{TypeText{}}}, inputs},
		Function{ID: "verify", Label: "Verify", Kind: KindVerification{TypeText{}}},
		Function{ID: "complete", Label: "Complete", Kind: KindCompletion{TypeChannel{}}},
	)
}

func fixChannel() Channel {
	return Channel{
		Mailbox: &Mailbox{Delimiter: Str("/"), Attributes: []string{"\\Inbox"}},
		ID:      "c", Name: "general", Kind: ChannelKind{Tag: ChannelText}, Parent: Str("cat"),
		Position: U32(3), Members: &[]string{"u1", "u2"},
		AcceptedContent: []ContentRule{
			{ContentKind{Tag: KindText}, U64(2000), UnitChars, nil},
			{ContentKind{Tag: KindImage}, nil, UnitUTF16, []string{"image/png", "image/*"}},
			{ContentKind{KindCustom, "sticker"}, U64(1), UnitBytes, nil},
		},
		Limits: MessageLimits{U32(10), U64(25 << 20)},
	}
}

func fixtures() map[string]any {
	f := map[string]any{}
	f["config"] = ProviderConfig{"discord", "ws://127.0.0.1:1/t", map[string]string{"b": "2", "a": "1"}}
	cmd := func(c Command) HostMessage { return HmCommand{c} }

	for i, c := range []FunctionContext{InProvider{}, InOrganization{"g"}, InChannel{guildChannel()}, InMessage{guildChannel(), "m"}} {
		f[fmt.Sprintf("host_functions_%d", i)] = cmd(CmdFunctions{uint64(i), c})
	}
	f["host_call"] = cmd(CmdCall{1<<64 - 1, FunctionCall{"f", InOrganization{"g"}, allValues()}})
	f["host_verify"] = cmd(CmdVerify{3, VerificationRequest{FunctionCall{"v", InProvider{}, Arguments{}}, &FunctionInputRef{"open", "members"}, List{Text("1")}}})
	f["host_complete"] = cmd(CmdComplete{4, CompletionRequest{FunctionCall{"c", InChannel{dmRef()}, Arguments{}}, nil, "ab", Str("next"), 25}})
	f["host_send"] = cmd(CmdSend{5, dmRef(), Str("r"), allContent(), Str("n")})
	f["host_history"] = cmd(CmdHistory{6, guildChannel(), nil, 50})
	f["host_get_message"] = cmd(CmdGetMessage{7, guildChannel(), "m"})
	f["host_open_channel"] = cmd(CmdOpenChannel{8, nil, []string{"u1", "u2"}})
	f["host_read_blob"] = cmd(CmdReadBlob{9, "b", 1 << 33, 4096})
	f["host_release_blob"] = cmd(CmdReleaseBlob{"b"})
	f["host_cancel"] = cmd(CmdCancel{300})
	f["host_shutdown"] = cmd(CmdShutdown{})
	f["host_secret_0"] = HmSecret{Key: "token", Value: []byte("token"), Found: true}
	f["host_secret_1"] = HmSecret{Key: "token"}
	f["host_secret_2"] = HmSecret{Key: "token", Err: Str("locked")}
	f["host_secret_stored_0"] = HmSecretStored{"token", nil}
	f["host_secret_stored_1"] = HmSecretStored{"token", Str("full")}
	f["host_blob_0"] = HmBlob{"b", 7, []byte{1, 2, 3}, nil}
	f["host_blob_1"] = HmBlob{"b", 7, nil, Str("gone")}
	f["host_addresses_0"] = HmAddresses{"x:1", []SocketAddr{
		{[]byte{127, 0, 0, 1}, 443},
		{[]byte{0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1}, 8080},
	}, nil}
	f["host_addresses_1"] = HmAddresses{"x:1", nil, Str("nxdomain")}
	f["host_browser_login_0"] = HmBrowserLogin{Request: 1 << 40, Token: "tok"}
	f["host_browser_login_1"] = HmBrowserLogin{Request: 2, Err: Str("cancelled")}

	for i := LogError; i <= LogTrace; i++ {
		f[fmt.Sprintf("log_%d", i)] = PmLog{i, "hi"}
	}
	events := []MessageEvent{
		Received{fixMessage()}, Received{plainMessage()},
		Edited{guildChannel(), "m", []Content{ContentText{"new"}}, 5},
		Deleted{dmRef(), "m"},
		Reacted{dmRef(), "m", "u", "👍", true},
		Read{dmRef(), "u", Str("m")}, Read{dmRef(), "u", nil},
		Typing{guildChannel(), "u"}, Gap{guildChannel()},
		NotSent{dmRef(), "n", ErrCancelled{}},
	}
	for i, e := range events {
		f[fmt.Sprintf("event_%d", i)] = PmMessage{e}
	}
	statuses := []ProviderStatus{
		StatusConnecting{},
		StatusLoginRequired{LoginStep{"log in", Str("login"), Str("https://l"), Str("qr")}},
		StatusLoginRequired{LoginStep{Message: "log in"}},
		StatusSyncing{}, StatusReady{}, StatusDisconnected{"lost"}, StatusFailed{"bad"},
	}
	for i, s := range statuses {
		f[fmt.Sprintf("status_%d", i)] = PmStatus{s}
	}
	f["secret_get"] = PmSecretGet{"k"}
	f["secret_set"] = PmSecretSet{"k", bytes.Repeat([]byte{1}, 200)}
	f["secret_delete"] = PmSecretDelete{"k"}
	updates := []DirectoryUpdate{
		OrganizationUpserted{Organization{ID: "g", Name: "Guild", Me: Str("u1"), Users: []User{{ID: "u1", Name: "Me"}}, Channels: []Channel{fixChannel()}}},
		OrganizationUpdated{ID: "g", Name: "G2"},
		OrganizationRemoved{"g"},
		UserUpserted{nil, User{ID: "u", Name: "U"}},
		UserRemoved{Str("g"), "u"},
		ChannelRemoved{guildChannel()},
		Me{nil, Str("u1")},
		Me{Str("g"), nil},
		DirectoryReset{},
	}
	for i := Online; i <= Offline; i++ {
		updates = append(updates, PresenceChanged{Str("g"), fmt.Sprintf("u%d", i), i})
	}
	for k := ChannelVoice; k <= ChannelCustom; k++ {
		kind := ChannelKind{Tag: k}
		if k == ChannelCustom {
			kind.Custom = "stage"
		}
		updates = append(updates, ChannelUpserted{nil, Channel{ID: "k", Name: "k", Kind: kind}})
	}
	for i, u := range updates {
		f[fmt.Sprintf("directory_%d", i)] = PmDirectory{u}
	}
	replies := []Reply{
		ReplyFunctions{fixFunctions()},
		ReplyValue{Record(allValues())},
		ReplyVerified{Valid{}},
		ReplyVerified{Invalid{[]InputIssue{{Str("members"), "unknown_user", "no"}, {nil, "x", "y"}}}},
		ReplyCompleted{CompletionPage{[]CompletionItem{{"Alice", Str("@alice"), Text("1")}}, Str("2")}},
		ReplyCompleted{CompletionPage{}},
		ReplySent{"m9"},
		ReplyOpened{dmRef()},
		ReplyHistory{HistoryPage{[]Message{fixMessage(), plainMessage()}, Str("m0")}},
		ReplyMessage{fixMessage()},
		ReplyBlob{[]byte{7, 7, 7}},
	}
	for i, r := range replies {
		f[fmt.Sprintf("reply_ok_%d", i)] = PmReply{Request: uint64(i) + 100, Reply: r}
	}
	errs := []CommandError{
		ErrUnsupported{}, ErrUnknownFunction{"f"},
		ErrInvalidInput{[]InputIssue{{Str("i"), "c", "m"}}},
		ErrProvider{"http_500", "oops", Integer(500)}, ErrProvider{"c", "m", nil},
		ErrUnknownOrganization{"g"}, ErrUnknownChannel{dmRef()}, ErrUnknownUser{"u"},
		ErrUnknownMessage{"m"}, ErrUnknownBlob{"b"},
		ErrRateLimited{U64(1500)}, ErrRateLimited{nil},
		ErrForbidden{"no"}, ErrLoginRequired{}, ErrCancelled{}, ErrFailed{"f"},
	}
	for _, v := range []Violation{
		ViolationEmpty{}, ViolationUnsupported{1, ContentKind{KindCustom, "x"}},
		ViolationUnsupported{2, ContentKind{Tag: KindFormatted}},
		ViolationUnsupportedMime{3, Str("a/b")}, ViolationTextTooLong{2001, 2000},
		ViolationInvalidFormatting{4}, ViolationTooLarge{5, 9, 8},
		ViolationTooManyAttachments{11, 10}, ViolationTotalTooLarge{9, 8},
	} {
		errs = append(errs, ErrRejected{v})
	}
	for i, e := range errs {
		f[fmt.Sprintf("reply_err_%d", i)] = PmReply{Request: uint64(i), Err: e}
	}
	f["read_blob"] = PmReadBlob{"b", 1 << 40, MaxRead}
	f["lookup"] = PmLookup{"discord.com:443"}
	f["browser_login"] = PmBrowserLogin{1 << 40, "discord"}
	f["cancel_browser_login"] = PmCancelBrowserLogin{1 << 40}
	return f
}

func TestRustFixtures(t *testing.T) {
	want := fixtures()
	files, err := filepath.Glob("testdata/fixtures/*.bin")
	if err != nil || len(files) == 0 {
		t.Fatalf("no fixtures: %v", err)
	}
	if len(files) != len(want) {
		t.Errorf("%d fixture files, %d Go values", len(files), len(want))
	}
	for _, file := range files {
		name := strings.TrimSuffix(filepath.Base(file), ".bin")
		golden, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		v, ok := want[name]
		if !ok {
			t.Errorf("%s: no Go value", name)
			continue
		}
		var encoded, reencoded []byte
		switch v := v.(type) {
		case ProviderConfig:
			encoded = EncodeProviderConfig(v)
			d, err := DecodeProviderConfig(golden)
			if err != nil {
				t.Errorf("%s: decode: %v", name, err)
				continue
			}
			reencoded = EncodeProviderConfig(d)
		case HostMessage:
			encoded = EncodeHostMessage(v)
			d, err := DecodeHostMessage(golden)
			if err != nil {
				t.Errorf("%s: decode: %v", name, err)
				continue
			}
			reencoded = EncodeHostMessage(d)
		case ProviderMessage:
			encoded = EncodeProviderMessage(v)
			d, err := DecodeProviderMessage(golden)
			if err != nil {
				t.Errorf("%s: decode: %v", name, err)
				continue
			}
			reencoded = EncodeProviderMessage(d)
		}
		if !bytes.Equal(encoded, golden) {
			t.Errorf("%s: Go encodes\n%x\nRust encodes\n%x", name, encoded, golden)
		}
		if !bytes.Equal(reencoded, golden) {
			t.Errorf("%s: decoding then encoding gives\n%x", name, reencoded)
		}
	}
}

func TestDecodeRejectsBadInput(t *testing.T) {
	golden, err := os.ReadFile("testdata/fixtures/host_send.bin")
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < len(golden); i++ {
		if _, err := DecodeHostMessage(golden[:i]); err == nil {
			t.Fatalf("truncated to %d bytes decoded", i)
		}
	}
	if _, err := DecodeHostMessage(append(golden, 0)); err == nil {
		t.Fatal("trailing byte decoded")
	}
	if _, err := DecodeHostMessage([]byte{0, 12}); err == nil {
		t.Fatal("unknown command decoded")
	}
	// A length far beyond the input must fail rather than allocate.
	if _, err := DecodeHostMessage([]byte{0, 9, 0xff, 0xff, 0xff, 0xff, 0x0f}); err == nil {
		t.Fatal("huge length decoded")
	}
}
