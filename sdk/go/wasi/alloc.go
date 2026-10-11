//go:build wasip1

package wasi

import (
	"sync"
	"unsafe"
)

// The host and the preview1 adapter allocate in this arena through a
// cabi_realloc the build adds in WAT, since Go code can't run there. Its
// layout must match cmd/componentize: magic, the u32 address of the next free
// byte (0 before the first allocation), then free space.
const arenaSize = 4 << 20

type arenaT struct {
	magic [16]byte
	next  uint32
	_     uint32
	_     [arenaSize - 24]byte
	_     [0]uint64
}

var arena = arenaT{magic: [16]byte{'Q', 'M', 'S', 'G', '-', 'A', 'R', 'E', 'N', 'A', '-', 'V', '1', ':'}}

// arenaMu makes each import that returns a list, and the copy out of the
// arena after it, one step: Lists reads results between the two.
var arenaMu sync.Mutex

// Lists runs call, which may make the host allocate lists in the arena, then
// frees what the host allocated once read is done copying out of it.
func Lists(call func(), read func()) {
	arenaMu.Lock()
	mark := (*uint32)(unsafe.Pointer(&arena.next))
	saved := *mark
	call()
	read()
	*mark = saved
	arenaMu.Unlock()
}

// Bytes copies n bytes at ptr, which must be in the arena.
func Bytes(ptr, n uint32) []byte {
	return append([]byte(nil), arenaSlice(ptr, n)...)
}

// arenaSlice is the n bytes at ptr, which the host allocated in the arena.
func arenaSlice(ptr, n uint32) []byte {
	if n == 0 {
		return nil
	}
	base := uintptr(unsafe.Pointer(&arena))
	off := uintptr(ptr) - base
	if uintptr(ptr) < base || off+uintptr(n) > unsafe.Sizeof(arena) || off+uintptr(n) < off {
		panic("wasi: host list outside the arena")
	}
	return unsafe.Slice((*byte)(unsafe.Add(unsafe.Pointer(&arena), off)), n)
}
