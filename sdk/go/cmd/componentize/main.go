// Command componentize turns a Go wasip1 reactor module into a qmsg provider
// component.
//
// The Go runtime can't serve cabi_realloc: the canonical ABI calls it while
// imports are forbidden, Go's allocator calls clock imports, and Go's heap
// takes any memory grown behind its back. So cabi_realloc is replaced by a
// bump allocator over a static Go array the SDK declares, found by the
// magic bytes it starts with. The exports also run the Go runtime's
// _initialize once, since components never call it.
package main

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

// Must match the SDK's arena: magic, then the u32 address of the next free
// byte (0 at first), then free space up to arenaSize from the start.
const (
	arenaMagic = "QMSG-ARENA-V1:\x00\x00"
	arenaSize  = 4 << 20
)

func main() {
	if len(os.Args) != 6 {
		fmt.Fprintln(os.Stderr, "usage: componentize <wasm-tools> <adapter.wasm> <wit-dir> <in.wasm> <out.wasm>")
		os.Exit(2)
	}
	if err := run(os.Args[1], os.Args[2], os.Args[3], os.Args[4], os.Args[5]); err != nil {
		fmt.Fprintln(os.Stderr, "componentize:", err)
		os.Exit(1)
	}
}

func run(tools, adapter, wit, in, out string) error {
	dir, err := os.MkdirTemp("", "componentize")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	wat := filepath.Join(dir, "m.wat")
	if err := cmd(tools, "print", in, "-o", wat); err != nil {
		return err
	}
	text, err := os.ReadFile(wat)
	if err != nil {
		return err
	}
	text, err = patch(text)
	if err != nil {
		return err
	}
	if err := os.WriteFile(wat, text, 0o644); err != nil {
		return err
	}
	core := filepath.Join(dir, "m.wasm")
	embedded := filepath.Join(dir, "e.wasm")
	if err := cmd(tools, "parse", wat, "-o", core); err != nil {
		return err
	}
	if err := cmd(tools, "component", "embed", wit, "--world", "go-provider", core, "-o", embedded); err != nil {
		return err
	}
	return cmd(tools, "component", "new", embedded, "--adapt", "wasi_snapshot_preview1="+adapter, "-o", out)
}

func cmd(name string, args ...string) error {
	c := exec.Command(name, args...)
	c.Stdout, c.Stderr = os.Stderr, os.Stderr
	if err := c.Run(); err != nil {
		return fmt.Errorf("%s %s: %w", filepath.Base(name), args[0], err)
	}
	return nil
}

var dataRe = regexp.MustCompile(`(?m)^  \(data \(;\d+;\) \(i32\.const (\d+)\) "(.*)"\)$`)

// arenaAddr finds the address of the SDK's arena in the data segments.
func arenaAddr(text []byte) (uint32, error) {
	var found []uint32
	for _, m := range dataRe.FindAllSubmatch(text, -1) {
		base, _ := strconv.ParseUint(string(m[1]), 10, 32)
		data, err := unescape(m[2])
		if err != nil {
			return 0, err
		}
		for off := 0; ; {
			i := bytes.Index(data[off:], []byte(arenaMagic))
			if i < 0 {
				break
			}
			found = append(found, uint32(base)+uint32(off+i))
			off += i + 1
		}
	}
	if len(found) != 1 {
		return 0, fmt.Errorf("expected one SDK arena, found %d (is the qmsg Go SDK linked?)", len(found))
	}
	if found[0]%8 != 0 {
		return 0, fmt.Errorf("SDK arena at %d is not 8-byte aligned", found[0])
	}
	return found[0], nil
}

func unescape(s []byte) ([]byte, error) {
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		if s[i] != '\\' {
			out = append(out, s[i])
			continue
		}
		i++
		switch c := s[i]; c {
		case 't':
			out = append(out, '\t')
		case 'n':
			out = append(out, '\n')
		case 'r':
			out = append(out, '\r')
		case '"', '\'', '\\':
			out = append(out, c)
		case 'u':
			end := bytes.IndexByte(s[i:], '}')
			r, err := strconv.ParseUint(string(s[i+2:i+end]), 16, 32)
			if err != nil {
				return nil, err
			}
			out = append(out, string(rune(r))...)
			i += end
		default:
			b, err := strconv.ParseUint(string(s[i:i+2]), 16, 8)
			if err != nil {
				return nil, fmt.Errorf("bad escape %q", s[i-1:i+2])
			}
			out = append(out, byte(b))
			i++
		}
	}
	return out, nil
}

