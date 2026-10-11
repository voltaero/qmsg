package main

import (
	"strings"

	"github.com/diamondburned/arikawa/v3/discord"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

// maxText is Discord's message length for accounts without Nitro. Discord
// counts Unicode code points.
const maxText = 2000

// orgOf is the organization a guild's channels belong to; direct messages
// have none.
func orgOf(guild discord.GuildID) *string {
	if !guild.IsValid() {
		return nil
	}
	return qmsg.Str(guild.String())
}

func channelRef(guild discord.GuildID, ch discord.ChannelID) qmsg.ChannelRef {
	return qmsg.ChannelRef{Organization: orgOf(guild), Channel: ch.String()}
}

func channelKind(t discord.ChannelType) qmsg.ChannelKind {
	switch t {
	case discord.GuildText:
		return qmsg.ChannelKind{Tag: qmsg.ChannelText}
	case discord.DirectMessage:
		return qmsg.ChannelKind{Tag: qmsg.ChannelDirect}
	case discord.GuildVoice:
		return qmsg.ChannelKind{Tag: qmsg.ChannelVoice}
	case discord.GroupDM:
		return qmsg.ChannelKind{Tag: qmsg.ChannelGroup}
	case discord.GuildCategory:
		return qmsg.ChannelKind{Tag: qmsg.ChannelCategory}
	case discord.GuildAnnouncement:
		return qmsg.ChannelKind{Tag: qmsg.ChannelAnnouncement}
	case discord.GuildAnnouncementThread, discord.GuildPublicThread, discord.GuildPrivateThread:
		return qmsg.ChannelKind{Tag: qmsg.ChannelThread}
	case discord.GuildForum:
		return qmsg.ChannelKind{Tag: qmsg.ChannelForum}
	case discord.GuildStageVoice:
		return qmsg.ChannelKind{Tag: qmsg.ChannelCustom, Custom: "stage"}
	case discord.GuildDirectory:
		return qmsg.ChannelKind{Tag: qmsg.ChannelCustom, Custom: "directory"}
	default:
		return qmsg.ChannelKind{Tag: qmsg.ChannelCustom, Custom: "unknown"}
	}
}

// holdsMessages says whether a channel kind has a message history at all.
func holdsMessages(t discord.ChannelType) bool {
	switch t {
	case discord.GuildText, discord.DirectMessage, discord.GroupDM, discord.GuildAnnouncement,
		discord.GuildAnnouncementThread, discord.GuildPublicThread, discord.GuildPrivateThread,
		discord.GuildVoice, discord.GuildStageVoice:
		return true
	}
	return false
}

func isThread(t discord.ChannelType) bool {
	return t == discord.GuildAnnouncementThread || t == discord.GuildPublicThread || t == discord.GuildPrivateThread
}

func userName(u discord.User) string {
	switch {
	case u.DisplayName != "":
		return u.DisplayName
	case u.Username != "":
		return u.Username
	default:
		return u.ID.String()
	}
}

func memberName(m discord.Member) string {
	if m.Nick != "" {
		return m.Nick
	}
	return userName(m.User)
}

// channel describes ch, accepting text only when send is allowed.
func channel(ch discord.Channel, me discord.UserID, send bool) qmsg.Channel {
	c := qmsg.Channel{ID: ch.ID.String(), Name: ch.Name, Kind: channelKind(ch.Type)}
	if ch.ParentID.IsValid() {
		c.Parent = qmsg.Str(ch.ParentID.String())
	}
	if ch.GuildID.IsValid() {
		c.Position = qmsg.U32(uint32(max(ch.Position, 0)))
	}
	if len(ch.DMRecipients) > 0 {
		members := []string{me.String()}
		var names []string
		for _, u := range ch.DMRecipients {
			members = append(members, u.ID.String())
			names = append(names, userName(u))
		}
		c.Members = &members
		if c.Name == "" {
			c.Name = strings.Join(names, ", ")
		}
	}
	if send && holdsMessages(ch.Type) {
		c.AcceptedContent = []qmsg.ContentRule{{
			Kind:     qmsg.ContentKind{Tag: qmsg.KindText},
			MaxSize:  qmsg.U64(maxText),
			TextUnit: qmsg.UnitChars,
		}}
	}
	return c
}

func millis(t discord.Timestamp) uint64 {
	if !t.IsValid() {
		return 0
	}
	return uint64(t.Time().UnixMilli())
}

// reactionKey is a reaction as Discord's API names it: the emoji itself, or
// `name:id` for custom ones.
func reactionKey(e discord.Emoji) string { return string(e.APIString()) }

func attachment(a discord.Attachment) qmsg.Content {
	m := qmsg.Media{Name: qmsg.Str(a.Filename), Size: qmsg.U64(a.Size), Source: qmsg.SourceURL{URL: string(a.URL)}}
	if a.ContentType != "" {
		m.Mime = qmsg.Str(a.ContentType)
	}
	switch {
	case strings.HasPrefix(a.ContentType, "image/"):
		return qmsg.ContentImage{Media: m}
	case strings.HasPrefix(a.ContentType, "video/"):
		return qmsg.ContentVideo{Media: m}
	case strings.HasPrefix(a.ContentType, "audio/"):
		return qmsg.ContentAudio{Media: m}
	default:
		return qmsg.ContentFile{Media: m}
	}
}

func contents(m discord.Message) []qmsg.Content {
	var out []qmsg.Content
	if m.Content != "" {
		out = append(out, qmsg.ContentText{Text: m.Content})
	}
	for _, a := range m.Attachments {
		out = append(out, attachment(a))
	}
	return out
}

func message(m discord.Message) qmsg.Message {
	out := qmsg.Message{
		ID:      m.ID.String(),
		Channel: channelRef(m.GuildID, m.ChannelID),
		Author:  m.Author.ID.String(),
		SentAt:  millis(m.Timestamp),
		Content: contents(m),
	}
	if !m.Timestamp.IsValid() {
		out.SentAt = uint64(m.ID.Time().UnixMilli())
	}
	if m.Reference != nil && m.Reference.MessageID.IsValid() {
		out.ReplyTo = qmsg.Str(m.Reference.MessageID.String())
	}
	if m.EditedTimestamp.IsValid() {
		out.EditedAt = qmsg.U64(millis(m.EditedTimestamp))
	}
	for _, r := range m.Reactions {
		out.Reactions = append(out.Reactions, qmsg.Reaction{Key: reactionKey(r.Emoji), Count: uint32(max(r.Count, 0)), Me: r.Me})
	}
	if m.Nonce != "" {
		out.Nonce = qmsg.Str(m.Nonce)
	}
	return out
}

// sendable checks content against what channel accepts, as the
// orchestrator's Channel::check would: only text, within maxText.
func sendable(content []qmsg.Content) (string, qmsg.CommandError) {
	if len(content) == 0 {
		return "", qmsg.ErrRejected{Violation: qmsg.ViolationEmpty{}}
	}
	var b strings.Builder
	for i, c := range content {
		t, ok := c.(qmsg.ContentText)
		if !ok {
			return "", qmsg.ErrRejected{Violation: qmsg.ViolationUnsupported{Part: uint32(i), Kind: contentKindOf(c)}}
		}
		b.WriteString(t.Text)
	}
	text := b.String()
	if n := len([]rune(text)); n > maxText {
		return "", qmsg.ErrRejected{Violation: qmsg.ViolationTextTooLong{Length: uint64(n), Max: maxText}}
	}
	if strings.TrimSpace(text) == "" {
		return "", qmsg.ErrRejected{Violation: qmsg.ViolationEmpty{}}
	}
	return text, nil
}

func contentKindOf(c qmsg.Content) qmsg.ContentKind {
	switch c := c.(type) {
	case qmsg.ContentText:
		return qmsg.ContentKind{Tag: qmsg.KindText}
	case qmsg.ContentFormatted:
		return qmsg.ContentKind{Tag: qmsg.KindFormatted}
	case qmsg.ContentImage:
		return qmsg.ContentKind{Tag: qmsg.KindImage}
	case qmsg.ContentVideo:
		return qmsg.ContentKind{Tag: qmsg.KindVideo}
	case qmsg.ContentAudio:
		return qmsg.ContentKind{Tag: qmsg.KindAudio}
	case qmsg.ContentFile:
		return qmsg.ContentKind{Tag: qmsg.KindFile}
	case qmsg.ContentCustom:
		return qmsg.ContentKind{Tag: qmsg.KindCustom, Custom: c.Kind}
	}
	return qmsg.ContentKind{Tag: qmsg.KindCustom, Custom: "unknown"}
}
