package main

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net/mail"
	"strconv"
	"strings"
	"time"

	message "github.com/emersion/go-message"
	_ "github.com/emersion/go-message/charset"
	gomail "github.com/emersion/go-message/mail"
	qmsg "github.com/voltaero/qmsg/sdk/go"
)

const maxMail = 8 << 20
const maxPage = 20
const maxMailbox = 100000
const maxParts = 100

func parseAddress(s string) (*mail.Address, error) {
	if strings.ContainsAny(s, "\r\n\x00") {
		return nil, fmt.Errorf("invalid address")
	}
	a, e := mail.ParseAddress(s)
	if e == nil && !strings.Contains(a.Address, "@") {
		e = fmt.Errorf("address needs a domain")
	}
	return a, e
}
func addresses(h gomail.Header, k string) []qmsg.EmailAddress {
	as, _ := h.AddressList(k)
	out := make([]qmsg.EmailAddress, 0, len(as))
	for _, a := range as {
		v := qmsg.EmailAddress{Address: validText(a.Address)}
		if a.Name != "" {
			v.Name = qmsg.Str(validText(a.Name))
		}
		out = append(out, v)
	}
	return out
}
func parseMail(raw []byte, ref qmsg.ChannelRef, id string, flags []string) (qmsg.Message, error) {
	if len(raw) > maxMail {
		return qmsg.Message{}, fmt.Errorf("mail exceeds 8 MiB")
	}
	entity, err := message.ReadWithOptions(bytes.NewReader(raw), &message.ReadOptions{MaxHeaderBytes: 64 << 10})
	if err != nil {
		return qmsg.Message{}, fmt.Errorf("cannot read MIME mail: %w", err)
	}
	h := gomail.Header{Header: entity.Header}
	e := &qmsg.Email{From: addresses(h, "From"), To: addresses(h, "To"), Cc: addresses(h, "Cc"), Bcc: addresses(h, "Bcc"), ReplyTo: addresses(h, "Reply-To"), ContentIDs: map[string]string{}}
	e.Subject, _ = h.Subject()
	e.Subject = validText(e.Subject)
	mid, _ := h.MessageID()
	if mid != "" {
		mid = validText(mid)
		e.MessageID = &mid
	}
	refs, _ := h.MsgIDList("References")
	for i := range refs {
		refs[i] = validText(refs[i])
	}
	e.References = refs
	replies, _ := h.MsgIDList("In-Reply-To")
	if len(replies) > 0 {
		value := validText(replies[0])
		e.InReplyTo = &value
	}
	for _, f := range flags {
		switch strings.ToLower(f) {
		case "\\seen":
			e.Read = true
		case "\\flagged":
			e.Starred = true
		case "\\draft":
			e.Draft = true
		}
	}
	m := qmsg.Message{ID: id, Channel: ref, Email: e}
	if len(e.From) > 0 {
		m.Author = e.From[0].Address
	}
	date, _ := h.Date()
	if !date.IsZero() && date.UnixMilli() > 0 {
		m.SentAt = uint64(date.UnixMilli())
	}
	total, parts := 0, 0
	err = entity.Walk(func(path []int, part *message.Entity, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		parts++
		if parts > maxParts {
			return fmt.Errorf("too many MIME parts")
		}
		if len(path) > 30 {
			return fmt.Errorf("MIME nesting exceeds 30 levels")
		}
		if part.MultipartReader() != nil {
			return nil
		}
		data, err := io.ReadAll(io.LimitReader(part.Body, int64(maxMail-total+1)))
		if err != nil {
			return err
		}
		total += len(data)
		if total > maxMail {
			return fmt.Errorf("decoded mail exceeds 8 MiB")
		}
		ct, _, _ := part.Header.ContentType()
		disposition, _, _ := part.Header.ContentDisposition()
		if ct == "" {
			ct = "text/plain"
			if disposition == "attachment" {
				ct = "application/octet-stream"
			}
		}
		if disposition != "attachment" && ct == "text/plain" {
			m.Content = append(m.Content, qmsg.ContentText{Text: validText(string(data))})
			return nil
		}
		if disposition != "attachment" && ct == "text/html" {
			html := validText(string(data))
			e.HTML = &html
			return nil
		}
		name := "attachment"
		ah := gomail.AttachmentHeader{Header: part.Header}
		if v, _ := ah.Filename(); v != "" {
			name = validText(v)
		}
		cid := validText(strings.Trim(part.Header.Get("Content-ID"), "<>"))
		if cid != "" {
			e.ContentIDs[strconv.Itoa(len(m.Content))] = cid
		}
		media := qmsg.Media{Name: &name, Mime: &ct, Size: qmsg.U64(uint64(len(data))), Source: qmsg.SourceBytes{Bytes: data}}
		if strings.HasPrefix(ct, "image/") {
			m.Content = append(m.Content, qmsg.ContentImage{Media: media})
		} else {
			m.Content = append(m.Content, qmsg.ContentFile{Media: media})
		}
		return nil
	})
	if err != nil {
		return m, fmt.Errorf("cannot read MIME part: %w", err)
	}

	return m, nil
}

type compose struct {
	to, cc, bcc   []string
	subject, html string
	content       []qmsg.Content
	inReplyTo     *string
	references    []string
}

func recipients(values []string) ([]*gomail.Address, error) {
	out := []*gomail.Address{}
	for _, s := range values {
		a, e := parseAddress(s)
		if e != nil {
			return nil, fmt.Errorf("invalid recipient")
		}
		out = append(out, a)
	}
	return out, nil
}

