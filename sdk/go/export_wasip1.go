//go:build wasip1

package qmsg

import (
	"fmt"
	"unsafe"

	"github.com/voltaero/qmsg/sdk/go/wasi"
)

// The world's exports, as cmd/componentize expects them: it wraps both so
// the Go runtime starts first.

//go:wasmexport abi-version
func abiVersion() uint32 { return ABIVersion }

// runResult is the canonical ABI's result<_, string>: a tag, then the
// string's pointer and length.
var (
	runResult [3]uint32
	runError  []byte
)

//go:wasmexport run
func run(ptr, n uint32) uint32 {
	err := runConfig(wasi.Bytes(ptr, n))
	runResult = [3]uint32{}
	if err != nil {
		runError = []byte(err.Error())
		if len(runError) == 0 {
			runError = []byte("failed")
		}
		runResult = [3]uint32{1, uint32(uintptr(unsafe.Pointer(&runError[0]))), uint32(len(runError))}
	}
	return uint32(uintptr(unsafe.Pointer(&runResult)))
}

func runConfig(raw []byte) (err error) {
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("provider panicked: %v", r)
		}
	}()
	if registered == nil {
		return fmt.Errorf("no provider registered; call qmsg.Register from init")
	}
	config, err := DecodeProviderConfig(raw)
	if err != nil {
		return fmt.Errorf("invalid provider config: %w", err)
	}
	return RunWith(config, registered)
}
