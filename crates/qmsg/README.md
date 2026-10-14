# qmsg terminal app

A keyboard-driven chat client for the qmsg orchestrator. The account/channel
sidebar and message pane take inspiration from
[tuicord-v2](https://github.com/clcment446/tuicord-v2). Rendering uses
[fxcode](https://github.com/IGN-Styly/fxcode), pinned to the text-widget change.

## Try it

```sh
cargo run -p qmsg -- --demo
```

The demo stays offline and never sends messages. fxcode links libmpv; install
`mpv` on Arch Linux or `libmpv-dev` on Debian/Ubuntu before building.
The app needs a Unix terminal
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

## Layout and keys

The layout follows tuicord-v2: servers, channels, and messages side by side,
with the account list and composer under the messages. Terminals at least 120
columns wide also show members; narrow terminals hide the account list, and the
quick switcher still reaches every account.

Navigation is modal like Vim. In normal mode, letters move around and never type.
Press `i` to write; Esc or `;q` returns to normal mode and keeps the draft.

| Key | Action |
| --- | --- |
| Tab / Shift+Tab | Next / previous panel |
| h / l, H / L | Previous / next panel |
| j / k, ↓ / ↑ | Move in a list; Enter opens the server, channel, or account |
| j / k in messages | Scroll a line |
| J / K, ↓ / ↑ in messages | Next / previous message |
| gg / G | Oldest loaded / newest messages |
| Page Up / Page Down | Scroll a page; reaching the top loads older history |
| i | Write in the open channel |
| Esc / ;q | Leave insert mode |
| Enter / Alt+Enter | Send / add a line |
| Backspace / Ctrl+U | Delete a character / clear the draft |
| Ctrl+K or + | Quick switch to any channel of any account |
| Ctrl+L | Sign in to the selected account |
| Ctrl+R | Refresh |
| F1 or Ctrl+/ | Show keyboard help |
| Ctrl+Q / Ctrl+C | Quit |

Moving through a list only highlights a row, as in tuicord; Enter opens it.
Channels sit under their categories, and threads under their channels.

Drafts stay with each channel for the current session. Sending holds the
submitted message until the result arrives; you can keep writing the next draft.
A rejected send puts its text back before that new draft. If delivery is unknown,
check history before sending it again. The client never
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
