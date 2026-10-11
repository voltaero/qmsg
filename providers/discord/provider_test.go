package main

import (
	"github.com/diamondburned/arikawa/v3/discord"
	qmsg "github.com/voltaero/qmsg/sdk/go"
	"strings"
	"testing"
)

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
