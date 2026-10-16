package main

import (
	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/ningen/v3"
	qmsg "github.com/voltaero/qmsg/sdk/go"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestDMNameUsesTheDeduplicatedReadyProfile(t *testing.T) {
	st := ningen.New("fake-token")
	st.Cabinet.PresenceSet(0, &discord.Presence{User: discord.User{ID: 1002, Username: "ana.dev", DisplayName: "Ana", Avatar: "avatar"}}, true)
	original := discord.Channel{ID: 3100, Type: discord.DirectMessage, DMRecipients: []discord.User{{ID: 1002}}}
	resolved := resolveDMRecipients(st, original)
	c := channel(resolved, 1001, true)
	if c.Name != "Ana" || c.Icon == nil {
		t.Fatalf("DM lost its profile: name=%q icon=%v", c.Name, c.Icon)
	}
	if resolved.DMRecipients[0].Username != "ana.dev" {
		t.Fatal("recipient profile remains ID-only")
	}
	if original.DMRecipients[0].Username != "" {
		t.Fatal("resolver mutated the shared channel cache")
	}
}

func TestAttachmentPreviewDoesNotPointAtTheFullOriginal(t *testing.T) {
	original := "https://cdn.discordapp.com/attachments/1/2/photo.png?ex=ffffffff&hm=signature"
	c := attachment(discord.Attachment{Filename: "photo.png", ContentType: "image/png", URL: discord.URL(original), Proxy: "https://media.discordapp.net/attachments/1/2/photo.png?ex=ffffffff&hm=signature", Width: 2048, Height: 1536})
	m := c.(qmsg.ContentImage).Media
	u, e := url.Parse(*m.Metadata.PreviewURL)
	if e != nil {
		t.Fatal(e)
	}
	if u.Query().Get("width") != "512" || u.Query().Get("height") != "384" {
		t.Fatal("image preview must stay bounded and preserve the original aspect ratio")
	}
	if u.Query().Get("hm") != "signature" || m.Source.(qmsg.SourceURL).URL != original {
		t.Fatal("preview modified the original or its signature")
	}
}

func TestAttachmentPreviewAspectRatios(t *testing.T) {
	for _, tc := range []struct {
		name                  string
		width, height         uint
		wantWidth, wantHeight string
	}{
		{"landscape", 480, 240, "512", "256"},
		{"portrait", 240, 480, "256", "512"},
		{"square", 100, 100, "512", "512"},
		{"thin", 10000, 1, "512", "1"},
		{"unknown", 0, 0, "512", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			c := attachment(discord.Attachment{ContentType: "image/png", Proxy: "https://media.discordapp.net/attachments/1/2/image.png?ex=ffffffff&hm=signature&height=900", Width: tc.width, Height: tc.height})
			m := c.(qmsg.ContentImage).Media
			u, err := url.Parse(*m.Metadata.PreviewURL)
			if err != nil {
				t.Fatal(err)
			}
			q := u.Query()
			if q.Get("width") != tc.wantWidth || q.Get("height") != tc.wantHeight {
				t.Fatalf("preview dimensions = %sx%s; want %sx%s", q.Get("width"), q.Get("height"), tc.wantWidth, tc.wantHeight)
			}
			if q.Get("hm") != "signature" || q.Get("ex") != "ffffffff" {
				t.Fatal("preview lost its signed parameters")
			}
		})
	}
}

func TestProfilesAndGroupIcons(t *testing.T) {
	u := profile(discord.User{ID: 1002, Username: "friend", DisplayName: "Friend", Avatar: "a_avatar"})
	if u.Username == nil || *u.Username != "friend" || u.Avatar == nil {
		t.Fatalf("missing profile: %+v", u)
	}
	if source, ok := u.Avatar.Source.(qmsg.SourceURL); !ok || !strings.Contains(source.URL, "a_avatar.gif") {
		t.Fatalf("animated avatar missing: %+v", u.Avatar)
	}
	c := channel(discord.Channel{ID: 3100, Type: discord.GroupDM, Icon: "group-icon", Topic: "topic"}, 1001, true)
	if c.Icon == nil || c.Topic == nil || *c.Topic != "topic" {
		t.Fatalf("missing channel details: %+v", c)
	}
}

func TestDirectMessagesSortNewestFirst(t *testing.T) {
	at := func(d time.Duration) discord.MessageID {
		return discord.MessageID(discord.NewSnowflake(discordEpoch.Add(d)))
	}
	old := channel(discord.Channel{ID: discord.ChannelID(at(time.Hour)), Type: discord.DirectMessage, LastMessageID: at(48 * time.Hour)}, 1001, true)
	recent := channel(discord.Channel{ID: discord.ChannelID(at(2 * time.Hour)), Type: discord.DirectMessage, LastMessageID: at(72 * time.Hour)}, 1001, true)
	quiet := channel(discord.Channel{ID: discord.ChannelID(at(24 * time.Hour)), Type: discord.GroupDM}, 1001, true)
	if *recent.Position >= *old.Position || *old.Position >= *quiet.Position {
		t.Fatalf("positions %d %d %d are not newest first", *recent.Position, *old.Position, *quiet.Position)
	}
	guild := channel(discord.Channel{ID: 5, GuildID: 1, Type: discord.GuildText, Position: 3}, 1001, true)
	if *guild.Position != 3 {
		t.Fatal("server channels keep Discord's position")
	}
}

func TestGroupWithoutOtherRecipientsContainsOnlySelf(t *testing.T) {
	c := channel(discord.Channel{ID: 3101, Type: discord.GroupDM}, 1001, true)
	if c.Members == nil || len(*c.Members) != 1 || (*c.Members)[0] != "1001" {
		t.Fatalf("empty group must not fall back to all contacts: %v", c.Members)
	}
}
