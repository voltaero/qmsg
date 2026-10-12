module github.com/voltaero/qmsg/providers/discord

go 1.26.0

replace github.com/voltaero/qmsg/sdk/go => ../../sdk/go

require (
	github.com/diamondburned/arikawa/v3 v3.6.1-0.20260306005901-b24e0a8447e1
	github.com/diamondburned/ningen/v3 v3.0.1-0.20260306213430-5a08d3a709b4
	github.com/gorilla/websocket v1.5.3
	github.com/voltaero/qmsg/sdk/go v0.0.0-00010101000000-000000000000
	golang.org/x/crypto/x509roots/fallback v0.0.0-20261009200856-99e4382b128e
)

require (
	github.com/gorilla/schema v1.4.1 // indirect
	github.com/pkg/errors v0.9.1 // indirect
	github.com/twmb/murmur3 v1.1.8 // indirect
	golang.org/x/time v0.14.0 // indirect
)