func escape(b []byte) string {
	var s strings.Builder
	for _, c := range b {
		fmt.Fprintf(&s, "\\%02x", c)
	}
	return s.String()
}

func patch(text []byte) ([]byte, error) {
	addr, err := arenaAddr(text)
	if err != nil {
		return nil, err
	}
	next := addr + uint32(len(arenaMagic))
	end := addr + arenaSize
	exports := map[string]string{
		`  (export "_initialize" (func $_rt0_wasm_wasip1_lib))`: "",
		`  (export "abi-version" (func $abi_version))`:          `  (export "abi-version" (func $qmsg_abi_version))`,
		`  (export "run" (func $run))`:                          `  (export "run" (func $qmsg_run))`,
	}
	lines := strings.Split(string(text), "\n")
	seen := 0
	for i, l := range lines {
		if r, ok := exports[l]; ok {
			lines[i] = r
			seen++
		} else if m := dataRe.FindStringSubmatch(l); m != nil {
			// Memory starts zeroed, so trailing zeros, such as the
			// arena's free space, needn't be in the file.
			data, err := unescape([]byte(m[2]))
			if err != nil {
				return nil, err
			}
			lines[i] = fmt.Sprintf(`  (data (i32.const %s) "%s")`, m[1], escape(bytes.TrimRight(data, "\x00")))
		}
	}
	if seen != len(exports) {
		return nil, fmt.Errorf("found %d of the %d expected exports", seen, len(exports))
	}
	// Insert before the module's closing paren.
	last := len(lines) - 1
	for strings.TrimSpace(lines[last]) == "" {
		last--
	}
	if lines[last] != ")" {
		return nil, fmt.Errorf("unexpected module end %q", lines[last])
	}
	lines[last] = fmt.Sprintf(added, next, end, next+4) + "\n)"
	return []byte(strings.Join(lines, "\n")), nil
}

// The u32 at `next` holds the address of the next free byte, or 0 before
// the first allocation. Everything
// here runs without calling Go or any import.
const added = `  (global $qmsg_inited (mut i32) (i32.const 0))
  (func $qmsg_init
    global.get $qmsg_inited
    i32.eqz
    if
      i32.const 1
      global.set $qmsg_inited
      call $_rt0_wasm_wasip1_lib
    end)
  (func $qmsg_abi_version (result i32)
    call $qmsg_init
    call $abi_version)
  (func $qmsg_run (param i32 i32) (result i32)
    call $qmsg_init
    local.get 0
    local.get 1
    call $run)
  (func $qmsg_cabi_realloc (param $old i32) (param $old_size i32) (param $align i32) (param $size i32) (result i32)
    (local $p i32) (local $end i32)
    ;; p = align_up(*next or the arena's first free byte, align)
    i32.const %[1]d
    i32.load
    local.tee $p
    i32.eqz
    if
      i32.const %[3]d
      local.set $p
    end
    local.get $p
    local.get $align
    i32.add
    i32.const 1
    i32.sub
    i32.const 0
    local.get $align
    i32.sub
    i32.and
    local.tee $p
    local.get $size
    i32.add
    local.tee $end
    i32.const %[2]d
    i32.gt_u
    local.get $end
    local.get $p
    i32.lt_u
    i32.or
    if
      unreachable
    end
    i32.const %[1]d
    local.get $end
    i32.store
    local.get $old
    if
      local.get $p
      local.get $old
      local.get $old_size
      local.get $size
      local.get $old_size
      local.get $size
      i32.lt_u
      select
      memory.copy
    end
    local.get $p)
  (export "cabi_realloc" (func $qmsg_cabi_realloc))`
