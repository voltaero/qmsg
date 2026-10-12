use crate::{
    client::{Request, Update},
    model::{Provider, Room, Target, Timeline},
};
use fxcode::{Border, Color, Container, Node, Style, TerminalSize, Text};
use qmsg_types::{Content, ProviderStatus};
use std::collections::HashMap;
use termion::event::Key;
use tokio::sync::mpsc::Sender;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const BG: Color = Color::from_rgba(19, 22, 31, 255);
const PANEL: Color = Color::from_rgba(25, 29, 40, 255);
const INK: Color = Color::from_rgba(220, 225, 239, 255);
const MUTED: Color = Color::from_rgba(137, 148, 172, 255);
const ACCENT: Color = Color::from_rgba(128, 189, 255, 255);
const EDGE: Color = Color::from_rgba(58, 66, 88, 255);
const SELECT: Color = Color::from_rgba(43, 57, 79, 255);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Accounts,
    Channels,
    Messages,
    Compose,
}
pub struct State {
    pub providers: Vec<Provider>,
    account: usize,
    room: usize,
    focus: Focus,
    pub timeline: Timeline,
    drafts: HashMap<String, String>,
    pending: Option<Target>,
    pub status: String,
    scroll: usize,
    help: bool,
    pub demo: bool,
    loading_older: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            providers: vec![],
            account: 0,
            room: 0,
            focus: Focus::Channels,
            timeline: Timeline::default(),
            drafts: HashMap::new(),
            pending: None,
            status: "Connecting to orchestrator…".into(),
            scroll: 0,
            help: false,
            demo: false,
            loading_older: false,
        }
    }
}
impl State {
    fn rooms(&self) -> Vec<Room> {
        self.providers
            .get(self.account)
            .map(Provider::rooms)
            .unwrap_or_default()
    }
    pub fn selected(&self) -> Option<Room> {
        self.rooms().get(self.room).cloned()
    }
    fn draft_key(target: &Target) -> String {
        format!(
            "{}:{}:{:?}:{}",
            target.provider.name,
            target.provider.instance,
            target.channel.organization,
            target.channel.channel
        )
    }
    fn draft(&self) -> &str {
        self.selected()
            .and_then(|r| self.drafts.get(&Self::draft_key(&r.target)))
            .map(String::as_str)
            .unwrap_or("")
    }
    fn select(&mut self, tx: &Sender<Request>) {
        self.timeline = Timeline::default();
        self.scroll = 0;
        self.loading_older = false;
        if let Some(room) = self.selected() {
            if !self.demo {
                self.queue(tx, Request::Select(room.target));
            }
        }
    }
    fn queue(&mut self, tx: &Sender<Request>, request: Request) -> bool {
        if tx.try_send(request).is_err() {
            self.status = "Busy or disconnected. Try again shortly.".into();
            false
        } else {
            true
        }
    }
    pub fn update(&mut self, update: Update, tx: &Sender<Request>) {
        match update {
            Update::Connected => self.status = "Connected · refreshing every 3 seconds".into(),
            Update::Error(error) => {
                self.status = error;
                self.loading_older = false;
            }
            Update::Snapshot(snapshot) => {
                let old = self.selected().map(|r| r.target);
                let account = self.providers.get(self.account).map(|p| p.provider.clone());
                self.providers = snapshot.providers;
                self.account = account
                    .and_then(|id| self.providers.iter().position(|p| p.provider == id))
                    .unwrap_or(0);
                self.room = old
                    .as_ref()
                    .and_then(|target| self.rooms().iter().position(|r| &r.target == target))
                    .unwrap_or(0);
                if self.selected().map(|r| r.target) != old {
                    self.select(tx);
                }
            }
            Update::History(target, page, older) => {
                if self.selected().is_some_and(|r| r.target == target) {
                    if older {
                        self.timeline.prepend(page);
                        self.loading_older = false;
                    } else if self.scroll == 0 {
                        self.timeline.messages = page.messages;
                        self.timeline.cursor = page.next_cursor;
                    }
                }
            }
            Update::Sent(target) => {
                self.drafts.remove(&Self::draft_key(&target));
                self.pending = None;
                self.status = "Message sent".into();
                self.queue(tx, Request::Refresh);
            }
            Update::SendFailed(_target, error) => {
                self.pending = None;
                self.status = error;
            }
        }
    }
    pub fn key(&mut self, key: Key, tx: &Sender<Request>) -> bool {
        if key == Key::Ctrl('c') || key == Key::Ctrl('q') {
            return false;
        }
        if self.help {
            self.help = false;
            return true;
        }
        match key {
            Key::F(1) => self.help = true,
            Key::Char('\t') => {
                self.focus = match self.focus {
                    Focus::Accounts => Focus::Channels,
                    Focus::Channels => Focus::Messages,
                    Focus::Messages => Focus::Compose,
                    Focus::Compose => Focus::Accounts,
                }
            }
            Key::BackTab => {
                self.focus = match self.focus {
                    Focus::Accounts => Focus::Compose,
                    Focus::Channels => Focus::Accounts,
                    Focus::Messages => Focus::Channels,
                    Focus::Compose => Focus::Messages,
                }
            }
            Key::Esc => self.focus = Focus::Channels,
            Key::Ctrl('r') if !self.demo => {
                self.queue(tx, Request::Refresh);
            }
            Key::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
                self.older(tx);
            }
            Key::PageDown => self.scroll = self.scroll.saturating_sub(10),
            Key::End if self.focus == Focus::Messages => {
                self.scroll = 0;
            }
            Key::Up | Key::Down
                if self.focus == Focus::Accounts || self.focus == Focus::Channels =>
            {
                let up = key == Key::Up;
                let count = if self.focus == Focus::Accounts {
                    self.providers.len()
                } else {
                    self.rooms().len()
                };
                if count > 0 {
                    let index = if self.focus == Focus::Accounts {
                        &mut self.account
                    } else {
                        &mut self.room
                    };
                    let next = if up {
                        index.saturating_sub(1)
                    } else {
                        (*index + 1).min(count - 1)
                    };
                    if next != *index {
                        *index = next;
                        if self.focus == Focus::Accounts {
                            self.room = 0;
                        }
                        self.select(tx);
                    }
                }
            }
            Key::Up if self.focus == Focus::Messages => {
                self.scroll += 1;
                self.older(tx);
            }
            Key::Down if self.focus == Focus::Messages => {
                self.scroll = self.scroll.saturating_sub(1)
            }
            Key::Char('\n') if self.focus == Focus::Accounts => self.focus = Focus::Channels,
            Key::Char('\n') if self.focus == Focus::Channels => self.focus = Focus::Compose,
            Key::Char('i') if self.focus != Focus::Compose => self.focus = Focus::Compose,
            Key::Char('\n') if self.focus == Focus::Compose => {
                if let Some(room) = self.selected() {
                    let text = self.draft().to_string();
                    if !text.trim().is_empty() && self.pending.is_none() {
                        if self.demo {
                            self.status = "Demo mode · messages are not sent".into();
                        } else if let Err(error) =
                            room.channel.check(&[Content::Text(text.clone())])
                        {
                            self.status = format!("Cannot send: {error:?}");
                        } else if self.queue(tx, Request::Send(room.target.clone(), text)) {
                            self.pending = Some(room.target);
                            self.status = "Sending…".into();
                        }
                    }
                }
            }
            key if self.focus == Focus::Compose && self.pending.is_none() => {
                if let Some(room) = self.selected() {
                    let draft = self
                        .drafts
                        .entry(Self::draft_key(&room.target))
                        .or_default();
                    match key {
                        Key::Backspace => {
                            if let Some((offset, _)) = draft.grapheme_indices(true).next_back() {
                                draft.truncate(offset);
                            }
                        }
                        Key::Ctrl('u') => draft.clear(),
                        Key::Alt('\n') => draft.push('\n'),
                        Key::Char(c) if !c.is_control() && draft.len() < 64 * 1024 => draft.push(c),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        true
    }
    fn older(&mut self, tx: &Sender<Request>) {
        if self.demo || self.loading_older {
            return;
        }
        if let (Some(room), Some(cursor)) = (self.selected(), self.timeline.cursor.clone()) {
            if self.queue(tx, Request::Older(room.target, cursor)) {
                self.loading_older = true;
            }
        }
    }
    pub fn view(&self, size: TerminalSize) -> Vec<Node> {
        let (w, h) = (size.cols, size.rows);
        let mut nodes = vec![Container::new().style(Style::new().background(BG)).into()];
        if w < 60 || h < 16 {
            nodes.push(text(
                "qmsg · terminal too small\nResize to at least 60 × 16. Ctrl+Q quits.",
                1,
                1,
                w.saturating_sub(2),
                h.saturating_sub(2),
                ACCENT,
                BG,
            ));
            return nodes;
        }
        nodes.push(text(
            " qmsg  /  all your conversations",
            0,
            0,
            w,
            1,
            ACCENT,
            PANEL,
        ));
        let badge = if self.demo {
            "DEMO · offline"
        } else {
            "TERMINAL"
        };
        nodes.push(text(badge, w.saturating_sub(17), 0, 16, 1, MUTED, PANEL));
        let left = if w >= 100 { 28 } else { 21 };
        let accounts_h = (self.providers.len() as u16 + 3).clamp(5, 9).min(h - 10);
        panel(
            &mut nodes,
            " Accounts ",
            0,
            2,
            left,
            accounts_h,
            self.focus == Focus::Accounts,
        );
        let visible = usize::from(accounts_h - 2);
        let start = self.account.saturating_sub(visible.saturating_sub(1));
        if self.providers.is_empty() {
            nodes.push(text("No accounts yet", 2, 3, left - 4, 1, MUTED, PANEL));
        }
        for (row, (index, provider)) in self
            .providers
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .enumerate()
        {
            let marker = if index == self.account { "›" } else { " " };
            let status = match provider.status {
                ProviderStatus::Ready => "●",
                ProviderStatus::LoginRequired(_) => "!",
                _ => "○",
            };
            nodes.push(text(
                &format!("{marker} {status} {}", provider.provider.name),
                1,
                3 + row as u16,
                left - 2,
                1,
                if index == self.account { ACCENT } else { INK },
                if index == self.account { SELECT } else { PANEL },
            ));
        }
        let channels_y = 2 + accounts_h;
        panel(
            &mut nodes,
            " Channels ",
            0,
            channels_y,
            left,
            h - channels_y - 2,
            self.focus == Focus::Channels,
        );
        let rooms = self.rooms();
        let visible = usize::from(h - channels_y - 4);
        let start = self.room.saturating_sub(visible.saturating_sub(1));
        if rooms.is_empty() {
            nodes.push(text(
                "No channels",
                2,
                channels_y + 1,
                left - 4,
                1,
                MUTED,
                PANEL,
            ));
        }
        for (row, (index, room)) in rooms
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .enumerate()
        {
            nodes.push(text(
                &format!(
                    "{} # {}",
                    if index == self.room { "›" } else { " " },
                    room.channel.name
                ),
                1,
                channels_y + 1 + row as u16,
                left - 2,
                1,
                if index == self.room { ACCENT } else { INK },
                if index == self.room { SELECT } else { PANEL },
            ));
        }
        let x = left + 1;
        let width = w - x;
        let selected = self.selected();
        let title = selected
            .as_ref()
            .map(|r| format!(" # {} / {} ", r.channel.name, r.group))
            .unwrap_or_else(|| " Welcome to qmsg ".into());
        panel(
            &mut nodes,
            &title,
            x,
            2,
            width,
            h - 8,
            self.focus == Focus::Messages,
        );
        let content_w = width - 4;
        let mut lines: Vec<(String, Color)> = Vec::new();
        if let Some(provider) = self.providers.get(self.account) {
            if let ProviderStatus::LoginRequired(step) = &provider.status {
                for line in wrap(&step.message, content_w as usize) {
                    lines.push((line, ACCENT));
                }
                if let Some(url) = &step.link {
                    for line in wrap(url, content_w as usize) {
                        lines.push((line, ACCENT));
                    }
                }
                lines.push((
                    "Complete provider sign-in from the host or another client.".into(),
                    MUTED,
                ));
            }
        }
        if let Some(room) = &selected {
            if let Some(topic) = &room.channel.topic {
                for line in wrap(topic, content_w as usize) {
                    lines.push((line, MUTED));
                }
                lines.push((String::new(), INK));
            }
            for message in &self.timeline.messages {
                let author = room
                    .users
                    .get(&message.author)
                    .map(|u| u.name.as_str())
                    .unwrap_or(&message.author);
                let minutes = message.sent_at / 60_000;
                lines.push((
                    format!(
                        "{author}  {:02}:{:02} UTC{}",
                        minutes / 60 % 24,
                        minutes % 60,
                        if message.edited_at.is_some() {
                            " · edited"
                        } else {
                            ""
                        }
                    ),
                    ACCENT,
                ));
                if let Some(reply) = &message.reply_to {
                    lines.push((format!("↳ reply to {reply}"), MUTED));
                }
                for part in &message.content {
                    let body = match part {
                        Content::Text(s) => s.clone(),
                        Content::Formatted(s) => s.text.clone(),
                        Content::Image(m)
                        | Content::Video(m)
                        | Content::Audio(m)
                        | Content::File(m) => {
                            format!("[attachment: {}]", m.name.as_deref().unwrap_or("unnamed"))
                        }
                        Content::Custom { kind, .. } => format!("[{kind}]"),
                    };
                    for line in wrap(&body, content_w as usize) {
                        lines.push((line, INK));
                    }
                }
                lines.push((String::new(), INK));
            }
        }
        if lines.is_empty() {
            let empty = if selected.is_some() {
                "No messages yet. Press Enter to write a message."
            } else {
                "Choose an account and channel.\nManage accounts with the orchestrator or another paired client.\n\nTab moves between panes. F1 shows the keys."
            };
            lines.extend(
                wrap(empty, content_w as usize)
                    .into_iter()
                    .map(|s| (s, MUTED)),
            );
        }
        let rows = usize::from(h - 10);
        let end = lines
            .len()
            .saturating_sub(self.scroll.min(lines.len().saturating_sub(rows)));
        let start = end.saturating_sub(rows);
        for (row, (line, color)) in lines[start..end].iter().enumerate() {
            nodes.push(text(
                line,
                x + 2,
                3 + row as u16,
                content_w,
                1,
                *color,
                PANEL,
            ));
        }
        let composer_title = if self.pending.is_some() {
            " Sending… "
        } else {
            " Message · Enter sends "
        };
        panel(
            &mut nodes,
            composer_title,
            x,
            h - 6,
            width,
            4,
            self.focus == Focus::Compose,
        );
        let draft = self.draft();
        let body = if draft.is_empty() {
            if self.focus == Focus::Compose {
                "▏".to_string()
            } else {
                "Press i to write…".to_string()
            }
        } else {
            format!(
                "{draft}{}",
                if self.focus == Focus::Compose {
                    "▏"
                } else {
                    ""
                }
            )
        };
        let wrapped = wrap(&body, content_w as usize);
        let start = wrapped.len().saturating_sub(2);
        nodes.push(text(
            &wrapped[start..].join("\n"),
            x + 2,
            h - 5,
            content_w,
            2,
            INK,
            PANEL,
        ));
        nodes.push(text(
            &format!(" {}", self.status),
            0,
            h - 2,
            w,
            1,
            MUTED,
            BG,
        ));
        nodes.push(text(" Tab panes  ↑↓ select  i write  PgUp/PgDn history  Ctrl+R refresh  F1 help  Ctrl+Q quit", 0, h - 1, w, 1, ACCENT, PANEL));
        if self.help {
            let mw = 56.min(w - 4);
            let mx = (w - mw) / 2;
            let my = (h - 13) / 2;
            panel(&mut nodes, " Keyboard help ", mx, my, mw, 13, true);
            nodes.push(text("Tab / Shift+Tab    Next / previous pane\n↑ / ↓              Select account or channel\nEnter / i          Write in selected channel\nEnter              Send message\nAlt+Enter          New line\nBackspace / Ctrl+U Delete character / clear draft\nPgUp / PgDn        Scroll and load older messages\nEnd                Return to latest messages\nCtrl+R             Refresh from server\nEsc                Focus channels\nCtrl+Q / Ctrl+C    Quit · any key closes help", mx + 2, my + 1, mw - 4, 11, INK, PANEL));
        }
        nodes
    }
}
fn text(value: &str, x: u16, y: u16, w: u16, h: u16, color: Color, bg: Color) -> Node {
    Text::new(value)
        .color(color)
        .style(Style::new().offset(x, y).width(w).height(h).background(bg))
        .into()
}
fn panel(nodes: &mut Vec<Node>, title: &str, x: u16, y: u16, w: u16, h: u16, focused: bool) {
    let safe: String = title.chars().filter(|c| !c.is_control()).collect();
    // Border titles in fxcode are character-based, so keep their width bounded.
    let title: String = safe
        .chars()
        .map(|c| if c.width().unwrap_or(0) > 1 { '?' } else { c })
        .take(w.saturating_sub(4) as usize)
        .collect();
    nodes.push(
        Container::new()
            .style(
                Style::new()
                    .offset(x, y)
                    .width(w)
                    .height(h)
                    .background(PANEL)
                    .border(Border::plain(if focused { ACCENT } else { EDGE }).title(title)),
            )
            .into(),
    );
}
use unicode_width::UnicodeWidthChar;
pub fn wrap(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut lines = Vec::new();
    for line in value.split('\n') {
        let mut row = String::new();
        let mut used = 0;
        for grapheme in line.graphemes(true) {
            if grapheme.chars().any(char::is_control) {
                continue;
            }
            let n = grapheme.width();
            if n > width {
                continue;
            }
            if used + n > width {
                lines.push(std::mem::take(&mut row));
                used = 0;
            }
            row.push_str(grapheme);
            used += n;
        }
        lines.push(row);
    }
    lines
}
