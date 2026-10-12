package main

import (
	"bytes"
	"context"
	"github.com/diamondburned/arikawa/v3/utils/sendpart"
	qmsg "github.com/voltaero/qmsg/sdk/go"
	"io"
	"path"
	"strings"
)

const maxFile = 10 * 1024 * 1024 // Provider's conservative per-file budget.
type blobReader struct {
	cx           *qmsg.Context
	ctx          context.Context
	id           string
	offset, size uint64
}

func (r *blobReader) Read(dst []byte) (int, error) {
	if err := r.ctx.Err(); err != nil {
		return 0, err
	}
	if r.offset >= r.size {
		return 0, io.EOF
	}
	length := min(len(dst), 256*1024, int(r.size-r.offset))
	if length == 0 {
		return 0, nil
	}
	data, err := r.cx.ReadBlob(r.ctx, r.id, r.offset, uint32(length))
	if err != nil {
		return 0, err
	}
	if len(data) != length {
		return 0, io.ErrUnexpectedEOF
	}
	n := copy(dst, data)
	r.offset += uint64(n)
	return n, nil
}
func (p *provider) sendParts(ctx context.Context, content []qmsg.Content) (string, []sendpart.File, qmsg.CommandError) {
	var text strings.Builder
	var files []sendpart.File
	for i, part := range content {
		var media *qmsg.Media
		switch c := part.(type) {
		case qmsg.ContentText:
			text.WriteString(c.Text)
			continue
		case qmsg.ContentImage:
			media = &c.Media
		case qmsg.ContentVideo:
			media = &c.Media
		case qmsg.ContentAudio:
			media = &c.Media
		case qmsg.ContentFile:
			media = &c.Media
		default:
			return "", nil, qmsg.ErrRejected{Violation: qmsg.ViolationUnsupported{Part: uint32(i), Kind: contentKindOf(part)}}
		}
		var reader io.Reader
		var size uint64
		switch source := media.Source.(type) {
		case qmsg.SourceBytes:
			size = uint64(len(source.Bytes))
			reader = bytes.NewReader(source.Bytes)
		case qmsg.SourceBlob:
			if media.Size == nil {
				return "", nil, qmsg.ErrFailed{Reason: "blob attachments require size"}
			}
			size = *media.Size
			reader = &blobReader{cx: p.cx, ctx: ctx, id: source.Blob, size: size}
		default:
			return "", nil, qmsg.ErrFailed{Reason: "upload media before sending; URL sources cannot be sent"}
		}
		if size > maxFile {
			return "", nil, qmsg.ErrRejected{Violation: qmsg.ViolationTooLarge{Part: uint32(i), Size: size, Max: maxFile}}
		}
		name := "attachment"
		if media.Name != nil {
			name = path.Base(strings.ReplaceAll(*media.Name, "\\", "/"))
		}
		if name == "." || name == "/" || strings.ContainsAny(name, "\r\n\x00") {
			return "", nil, qmsg.ErrFailed{Reason: "invalid attachment filename"}
		}
		if media.Metadata.Spoiler && !strings.HasPrefix(name, "SPOILER_") {
			name = "SPOILER_" + name
		}
		files = append(files, sendpart.File{Name: name, Reader: reader})
	}
	if len(files) > 10 {
		return "", nil, qmsg.ErrRejected{Violation: qmsg.ViolationTooManyAttachments{Count: uint32(len(files)), Max: 10}}
	}
	value := text.String()
	if n := len([]rune(value)); n > maxText {
		return "", nil, qmsg.ErrRejected{Violation: qmsg.ViolationTextTooLong{Length: uint64(n), Max: maxText}}
	}
	if len(files) == 0 && strings.TrimSpace(value) == "" {
		return "", nil, qmsg.ErrRejected{Violation: qmsg.ViolationEmpty{}}
	}
	return value, files, nil
}
