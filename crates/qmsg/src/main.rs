mod client;
mod input;
mod model;
mod ui;

use anyhow::{Context, Result};
use fxcode::{KittyMode, Renderer, TerminalSize};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};
use termion::{cursor, raw::IntoRawMode, screen::IntoAlternateScreen};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut pairing = None;
    let mut demo = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "qmsg — terminal messaging\n\nqmsg [--config FILE]\nqmsg --pair PAIRING_JSON [--config FILE]\nqmsg --demo\n\nCreate pairing JSON with qmsg-orchestrator pair CONFIG --json.\nDevice credentials live in $XDG_CONFIG_HOME/qmsg/device.json (mode 0600).\nF1 shows keyboard help. Ctrl+Q quits."
                );
                return Ok(());
            }
            "--config" => {
                config = Some(PathBuf::from(args.next().context("--config needs a path")?))
            }
            "--pair" => pairing = Some(PathBuf::from(args.next().context("--pair needs a path")?)),
            "--demo" => demo = true,
            _ => anyhow::bail!("Unknown option {arg}. Use --help."),
        }
    }
    let runtime = tokio::runtime::Runtime::new()?;
    if let Some(file) = pairing {
        let path = config.map(Ok).unwrap_or_else(client::config_path)?;
        runtime.block_on(client::pair(&file, &path))?;
        println!("Paired. Device credentials saved to {}", path.display());
        return Ok(());
    }
    let (requests, receiver, selection) = client::requests(32);
    let (updates, incoming) = mpsc::channel();
    let worker = if demo {
        None
    } else {
        let path = config.map(Ok).unwrap_or_else(client::config_path)?;
        Some(runtime.spawn(client::run(
            client::load(&path)?,
            receiver,
            selection,
            updates,
        )))
    };
    let state = if demo {
        demo_state()
    } else {
        ui::State::default()
    };
    let result = terminal(state, requests, incoming);
    if let Some(worker) = worker {
        worker.abort();
    }
    result
}
fn terminal(
    mut state: ui::State,
    requests: client::Requests,
    incoming: mpsc::Receiver<client::Update>,
) -> Result<()> {
    struct Output<W: Write>(W);
    impl<W: Write> Write for Output<W> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl<W: Write> Drop for Output<W> {
        fn drop(&mut self) {
            let _ = self.0.write_all(b"\x1b[?2004l");
            let _ = self.0.flush();
        }
    }
    let mut output = Output(io::stdout().into_raw_mode()?.into_alternate_screen()?);
    write!(output, "\x1b[?2004h{}", cursor::Hide)?;
    output.flush()?;
    // Restores the cursor on errors and unwinding, before leaving the alternate screen.
    struct Terminal<W: Write>(Renderer<W>);
    impl<W: Write> Drop for Terminal<W> {
        fn drop(&mut self) {
            let _ = self.0.finish();
        }
    }
    let mut size = TerminalSize::current()?;
    let mut terminal = Terminal(Renderer::with_kitty(output, size, KittyMode::Disabled));
    let (keys, input) = mpsc::channel();
    std::thread::spawn(move || input::read(io::stdin(), keys));
    let mut dirty = true;
    loop {
        while let Ok(update) = incoming.try_recv() {
            state.update(update, &requests);
            dirty = true;
        }
        let current = TerminalSize::current()?;
        if current.cols != size.cols || current.rows != size.rows {
            size = current;
            terminal.0.resize(size)?;
            dirty = true;
        }
        if dirty {
            terminal.0.draw(&state.view(size))?;
            dirty = false;
        }
        match input.recv_timeout(Duration::from_millis(50)) {
            Ok(key) => {
                match key? {
                    input::Input::Key(key) => {
                        if !state.key(key, &requests) {
                            break;
                        }
                    }
                    input::Input::Paste(text) => state.paste(&text),
                }
                dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
fn demo_state() -> ui::State {
    use model::*;
    use qmsg_types::*;
    let mut channel = Channel::new("general", "general", ChannelKind::Text);
    channel.position = Some(0);
    let mut scope = Scope::default();
    for name in ["Ana", "Claudio"] {
        scope.users.insert(
            name.into(),
            User {
                id: name.into(),
                name: name.into(),
                username: None,
                avatar: None,
            },
        );
    }
    scope.channels.insert(channel.id.clone(), channel);
    scope.channels.insert(
        "design".into(),
        Channel::new("design", "design", ChannelKind::Text),
    );
    scope.channels.insert(
        "random".into(),
        Channel::new("random", "random", ChannelKind::Text),
    );
    let provider = Provider {
        provider: ProviderId {
            name: "Community".into(),
            instance: 1,
        },
        status: ProviderStatus::Ready,
        organizations: vec![model::Organization {
            id: "qmsg".into(),
            name: "qmsg space".into(),
            scope,
        }],
        standalone: None,
    };
    let mut state = ui::State::default();
    state.demo = true;
    state.providers = vec![provider];
    state.status.clear();
    let channel = state.selected().unwrap().target.channel;
    for (index, (author, body)) in [
        ("Claudio", "I pushed the changes to the chat view."),
        (
            "Ana",
            "Looks good. The channel list is much easier to read now.",
        ),
        ("Claudio", "I'll check the smaller terminal size next."),
        ("Ana", "Olá, 世界 👋"),
        ("Claudio", "The screenshots are in the pull request."),
    ]
    .iter()
    .enumerate()
    {
        state.timeline.messages.push(Message::new(
            index.to_string(),
            channel.clone(),
            *author,
            (9 * 60 + 41 + index as u64) * 60_000,
            vec![Content::Text((*body).into())],
        ));
    }
    state
}
