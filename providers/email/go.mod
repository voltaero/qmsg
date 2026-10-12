module github.com/voltaero/qmsg/providers/email

go 1.26.0

replace github.com/voltaero/qmsg/sdk/go => ../../sdk/go

require (
	github.com/emersion/go-imap/v2 v2.0.0-beta.8
	github.com/emersion/go-message v0.18.2
	github.com/emersion/go-sasl v0.0.0-20241020182733-b788ff22d5a6
	github.com/emersion/go-smtp v0.25.0
	github.com/voltaero/qmsg/sdk/go v0.0.0-00010101000000-000000000000
	golang.org/x/crypto/x509roots/fallback v0.0.0-20261009200856-99e4382b128e
)

require (
	github.com/gorilla/websocket v1.5.3 // indirect
	golang.org/x/text v0.14.0 // indirect
)
