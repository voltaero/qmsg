# qmsg terminal app

A keyboard-driven chat client for the qmsg orchestrator. The account/channel
sidebar and message pane take inspiration from
[tuicord-v2](https://github.com/clcment446/tuicord-v2). Rendering uses
[fxcode](https://github.com/IGN-Styly/fxcode), pinned to the text-widget change.

## Try it

```sh
cargo run -p qmsg -- --demo
```

The demo stays offline and never sends messages. The app needs a Unix terminal
at least 60 columns wide and 16 rows high. It restores the screen and cursor
when you quit.

## Connect

Start the orchestrator with your provider configuration. On its host, create a
short-lived pairing file, then give it to the terminal client:

```sh
umask 077
cargo run -p qmsg-orchestrator -- pair orchestrator.toml --json > pairing.json
cargo run -p qmsg -- --pair pairing.json
cargo run -p qmsg
```

Delete the pairing file after use. The device token and certificate trust live
in `$XDG_CONFIG_HOME/qmsg/device.json`, or `~/.config/qmsg/device.json`. The app
creates this file with mode `0600`; it contains a secret and must not be shared.
Use `--config PATH` to pair or run with a separate device file. Pairing refuses
to overwrite an existing file. To pair again after token expiry or revocation,
remove the old device file first.

HTTP is allowed only on loopback. For another machine, use HTTPS or a secure
HTTP/2 tunnel. The client checks private certificates against the fingerprint
in the pairing payload and checks the server host name. See the
[app API guide](../qmsg-orchestrator/APP_API.md) for server and tunnel setup.

## Keys

| Key | Action |
| --- | --- |
| Tab / Shift+Tab | Next / previous pane |
| Up / Down | Pick an account or channel; scroll in the message pane |
| Enter / i | Start a message from the channel pane |
| Enter in composer | Send |
| Alt+Enter | Add a line |
| Backspace / Ctrl+U | Delete a character / clear draft |
| Page Up / Page Down | Scroll; Page Up also requests older history |
| End in message pane | Return to latest messages |
| Ctrl+L | Sign in to the selected provider |
| Ctrl+R | Refresh |
| Esc | Return to channels; cancel a sign-in form |
| F1 | Show keyboard help |
| Ctrl+Q / Ctrl+C | Quit |

Drafts stay with each channel for the current session. Sending locks the
composer until the result arrives. A failed send keeps the draft; check history
before sending it again because delivery may be unknown. The client never
retries a send on its own.

Sign-in forms support text, hidden secrets, booleans, integers, and named
choices. Tab selects the next field and Enter submits. Browser login runs in
the background; open the link shown in the message pane. Passwords are not
saved to disk. Providers with more complex input types need another client.

The directory and latest history refresh every three seconds. While reading
older messages, the visible history stays still; return to the bottom to see
new messages. Times show UTC. This first version sends text and shows text,
reply references, and attachment names. Account setup, attachments, reactions,
editing, and deletion still use the host or another client. It does not render
HTML or execute terminal escape sequences from messages.

## Checks

```sh
cargo test -p qmsg
cargo clippy -p qmsg --all-targets -- -D warnings
cargo fmt -p qmsg --check
```
