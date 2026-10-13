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
    login: Option<LoginForm>,
    login_busy: bool,
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
            login: None,
            login_busy: false,
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
        if let Some(room) = self.selected()
            && !self.demo
        {
            self.queue(tx, Request::Select(room.target));
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
            Update::LoginForm(provider, function) => {
                self.login = Some(LoginForm {
                    provider,
                    values: vec![String::new(); function.inputs.len()],
                    function,
                    field: 0,
                });
                self.login_busy = false;
            }
            Update::LoginDone => {
                self.login_busy = false;
                self.status.clear();
                self.queue(tx, Request::Refresh);
            }
            Update::Connected => self.status.clear(),
            Update::Error(error) => {
                self.login_busy = false;
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
                        self.timeline.refresh(page);
                    }
                }
            }
            Update::Sent(target) => {
                self.drafts.remove(&Self::draft_key(&target));
                self.pending = None;
                self.status.clear();
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
        if self.login.is_some() {
            if key == Key::Esc {
                self.login = None;
                return true;
            }
            if key == Key::Char('\n') {
                let form = self.login.as_ref().unwrap();
                match form.arguments() {
                    Ok(arguments) => {
                        let request = Request::SubmitLogin(
                            form.provider.clone(),
                            form.function.clone(),
                            arguments,
                        );
                        if self.queue(tx, request) {
                            self.login = None;
                            self.login_busy = true;
                            self.status = "Signing in… browser links appear here when ready".into();
                        }
                    }
                    Err(error) => self.status = error,
                }
            } else {
                self.login.as_mut().unwrap().key(key);
            }
            return true;
        }
        if self.help {
            self.help = false;
            return true;
        }
        match key {
            Key::F(1) => self.help = true,
            Key::Ctrl('l') if !self.demo && !self.login_busy => {
                if let Some(provider) = self.providers.get(self.account) {
                    if let ProviderStatus::LoginRequired(step) = &provider.status {
                        if let Some(function) = &step.function {
                            self.login_busy = self.queue(
                                tx,
                                Request::Login(provider.provider.clone(), function.clone()),
                            );
                        }
                    } else {
                        self.status = "This account does not need sign-in".into();
                    }
                }
            }
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
                        Key::Alt('\n' | '\r') => draft.push('\n'),
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
        if let (Some(room), Some(cursor)) = (self.selected(), self.timeline.cursor.clone())
            && self.queue(tx, Request::Older(room.target, cursor))
        {
            self.loading_older = true;
        }
    }
    pub fn view(&self, size: TerminalSize) -> Vec<Node> {
        let (w, h) = (size.cols, size.rows);
        let mut nodes = vec![
            Container::new()
                .style(Style::new().background(PANEL))
                .into(),
        ];
        if w < 60 || h < 16 {
            nodes.push(text(
                "qmsg · terminal too small\nResize to at least 60 × 16. Ctrl+Q quits.",
                1,
                1,
                w.saturating_sub(2),
                h.saturating_sub(2),
                ACCENT,
                PANEL,
            ));
            return nodes;
        }
        let left = if w >= 100 { 28 } else { 21 };
        let accounts_h = (self.providers.len() as u16 + 3).clamp(5, 9).min(h - 10);
        panel(
            &mut nodes,
            " Accounts ",
            0,
            0,
            left,
            accounts_h,
            self.focus == Focus::Accounts,
        );
        let visible = usize::from(accounts_h - 2);
        let start = self.account.saturating_sub(visible.saturating_sub(1));
        if self.providers.is_empty() {
            nodes.push(text("No accounts yet", 2, 1, left - 4, 1, MUTED, PANEL));
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
                1 + row as u16,
                left - 2,
                1,
                if index == self.account { ACCENT } else { INK },
                if index == self.account { SELECT } else { PANEL },
            ));
        }
        let channels_y = accounts_h;
        panel(
            &mut nodes,
            " Channels ",
            0,
            channels_y,
            left,
            h - channels_y - 1,
            self.focus == Focus::Channels,
        );
        let rooms = self.rooms();
        let visible = usize::from(h - channels_y - 3);
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
        let x = left;
        let width = w - x;
        let selected = self.selected();
        let title = selected
            .as_ref()
            .map(|r| format!(" # {} / {} ", r.channel.name, r.group))
            .unwrap_or_else(|| " Messages ".into());
        panel(
            &mut nodes,
            &title,
            x,
            0,
            width,
            h - 5,
            self.focus == Focus::Messages,
        );
        let content_w = width - 4;
        let mut lines: Vec<(String, Color)> = Vec::new();
        if let Some(provider) = self.providers.get(self.account)
            && let ProviderStatus::LoginRequired(step) = &provider.status
        {
            for line in wrap(&step.message, content_w as usize) {
                lines.push((line, ACCENT));
            }
            if let Some(url) = &step.link {
                for line in wrap(url, content_w as usize) {
                    lines.push((line, ACCENT));
                }
            }
            lines.push((
                "Press Ctrl+L to sign in. Open browser links in your browser.".into(),
                MUTED,
            ));
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
                "No messages yet."
            } else {
                "Select a channel."
            };
            lines.extend(
                wrap(empty, content_w as usize)
                    .into_iter()
                    .map(|s| (s, MUTED)),
            );
        }
        let rows = usize::from(h - 7);
        let end = lines
            .len()
            .saturating_sub(self.scroll.min(lines.len().saturating_sub(rows)));
        let start = end.saturating_sub(rows);
        for (row, (line, color)) in lines[start..end].iter().enumerate() {
            nodes.push(text(
                line,
                x + 2,
                1 + row as u16,
                content_w,
                1,
                *color,
                PANEL,
            ));
        }
        let composer_title = if self.pending.is_some() {
            " Sending… "
        } else {
            " Message "
        };
        panel(
            &mut nodes,
            composer_title,
            x,
            h - 5,
            width,
            4,
            self.focus == Focus::Compose,
        );
        let draft = self.draft();
        let body = if draft.is_empty() {
            if self.focus == Focus::Compose {
                "▏".to_string()
            } else {
                String::new()
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
            h - 4,
            content_w,
            2,
            INK,
            PANEL,
        ));
        nodes.push(text(
            &self.status,
            1,
            h - 1,
            w.saturating_sub(12),
            1,
            MUTED,
            PANEL,
        ));
        nodes.push(text(
            "F1 Help",
            w.saturating_sub(8),
            h - 1,
            7,
            1,
            MUTED,
            PANEL,
        ));
        if self.help {
            let mw = 56.min(w - 4);
            let mx = (w - mw) / 2;
            let my = (h - 13) / 2;
            panel(&mut nodes, " Keyboard help ", mx, my, mw, 13, true);
            nodes.push(text("Tab / Shift+Tab    Next / previous pane\n↑ / ↓              Select account or channel\nEnter / i          Write in selected channel\nEnter              Send message\nAlt+Enter          New line\nBackspace / Ctrl+U Delete character / clear draft\nPgUp / PgDn        Scroll and load older messages\nEnd                Return to latest messages\nCtrl+R / Ctrl+L    Refresh / sign in\nEsc                Focus channels\nCtrl+Q / Ctrl+C    Quit · any key closes help", mx + 2, my + 1, mw - 4, 11, INK, PANEL));
        }
        if let Some(form) = &self.login {
            let mw = 60.min(w - 4);
            let mh = (form.function.inputs.len() as u16 * 2 + 5).min(h - 4);
            let mx = (w - mw) / 2;
            let my = (h - mh) / 2;
            panel(
                &mut nodes,
                &format!(" {} ", form.function.label),
                mx,
                my,
                mw,
                mh,
                true,
            );
            nodes.push(text(
                "Tab changes field · Enter submits · Esc cancels",
                mx + 2,
                my + 1,
                mw - 4,
                1,
                MUTED,
                PANEL,
            ));
            let count = usize::from(mh.saturating_sub(4) / 2);
            let start = form.field.saturating_sub(count.saturating_sub(1));
            for (row, (index, field)) in form
                .function
                .inputs
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
                .enumerate()
            {
                let value = if matches!(field.hint, qmsg_types::InputHint::Secret) {
                    "•".repeat(form.values[index].graphemes(true).count())
                } else {
                    form.values[index].clone()
                };
                nodes.push(text(
                    &format!("{}{}", field.label, if field.required { " *" } else { "" }),
                    mx + 2,
                    my + 2 + row as u16 * 2,
                    mw - 4,
                    1,
                    MUTED,
                    PANEL,
                ));
                nodes.push(text(
                    &format!("{value}{}", if index == form.field { "▏" } else { "" }),
                    mx + 2,
                    my + 3 + row as u16 * 2,
                    mw - 4,
                    1,
                    INK,
                    if index == form.field { SELECT } else { PANEL },
                ));
            }
            if form.function.inputs.is_empty() {
                nodes.push(text(
                    "Press Enter to start sign-in.",
                    mx + 2,
                    my + 2,
                    mw - 4,
                    1,
                    ACCENT,
                    PANEL,
                ));
            }
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
    nodes.push(
        Container::new()
            .style(
                Style::new()
                    .offset(x, y)
                    .width(w)
                    .height(h)
                    .background(PANEL)
                    .border(
                        Border::plain(if focused { ACCENT } else { EDGE })
                            .title(title)
                            .title_color(if focused { ACCENT } else { MUTED }),
                    ),
            )
            .into(),
    );
}
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

