package qmsg

import (
	"context"
	"errors"
	"fmt"
)

type blobReadKey struct {
	id     string
	offset uint64
}

// ReadBlob reads a host-owned upload. Cancelled reads keep their slot until
// the host answers, so late replies cannot settle a later read.
func (cx *Context) ReadBlob(ctx context.Context, id string, offset uint64, length uint32) ([]byte, error) {
	if len(id) > MaxID || length > MaxRead {
		return nil, errors.New("invalid blob read")
	}
	key := blobReadKey{id, offset}
	ch := make(chan HmBlob, 1)
	cx.mu.Lock()
	if cx.blobReads == nil {
		cx.blobReads = map[blobReadKey]chan HmBlob{}
	}
	if _, ok := cx.blobReads[key]; ok || len(cx.blobReads) >= MaxBlobReads {
		cx.mu.Unlock()
		return nil, errors.New("too many blob reads")
	}
	cx.blobReads[key] = ch
	cx.mu.Unlock()
	if err := cx.send(PmReadBlob{Blob: id, Offset: offset, Len: length}); err != nil {
		cx.mu.Lock()
		delete(cx.blobReads, key)
		cx.mu.Unlock()
		return nil, err
	}
	select {
	case result := <-ch:
		if result.Err != nil {
			return nil, errors.New(*result.Err)
		}
		if len(result.Bytes) > int(length) {
			return nil, fmt.Errorf("blob reply exceeds requested length")
		}
		return result.Bytes, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-cx.done:
		return nil, errors.New("host disconnected")
	}
}
