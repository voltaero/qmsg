package main

import (
	"context"
	"github.com/diamondburned/arikawa/v3/discord"
	"github.com/diamondburned/arikawa/v3/utils/httputil"
	"github.com/diamondburned/ningen/v3"
	"net/http"
	"testing"

	qmsg "github.com/voltaero/qmsg/sdk/go"
)

func TestCancelledSendStopsBeforeSessionOrPlatformAccess(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	// No SDK connection or Discord session: touching either would fail.
	p := &provider{}
	id, err, submitted := p.trySend(ctx, qmsg.ChannelRef{Channel: "1"}, nil, nil, nil)
	if _, ok := err.(qmsg.ErrCancelled); !ok || submitted || id != "" {
		t.Fatalf("got id=%q err=%v submitted=%v", id, err, submitted)
	}
}

func TestFindUsersIncludesDeduplicatedReadyUsers(t *testing.T) {
	st := ningen.New("fake-token")
	p := &provider{me: 1001}
	st.Cabinet.ChannelSet(&discord.Channel{ID: 3001, Type: discord.DirectMessage, DMRecipients: []discord.User{{ID: 1002}}}, true)
	st.Cabinet.PresenceSet(0, &discord.Presence{User: discord.User{ID: 1002, Username: "c446.dev", DisplayName: "Friend"}}, true)
	st.Cabinet.PresenceSet(0, &discord.Presence{User: discord.User{ID: 1003, Username: "other.friend"}}, true)
	matches := p.findUsers(st, "c446.dev")
	if len(matches) != 1 || matches[0].ID != "1002" || matches[0].Name != "Friend" {
		t.Fatalf("matches: %+v", matches)
	}
	if len(p.findUsers(st, "other.friend")) != 1 {
		t.Fatal("global-only user missing")
	}
	if issue := p.verifyUser(st, "1002"); issue != nil {
		t.Fatalf("verification: %+v", issue)
	}
}

func TestInvalidBodyIsDefinitelyNotSent(t *testing.T) {
	err := commandError(&httputil.HTTPError{Status: http.StatusBadRequest, Code: 50035, Message: "Invalid body"}, nil)
	failure, ok := err.(qmsg.ErrProvider)
	if !ok {
		t.Fatalf("wrong error type: %T", err)
	}
	details, ok := failure.Details.(qmsg.Record)
	if !ok || details["delivery_unknown"] != qmsg.Boolean(false) {
		t.Fatalf("Discord rejected the body, but delivery is not marked as definite: %T", failure.Details)
	}
}

func TestIDOnlyUsersAreNotMarkedKnownBeforeTheirProfileArrives(t *testing.T) {
	p := &provider{}
	// Ready sometimes supplies just an ID. No SDK connection exists here:
	// publishing it as a full profile would fail and block the later real name.
	p.knowUser(nil, discord.User{ID: 1009})
	if p.users[""][1009] {
		t.Fatal("an ID-only record would hide the later name")
	}
}
