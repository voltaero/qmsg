package main

import (
	"encoding/json"
	"fmt"
	"sync"

	qmsg "github.com/voltaero/qmsg/sdk/go"
)

// REST and the gateway can confirm the same change in either order. Keep
// the last state for a bounded number of event identities. Gateway events
// arrive in order; opposite reaction changes still produce separate events.
type eventTracker struct {
	mu    sync.Mutex
	last  map[string]string
	order []string
}

func eventIdentity(event qmsg.MessageEvent) (string, string) {
	channel := func(ref qmsg.ChannelRef) string { b, _ := json.Marshal(ref); return string(b) }
	switch e := event.(type) {
	case qmsg.Received:
		return "received:" + channel(e.Message.Channel) + ":" + e.Message.ID, "received"
	case qmsg.Edited:
		body, _ := json.Marshal(e.Content)
		return "edited:" + channel(e.Channel) + ":" + e.ID, fmt.Sprint(e.EditedAt) + ":" + string(body)
	case qmsg.Deleted:
		return "deleted:" + channel(e.Channel) + ":" + e.ID, "deleted"
	case qmsg.Reacted:
		return "reaction:" + channel(e.Channel) + ":" + e.ID + ":" + e.User + ":" + e.Key, fmt.Sprint(e.Added)
	case qmsg.Read:
		b, _ := json.Marshal(e.UpTo)
		return "read:" + channel(e.Channel) + ":" + e.User, string(b)
	default:
		return "", ""
	}
}

func (p *provider) emit(event qmsg.MessageEvent) error {
	t := &p.events
	t.mu.Lock()
	defer t.mu.Unlock()
	key, value := eventIdentity(event)
	if key != "" {
		if t.last == nil {
			t.last = map[string]string{}
		}
		if prior, ok := t.last[key]; ok && prior == value {
			return nil
		}
	}
	if err := p.cx.Emit(event); err != nil {
		return err
	}
	if key != "" {
		if _, ok := t.last[key]; !ok {
			if len(t.order) == 4096 {
				delete(t.last, t.order[0])
				t.order = t.order[1:]
			}
			t.order = append(t.order, key)
		}
		t.last[key] = value
	}
	return nil
}
