use std::{
    io::{self, Read},
    sync::mpsc::Sender,
};
use termion::{
    event::{Event, Key},
    input::TermReadEventsAndRaw,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    Key(Key),
    Paste(String),
}

pub fn read(reader: impl Read, output: Sender<io::Result<Input>>) {
    let mut pasted: Option<Vec<u8>> = None;
    for event in reader.events_and_raw() {
        let (event, raw) = match event {
            Ok(event) => event,
            Err(error) => {
                let _ = output.send(Err(error));
                return;
            }
        };
        if raw == b"\x1b[200~" {
            pasted = Some(Vec::new());
        } else if raw == b"\x1b[201~" {
            if let Some(bytes) = pasted.take()
                && output
                    .send(Ok(Input::Paste(
                        String::from_utf8_lossy(&bytes).into_owned(),
                    )))
                    .is_err()
            {
                return;
            }
        } else if let Some(bytes) = &mut pasted {
            let remaining = (64 * 1024usize).saturating_sub(bytes.len());
            bytes.extend(raw.into_iter().take(remaining));
        } else if let Event::Key(key) = event
            && output.send(Ok(Input::Key(key))).is_err()
        {
            return;
        }
    }
    if let Some(bytes) = pasted {
        let _ = output.send(Ok(Input::Paste(
            String::from_utf8_lossy(&bytes).into_owned(),
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bracketed_paste_keeps_newlines_out_of_key_events() {
        let (tx, rx) = std::sync::mpsc::channel();
        read(&b"\x1b[200~one\ntwo\r\n\x1b[201~\r"[..], tx);
        let events: Vec<_> = rx.into_iter().map(Result::unwrap).collect();
        assert_eq!(
            events,
            vec![
                Input::Paste("one\ntwo\r\n".into()),
                Input::Key(Key::Char('\n'))
            ]
        );
    }
    #[test]
    fn ordinary_keys_and_unicode_still_work() {
        let (tx, rx) = std::sync::mpsc::channel();
        read("é\x1b[A".as_bytes(), tx);
        let events: Vec<_> = rx.into_iter().map(Result::unwrap).collect();
        assert_eq!(
            events,
            vec![Input::Key(Key::Char('é')), Input::Key(Key::Up)]
        );
    }
}
