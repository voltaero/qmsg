package qmsg

import (
	"errors"
	"fmt"
	"math"
	"sort"
	"unicode/utf8"
)

// Postcard, as serde and the postcard crate use it for qmsg-types:
// unsigned integers are LEB128 varints, i64 is zigzag then varint, u8 and
// bool are one byte, strings and byte lists carry a varint length, options
// a 0/1 byte, enums a varint variant index, and structs their fields in
// order.

type encoder struct{ b []byte }

func (e *encoder) varint(v uint64) {
	for v >= 0x80 {
		e.b = append(e.b, byte(v)|0x80)
		v >>= 7
	}
	e.b = append(e.b, byte(v))
}

func (e *encoder) u32(v uint32)   { e.varint(uint64(v)) }
func (e *encoder) u64(v uint64)   { e.varint(v) }
func (e *encoder) i64(v int64)    { e.varint(uint64(v<<1) ^ uint64(v>>63)) }
func (e *encoder) tag(v uint32)   { e.varint(uint64(v)) }
func (e *encoder) str(s string)   { e.varint(uint64(len(s))); e.b = append(e.b, s...) }
func (e *encoder) bytes(b []byte) { e.varint(uint64(len(b))); e.b = append(e.b, b...) }

func (e *encoder) boolean(v bool) {
	if v {
		e.b = append(e.b, 1)
	} else {
		e.b = append(e.b, 0)
	}
}

func (e *encoder) optStr(s *string) {
	e.boolean(s != nil)
	if s != nil {
		e.str(*s)
	}
}

func (e *encoder) optU64(v *uint64) {
	e.boolean(v != nil)
	if v != nil {
		e.u64(*v)
	}
}

func (e *encoder) optU32(v *uint32) {
	e.boolean(v != nil)
	if v != nil {
		e.u32(*v)
	}
}

func (e *encoder) strMap(m map[string]string) {
	e.varint(uint64(len(m)))
	for _, k := range sortedKeys(m) {
		e.str(k)
		e.str(m[k])
	}
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	// BTreeMap order: byte-wise, which is Go's string order.
	sort.Strings(keys)
	return keys
}

// errDecode is raised by decoder methods and recovered by decode.
type errDecode struct{ msg string }

type decoder struct{ b []byte }

func (d *decoder) fail(format string, a ...any) { panic(errDecode{fmt.Sprintf(format, a...)}) }

func (d *decoder) byte() byte {
	if len(d.b) == 0 {
		d.fail("unexpected end of input")
	}
	c := d.b[0]
	d.b = d.b[1:]
	return c
}

func (d *decoder) varint(max uint64) uint64 {
	var v uint64
	for s := 0; s < 70; s += 7 {
		c := d.byte()
		if s == 63 && c > 1 {
			d.fail("varint overflow")
		}
		v |= uint64(c&0x7f) << s
		if c < 0x80 {
			if v > max {
				d.fail("varint %d out of range", v)
			}
			return v
		}
	}
	d.fail("varint too long")
	return 0
}

func (d *decoder) u32() uint32 { return uint32(d.varint(math.MaxUint32)) }
func (d *decoder) u64() uint64 { return d.varint(math.MaxUint64) }
func (d *decoder) u16() uint16 { return uint16(d.varint(math.MaxUint16)) }
func (d *decoder) tag() uint32 { return d.u32() }

func (d *decoder) i64() int64 {
	v := d.u64()
	return int64(v>>1) ^ -int64(v&1)
}

func (d *decoder) boolean() bool {
	switch d.byte() {
	case 0:
		return false
	case 1:
		return true
	}
	d.fail("invalid bool")
	return false
}

func (d *decoder) len() int {
	n := d.varint(uint64(len(d.b)))
	return int(n)
}

func (d *decoder) raw(n int) []byte {
	if n > len(d.b) {
		d.fail("unexpected end of input")
	}
	b := d.b[:n:n]
	d.b = d.b[n:]
	return b
}

func (d *decoder) bytes() []byte { return append([]byte{}, d.raw(d.len())...) }

func (d *decoder) str() string {
	b := d.raw(d.len())
	if !utf8.Valid(b) {
		d.fail("invalid utf-8")
	}
	return string(b)
}

func (d *decoder) option() bool { return d.boolean() }

func (d *decoder) optStr() *string {
	if !d.option() {
		return nil
	}
	s := d.str()
	return &s
}

func (d *decoder) optU64() *uint64 {
	if !d.option() {
		return nil
	}
	v := d.u64()
	return &v
}

func (d *decoder) optU32() *uint32 {
	if !d.option() {
		return nil
	}
	v := d.u32()
	return &v
}

// count reads a sequence length; each element takes at least one byte.
func (d *decoder) count() int { return d.len() }

func (d *decoder) strMap() map[string]string {
	n := d.count()
	m := make(map[string]string, n)
	for i := 0; i < n; i++ {
		k := d.str()
		m[k] = d.str()
	}
	return m
}

func (d *decoder) badTag(enum string, tag uint32) { d.fail("unknown %s variant %d", enum, tag) }

func decode[T any](b []byte, f func(*decoder) T) (v T, err error) {
	defer func() {
		if r := recover(); r != nil {
			e, ok := r.(errDecode)
			if !ok {
				panic(r)
			}
			err = errors.New("postcard: " + e.msg)
		}
	}()
	d := &decoder{b}
	v = f(d)
	if len(d.b) != 0 {
		return v, fmt.Errorf("postcard: %d trailing bytes", len(d.b))
	}
	return v, nil
}

func encode(f func(*encoder)) []byte {
	e := &encoder{}
	f(e)
	return e.b
}

// Str returns a pointer to s, for optional fields.
func Str(s string) *string { return &s }

// U64 returns a pointer to v, for optional fields.
func U64(v uint64) *uint64 { return &v }

// U32 returns a pointer to v, for optional fields.
func U32(v uint32) *uint32 { return &v }