struct LoginForm {
    provider: crate::model::ProviderId,
    function: qmsg_types::Function,
    values: Vec<String>,
    field: usize,
}
impl LoginForm {
    fn key(&mut self, key: Key) {
        if self.values.is_empty() {
            return;
        }
        match key {
            Key::Char('\t') | Key::Down => self.field = (self.field + 1) % self.values.len(),
            Key::BackTab | Key::Up => {
                self.field = (self.field + self.values.len() - 1) % self.values.len()
            }
            Key::Backspace => {
                let value = &mut self.values[self.field];
                if let Some((offset, _)) = value.grapheme_indices(true).next_back() {
                    value.truncate(offset);
                }
            }
            Key::Ctrl('u') => self.values[self.field].clear(),
            Key::Char(c) if !c.is_control() && self.values[self.field].len() < 16 * 1024 => {
                self.values[self.field].push(c)
            }
            _ => {}
        }
    }
    fn arguments(&self) -> Result<qmsg_types::Arguments, String> {
        use qmsg_types::{InputHint, Value, ValueType};
        let mut arguments = qmsg_types::Arguments::new();
        for (field, raw) in self.function.inputs.iter().zip(&self.values) {
            if raw.is_empty() && !field.required {
                continue;
            }
            let value = if let InputHint::Choices(choices) = &field.hint {
                choices
                    .iter()
                    .find(|c| c.label == *raw)
                    .map(|c| c.value.clone())
                    .ok_or_else(|| {
                        format!(
                            "{}: choose {}",
                            field.label,
                            choices
                                .iter()
                                .map(|c| c.label.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?
            } else {
                match field.value_type {
                    ValueType::Text => Value::Text(raw.clone()),
                    ValueType::Boolean => Value::Boolean(
                        raw.parse()
                            .map_err(|_| format!("{}: enter true or false", field.label))?,
                    ),
                    ValueType::Integer => Value::Integer(
                        raw.parse()
                            .map_err(|_| format!("{}: enter a whole number", field.label))?,
                    ),
                    _ => {
                        return Err(format!(
                            "{}: this input type is not supported in the terminal yet",
                            field.label
                        ));
                    }
                }
            };
            arguments.insert(field.id.clone(), value);
        }
        self.function.check(&arguments).map_err(|issues| {
            issues
                .into_iter()
                .map(|i| i.message)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        Ok(arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{client::Update, demo_state};
    use qmsg_types::{HistoryPage, Message};

    #[test]
    fn drafts_stay_with_their_channel() {
        let mut state = demo_state();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('é'), &tx);
        state.key(Key::Esc, &tx);
        state.key(Key::Down, &tx);
        assert_eq!(state.draft(), "");
        state.key(Key::Up, &tx);
        assert_eq!(state.draft(), "é");
    }

    #[test]
    fn stale_history_cannot_replace_another_channel() {
        let mut state = demo_state();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        let old = state.selected().unwrap().target;
        state.key(Key::Down, &tx);
        let page = HistoryPage {
            messages: vec![Message::new("old", old.channel.clone(), "user", 0, vec![])],
            next_cursor: None,
        };
        state.update(Update::History(old, page, false), &tx);
        assert!(state.timeline.messages.is_empty());
    }

    #[test]
    fn failed_send_preserves_draft_and_success_clears_only_its_channel() {
        let mut state = demo_state();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        let target = state.selected().unwrap().target;
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('x'), &tx);
        state.update(Update::SendFailed(target.clone(), "offline".into()), &tx);
        assert_eq!(state.draft(), "x");
        state.key(Key::Esc, &tx);
        state.key(Key::Down, &tx);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('y'), &tx);
        state.update(Update::Sent(target), &tx);
        assert_eq!(state.draft(), "y");
        state.key(Key::Esc, &tx);
        state.key(Key::Up, &tx);
        assert_eq!(state.draft(), "");
    }

    #[test]
    fn refreshing_latest_history_keeps_loaded_older_messages() {
        let mut state = demo_state();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        let target = state.selected().unwrap().target;
        let message = |id: &str, time| Message::new(id, target.channel.clone(), "me", time, vec![]);
        state.timeline.messages = vec![message("1", 1), message("2", 2), message("3", 3)];
        state.timeline.cursor = Some("oldest-loaded".into());
        state.update(
            Update::History(
                target.clone(),
                HistoryPage {
                    messages: vec![message("3", 3), message("4", 4)],
                    next_cursor: Some("newest-page".into()),
                },
                false,
            ),
            &tx,
        );
        assert_eq!(
            state
                .timeline
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3", "4"]
        );
        assert_eq!(state.timeline.cursor.as_deref(), Some("oldest-loaded"));
    }

    #[test]
    fn unicode_wrap_and_backspace_keep_graphemes_whole() {
        assert_eq!(wrap("a界e\u{301}", 3), vec!["a界", "e\u{301}"]);
        assert_eq!(wrap("\x1bhello\r", 20), vec!["hello"]);
        let mut state = demo_state();
        let (tx, _) = tokio::sync::mpsc::channel(8);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('e'), &tx);
        state.key(Key::Char('\u{301}'), &tx);
        state.key(Key::Backspace, &tx);
        assert!(state.draft().is_empty());
    }

    #[test]
    fn alt_enter_adds_a_line_without_sending() {
        let mut state = demo_state();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('a'), &tx);
        state.key(Key::Alt('\r'), &tx);
        state.key(Key::Char('b'), &tx);
        assert_eq!(state.draft(), "a\nb");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn all_sizes_render_without_underflow() {
        let state = demo_state();
        for (cols, rows) in [(0, 0), (1, 1), (59, 15), (60, 16), (80, 24), (140, 45)] {
            let size = TerminalSize {
                cols,
                rows,
                ..TerminalSize::default()
            };
            let mut renderer =
                fxcode::Renderer::with_kitty(Vec::new(), size, fxcode::KittyMode::Disabled);
            renderer.draw(&state.view(size)).unwrap();
        }
    }

    #[test]
    fn login_masks_secrets_and_checks_choices() {
        use qmsg_types::*;
        let mut state = demo_state();
        let mut function = Function::action("login", "Sign in", ActionKind::Login);
        let mut field = FunctionInput::new("password", "Password", ValueType::Text);
        field.hint = InputHint::Secret;
        function.inputs.push(field);
        state.login = Some(LoginForm {
            provider: state.providers[0].provider.clone(),
            function,
            values: vec!["dont-display-this".into()],
            field: 0,
        });
        let tree = state.view(TerminalSize {
            cols: 100,
            rows: 30,
            ..TerminalSize::default()
        });
        assert!(!format!("{tree:?}").contains("dont-display-this"));
        assert_eq!(
            state.login.as_ref().unwrap().arguments().unwrap()["password"],
            Value::Text("dont-display-this".into())
        );
    }
}