// Compose before opening SMTP. No URL fetching, no Bcc header, and bounded output.
func composeMail(ctx context.Context, from string, c compose, read func(context.Context, string, uint64, uint32) ([]byte, error)) ([]byte, string, []string, error) {
	if strings.ContainsAny(c.subject, "\r\n\x00") || len(c.subject) > 998 {
		return nil, "", nil, fmt.Errorf("invalid subject")
	}
	a, e := parseAddress(from)
	if e != nil {
		return nil, "", nil, e
	}
	to, e := recipients(c.to)
	if e != nil {
		return nil, "", nil, e
	}
	cc, e := recipients(c.cc)
	if e != nil {
		return nil, "", nil, e
	}
	bcc, e := recipients(c.bcc)
	if e != nil {
		return nil, "", nil, e
	}
	var envelope []string
	for _, list := range [][]*gomail.Address{to, cc, bcc} {
		for _, a := range list {
			envelope = append(envelope, a.Address)
		}
	}
	if len(envelope) == 0 || len(envelope) > 100 {
		return nil, "", nil, fmt.Errorf("name 1 to 100 recipients")
	}
	h := gomail.Header{}
	h.SetAddressList("From", []*gomail.Address{a})
	h.SetAddressList("To", to)
	if len(cc) > 0 {
		h.SetAddressList("Cc", cc)
	}
	h.SetSubject(c.subject)
	h.SetDate(time.Now())
	h.GenerateMessageIDWithHostname(a.Address[strings.LastIndex(a.Address, "@")+1:])
	id, _ := h.MessageID()
	if c.inReplyTo != nil {
		h.SetMsgIDList("In-Reply-To", []string{*c.inReplyTo})
	}
	if len(c.references) > 0 {
		h.SetMsgIDList("References", c.references)
	}
	b := &boundedBuffer{max: maxMail}
	w, e := gomail.CreateWriter(b, h)
	if e != nil {
		return nil, "", nil, e
	}
	inline, e := w.CreateInline()
	if e != nil {
		return nil, "", nil, e
	}
	var text strings.Builder
	var media []qmsg.Media
	for _, part := range c.content {
		switch p := part.(type) {
		case qmsg.ContentText:
			text.WriteString(p.Text)
		case qmsg.ContentFile:
			media = append(media, p.Media)
		case qmsg.ContentImage:
			media = append(media, p.Media)
		case qmsg.ContentAudio:
			media = append(media, p.Media)
		case qmsg.ContentVideo:
			media = append(media, p.Media)
		default:
			return nil, "", nil, fmt.Errorf("unsupported mail content")
		}
	}
	if len(media) > 20 {
		return nil, "", nil, fmt.Errorf("at most 20 attachments")
	}
	ih := gomail.InlineHeader{}
	ih.SetContentType("text/plain", map[string]string{"charset": "utf-8"})
	part, e := inline.CreatePart(ih)
	if e != nil {
		return nil, "", nil, e
	}
	if _, e = io.WriteString(part, text.String()); e != nil {
		return nil, "", nil, e
	}
	if e = part.Close(); e != nil {
		return nil, "", nil, e
	}
	if c.html != "" {
		ih := gomail.InlineHeader{}
		ih.SetContentType("text/html", map[string]string{"charset": "utf-8"})
		p, e := inline.CreatePart(ih)
		if e != nil {
			return nil, "", nil, e
		}
		if _, e = io.WriteString(p, c.html); e != nil {
			return nil, "", nil, e
		}
		if e = p.Close(); e != nil {
			return nil, "", nil, e
		}
	}
	if e = inline.Close(); e != nil {
		return nil, "", nil, e
	}
	for _, m := range media {
		if ctx.Err() != nil {
			return nil, "", nil, ctx.Err()
		}
		name := "attachment"
		ct := "application/octet-stream"
		if m.Name != nil {
			name = *m.Name
		}
		if m.Mime != nil {
			ct = *m.Mime
		}
		if strings.ContainsAny(name+ct, "\r\n\x00") {
			return nil, "", nil, fmt.Errorf("invalid attachment header")
		}
		ah := gomail.AttachmentHeader{}
		ah.SetContentType(ct, nil)
		ah.SetFilename(name)
		p, e := w.CreateAttachment(ah)
		if e != nil {
			return nil, "", nil, e
		}
		switch s := m.Source.(type) {
		case qmsg.SourceBytes:
			_, e = p.Write(s.Bytes)
		case qmsg.SourceBlob:
			for off := uint64(0); ; {
				if read == nil {
					return nil, "", nil, fmt.Errorf("blob reader unavailable")
				}
				chunk, err := read(ctx, s.Blob, off, 256<<10)
				if err != nil {
					return nil, "", nil, err
				}
				if len(chunk) == 0 {
					break
				}
				if off+uint64(len(chunk)) > maxMail {
					return nil, "", nil, fmt.Errorf("attachment exceeds limit")
				}
				_, e = p.Write(chunk)
				if e != nil {
					break
				}
				off += uint64(len(chunk))
			}
		default:
			return nil, "", nil, fmt.Errorf("upload attachment bytes; URL attachments are unsupported")
		}
		if e != nil {
			return nil, "", nil, e
		}
		if e = p.Close(); e != nil {
			return nil, "", nil, e
		}
	}
	if e = w.Close(); e != nil {
		return nil, "", nil, e
	}
	return b.Bytes(), id, envelope, nil
}

type boundedBuffer struct {
	bytes.Buffer
	max int
}

func (b *boundedBuffer) Write(p []byte) (int, error) {
	if len(p) > b.max-b.Len() {
		return 0, fmt.Errorf("encoded mail exceeds 8 MiB")
	}
	return b.Buffer.Write(p)
}

func validText(s string) string { return strings.ToValidUTF8(s, "�") }
