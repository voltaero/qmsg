package main

import (
	"encoding/json"
	"math"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/ningen/v3"
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

func resolveDMRecipients(st *ningen.State, ch discord.Channel) discord.Channel {
	if len(ch.DMRecipients) == 0 {
		return ch
	}
	ch.DMRecipients = append([]discord.User(nil), ch.DMRecipients...)
	indices := map[discord.UserID]int{}
	for i, user := range ch.DMRecipients {
		indices[user.ID] = i
	}
	// READY can deduplicate recipients into the global user cache. Read that
	// cache under its lock and enrich a copy, never the shared channel slice.
	st.PresenceStore.Each(0, func(presence *discord.Presence) bool {
		if i, found := indices[presence.User.ID]; found {
			user := &ch.DMRecipients[i]
			if user.Username == "" {
				user.Username = presence.User.Username
			}
			if user.DisplayName == "" {
				user.DisplayName = presence.User.DisplayName
			}
			if user.Avatar == "" {
				user.Avatar = presence.User.Avatar
			}
		}
		return false
	})
	return ch
}

func memberName(m discord.Member) string {
	if m.Nick != "" {
		return m.Nick
	}
	return userName(m.User)
}

// discordEpoch is when snowflake timestamps start.
var discordEpoch = time.Date(2015, 1, 1, 0, 0, 0, 0, time.UTC)

// recency orders direct messages as Discord does, newest conversation
// first: a smaller position is a later last message, or creation when
// there are no messages. Seconds since the Discord epoch fit in 32 bits
// for over a century.
func recency(ch discord.Channel) uint32 {
	at := ch.ID.Time()
	if ch.LastMessageID.IsValid() {
		at = ch.LastMessageID.Time()
	}
	return math.MaxUint32 - uint32(max(at.Sub(discordEpoch)/time.Second, 0))
}

// channel describes ch, accepting text only when send is allowed.
func channel(ch discord.Channel, me discord.UserID, send bool) qmsg.Channel {
	c := qmsg.Channel{ID: ch.ID.String(), Name: ch.Name, Kind: channelKind(ch.Type), Icon: publicImage(ch.IconURLWithType(discord.AutoImage))}
	if ch.Topic != "" {
		c.Topic = qmsg.Str(ch.Topic)
	}
	if ch.Type == discord.DirectMessage && len(ch.DMRecipients) == 1 {
		c.Icon = publicImage(ch.DMRecipients[0].AvatarURL())
	}
	if ch.ParentID.IsValid() {
		c.Parent = qmsg.Str(ch.ParentID.String())
	}
	if ch.GuildID.IsValid() {
		c.Position = qmsg.U32(uint32(max(ch.Position, 0)))
	} else {
		c.Position = qmsg.U32(recency(ch))
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
		for _, kind := range []qmsg.ContentKindTag{qmsg.KindImage, qmsg.KindVideo, qmsg.KindAudio, qmsg.KindFile} {
			c.AcceptedContent = append(c.AcceptedContent, qmsg.ContentRule{Kind: qmsg.ContentKind{Tag: kind}, MaxSize: qmsg.U64(maxFile)})
		}
		c.Limits.MaxAttachments = qmsg.U32(10)
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
	if a.Width > 0 {
		m.Metadata.Width = qmsg.U32(uint32(a.Width))
	}
	if a.Height > 0 {
		m.Metadata.Height = qmsg.U32(uint32(a.Height))
	}
	if a.Proxy != "" {
		preview := string(a.Proxy)
		if u, err := url.Parse(preview); err == nil && strings.HasPrefix(a.ContentType, "image/") && u.Hostname() == "media.discordapp.net" {
			query := u.Query()
			query.Set("width", "512")
			query.Set("height", "512")
			query.Set("format", "webp")
			u.RawQuery = query.Encode()
			preview = u.String()
		}
		m.Metadata.PreviewURL = qmsg.Str(preview)
	}
	if a.Description != "" {
		m.Metadata.Description = qmsg.Str(a.Description)
	}
	m.Metadata.Spoiler = strings.HasPrefix(a.Filename, "SPOILER_")
	if u, err := url.Parse(string(a.URL)); err == nil {
		if expiry, err := strconv.ParseUint(u.Query().Get("ex"), 16, 64); err == nil && expiry <= ^uint64(0)/1000 {
			m.Metadata.ExpiresAtMs = qmsg.U64(expiry * 1000)
		}
	}
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
	for _, embed := range m.Embeds {
		if data, err := json.Marshal(embed); err == nil {
			out = append(out, qmsg.ContentCustom{Kind: "discord.embed", Data: data})
		}
	}
	for _, sticker := range m.Stickers {
		if data, err := json.Marshal(sticker); err == nil {
			out = append(out, qmsg.ContentCustom{Kind: "discord.sticker", Data: data})
		}
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

func publicImage(value string) *qmsg.Media {
	if value == "" {
		return nil
	}
	return &qmsg.Media{Source: qmsg.SourceURL{URL: value}}
}
func profile(u discord.User) qmsg.User {
	return qmsg.User{ID: u.ID.String(), Name: userName(u), Username: qmsg.Str(u.Username), Avatar: publicImage(u.AvatarURL())}
}
func memberProfile(guild discord.GuildID, m discord.Member) qmsg.User {
	u := profile(m.User)
	u.Name = memberName(m)
	u.Avatar = publicImage(m.AvatarURL(guild))
	return u
}
