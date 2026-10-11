//go:build wasip1

package wasi

import (
	"sync"
	"time"
	"unsafe"
)

// The Go runtime is one thread, so waiting for a pollable must not block it.
// Goroutines register a pollable here and park on a channel; one goroutine
// checks every registered pollable without blocking, and sleeps through the
// Go scheduler between checks, which lets other goroutines run and lets the
// runtime block the thread in poll_oneoff when nothing else is runnable.

type waiter struct {
	p    uint32
	done chan struct{}
}

var (
	pollMu  sync.Mutex
	waiters []waiter
	wake    = make(chan struct{}, 1)
	started sync.Once
)

// MaxPollSleep bounds how late a ready pollable is noticed while idle.
var MaxPollSleep = 10 * time.Millisecond

// Wait parks the goroutine until p is ready or the timer fires. It does not
// drop p.
func Wait(p uint32, timeout <-chan time.Time) bool {
	started.Do(func() { go pollLoop() })
	done := make(chan struct{})
	pollMu.Lock()
	waiters = append(waiters, waiter{p, done})
	pollMu.Unlock()
	select {
	case wake <- struct{}{}:
	default:
	}
	select {
	case <-done:
		return true
	case <-timeout:
		pollMu.Lock()
		for i, w := range waiters {
			if w.done == done {
				waiters = append(waiters[:i], waiters[i+1:]...)
				break
			}
		}
		pollMu.Unlock()
		select {
		case <-done:
			return true
		default:
			return false
		}
	}
}

func pollLoop() {
	sleep := time.Duration(0)
	zero := subscribeDuration(0)
	for {
		pollMu.Lock()
		if len(waiters) == 0 {
			pollMu.Unlock()
			<-wake
			sleep = 0
			continue
		}
		list := make([]uint32, 0, len(waiters)+1)
		for _, w := range waiters {
			list = append(list, w.p)
		}
		list = append(list, zero)
		var ret [2]uint32
		var ready []byte
		Lists(func() { poll(unsafe.Pointer(&list[0]), uint32(len(list)), unsafe.Pointer(&ret)) },
			func() { ready = Bytes(ret[0], ret[1]*4) })
		readySet := map[uint32]bool{}
		for i := 0; i+4 <= len(ready); i += 4 {
			idx := uint32(ready[i]) | uint32(ready[i+1])<<8 | uint32(ready[i+2])<<16 | uint32(ready[i+3])<<24
			readySet[idx] = true
		}
		kept := waiters[:0]
		woke := false
		for i, w := range waiters {
			if readySet[uint32(i)] {
				close(w.done)
				woke = true
			} else {
				kept = append(kept, w)
			}
		}
		waiters = kept
		pollMu.Unlock()
		if woke {
			sleep = 0
			continue
		}
		if sleep < MaxPollSleep {
			sleep += time.Millisecond
		}
		select {
		case <-time.After(sleep):
		case <-wake:
			sleep = 0
		}
	}
}
