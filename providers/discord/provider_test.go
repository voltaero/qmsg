package main

import (
	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/ningen/v3"
	qmsg "github.com/voltaero/qmsg/sdk/go"
	"strings"
	"testing"
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
