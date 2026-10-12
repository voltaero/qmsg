package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/voltaero/qmsg/providers/email/internal/fakemail"
)

func main() {
	s, e := fakemail.Start()
	if e != nil {
		panic(e)
	}
	defer s.Close()
	for i := 1; i <= 6; i++ {
		raw := fmt.Sprintf("From: Bob <bob@example.test>\r\nTo: alice@example.test\r\nSubject: Test %d\r\nMessage-ID: <test%d@example.test>\r\nDate: Sun, 11 Oct 2026 10:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nMail %d", i, i, i)
		if e = s.Append("INBOX", []byte(raw)); e != nil {
			panic(e)
		}
	}
	json.NewEncoder(os.Stdout).Encode(map[string]string{"imap": s.IMAP, "smtp": s.SMTP, "starttls": s.StartTLS, "oauth": s.OAuth, "ca": s.CA})
	io.Copy(io.Discard, os.Stdin)
}
