use crate::{
    client::{Request, Requests, Update},
    media::{self, Kind, Loaded},
    model::{Group, Provider, Room, Target, Timeline},
};
use fxcode::{Border, Color, Container, ImageFit, Node, Style, TerminalSize, Text, Video};
use qmsg_types::{
    ChannelKind, ChannelRef, Content, Media, MediaSource, Message, ProviderStatus, User,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::Arc,
};
use termion::event::Key;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const PANEL: Color = Color::from_rgba(25, 29, 40, 255);
const INK: Color = Color::from_rgba(220, 225, 239, 255);
const MUTED: Color = Color::from_rgba(137, 148, 172, 255);
const ACCENT: Color = Color::from_rgba(128, 189, 255, 255);
const EDGE: Color = Color::from_rgba(58, 66, 88, 255);
const SELECT: Color = Color::from_rgba(43, 57, 79, 255);
const DRAFT_LIMIT: usize = 64 * 1024;
const FIELD_LIMIT: usize = 16 * 1024;
const QUERY_LIMIT: usize = 256;
/// Messages from one author closer together than this share a header.
const GROUP_MS: u64 = 7 * 60_000;
/// Media loads in flight at once, matching the backend's per-device read quota.
const MEDIA_LOADS: usize = 4;
/// Inline images fit in this many cells, as in tuicord.
const IMAGE_COLS: usize = 64;
const IMAGE_ROWS: usize = 12;
const HELP: &[(&str, &str)] = &[
    ("Tab / Shift+Tab", "Next / previous panel"),
    ("h / l, H / L", "Previous / next panel"),
    ("j / k", "Move in lists; scroll messages"),
    ("↓ / ↑, J / K", "Move in lists; step messages"),
    ("Enter", "Open server, channel or account"),
    ("gg / G", "Oldest loaded / newest messages"),
    ("PgUp / PgDn", "Scroll a page; older history loads"),
    ("i / Esc / ;q", "Insert / normal mode"),
    ("Enter / Alt+Enter", "Send / new line"),
    ("Ctrl+K / +", "Quick switch channels"),
    ("o / p · q", "Open media / play video · close"),
    ("Space ← → r", "Video: pause / seek 5 s / replay"),
    ("Ctrl+R / Ctrl+L", "Refresh / sign in"),
    ("F1 / Ctrl+Q", "Toggle this help / quit"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Servers,
    Channels,
    Messages,
    Accounts,
    Members,
    /// Insert mode: keys edit the draft.
    Compose,
}
/// Highlighted rows in each list. Moving them never opens anything; Enter does.
#[derive(Default, Clone, Copy)]
struct Cursors {
    account: usize,
    server: usize,
    room: usize,
    member: usize,
}
/// The pane sizes the last frame used, so keyboard motions match the screen.
#[derive(Default, Clone, Copy)]
struct Frame {
    width: usize,
    rows: usize,
    members: bool,
    accounts: bool,
    size: TerminalSize,
    /// Timeline messages on screen, as a range.
    visible: (usize, usize),
}
/// Media opened full screen with `o` or `p`.
struct Viewer {
    key: String,
    kind: Kind,
    name: String,
    message: String,
    /// Which media of the message, among its images and videos.
    index: usize,
    video: Option<Video>,
    /// Keeps the downloaded video until the viewer closes.
    file: Option<Arc<media::VideoFile>>,
}
struct Switcher {
    query: String,
    cursor: usize,
}
struct Line {
    text: String,
    color: Color,
    /// Text after `text` in another color, such as a header's time.
    extra: Option<(String, Color)>,
    /// The timeline message this line belongs to.
    message: Option<usize>,
    /// An inline image starting here: its media key, columns and rows.
    image: Option<(String, u16, u16)>,
}
impl Line {
    fn plain(text: String, color: Color, message: Option<usize>) -> Self {
        Self {
            text,
            color,
            extra: None,
            message,
            image: None,
        }
    }
}
pub struct State {
    pub providers: Vec<Provider>,
    account: usize,
    server: usize,
    room: usize,
    cursor: Cursors,
    focus: Focus,
    pub timeline: Timeline,
    /// Keyed by account name and channel, so drafts survive provider restarts.
    drafts: HashMap<(String, ChannelRef), String>,
    /// The message being sent and where to, so a failure can restore it.
    pending: Option<(Target, String)>,
    pub status: String,
    /// Lines scrolled up from the newest message.
    scroll: usize,
    /// How far the last frame could scroll up, so scrolling stops at the top.
    max_scroll: Cell<usize>,
    frame: Cell<Frame>,
    /// The message J / K moved to, by id so older pages keep it.
    focused: Option<String>,
    /// The first `g` of `gg`.
    pending_g: bool,
    help: bool,
    pub demo: bool,
    loading_older: bool,
    login: Option<LoginForm>,
    login_busy: bool,
    switcher: Option<Switcher>,
    /// The terminal shows Kitty graphics, so images and video can play.
    pub graphics: bool,
    images: RefCell<media::Cache>,
    /// Media keys being loaded, with the blob each one reads.
    loading: HashMap<String, Option<String>>,
    /// Blobs the backend already released after a load read them.
    consumed: HashSet<String>,
    /// Inline images loaded since the channel opened. They are not fetched
    /// again after the cache drops them, so a full cache cannot loop.
    loaded: HashSet<String>,
    failed: HashMap<String, String>,
    viewer: Option<Viewer>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            providers: vec![],
            account: 0,
            server: 0,
            room: 0,
            cursor: Cursors::default(),
            focus: Focus::Channels,
            timeline: Timeline::default(),
            drafts: HashMap::new(),
            pending: None,
            status: "Connecting to orchestrator…".into(),
            scroll: 0,
            max_scroll: Cell::new(0),
            frame: Cell::new(Frame::default()),
            focused: None,
            pending_g: false,
            help: false,
            demo: false,
            loading_older: false,
            login: None,
            login_busy: false,
            switcher: None,
            graphics: false,
            images: RefCell::new(media::Cache::default()),
            loading: HashMap::new(),
            consumed: HashSet::new(),
            loaded: HashSet::new(),
            failed: HashMap::new(),
            viewer: None,
        }
    }
}
impl State {
    fn groups(&self) -> Vec<Group<'_>> {
        self.providers
            .get(self.account)
            .map(Provider::groups)
            .unwrap_or_default()
    }
    fn rooms(&self) -> Vec<Room<'_>> {
        self.providers
            .get(self.account)
            .map(|p| p.rooms(self.server))
            .unwrap_or_default()
    }
    pub fn selected(&self) -> Option<Room<'_>> {
        self.rooms().into_iter().nth(self.room)
    }
    fn target(&self) -> Option<Target> {
        self.selected().map(|r| r.target)
    }
    fn draft(&self) -> &str {
        self.target()
            .and_then(|t| self.drafts.get(&t.stable()))
            .map(String::as_str)
            .unwrap_or("")
    }
    fn draft_mut(&mut self) -> Option<&mut String> {
        let target = self.target()?;
        Some(self.drafts.entry(target.stable()).or_default())
    }
    /// Users listed beside the chat: the channel's members, or its server's.
    fn members(&self) -> Vec<&User> {
        let Some(room) = self.selected() else {
            return vec![];
        };
        let mut users: Vec<&User> = match &room.channel.members {
            Some(ids) => ids.iter().filter_map(|id| room.users.get(id)).collect(),
            None => room.users.values().collect(),
        };
        users.sort_by_cached_key(|u| u.name.to_lowercase());
        users
    }
    fn select(&mut self, tx: &Requests) {
        let old = blobs(&self.timeline.messages);
        self.timeline = Timeline::default();
        self.release(old, tx);
        self.consumed.clear();
        self.loaded.clear();
        self.failed.clear();
        self.viewer = None;
        self.scroll = 0;
        self.max_scroll.set(0);
        self.focused = None;
        self.loading_older = false;
        self.sync_cursors();
        if !self.demo
            && let Err(error) = tx.select(self.target())
        {
            self.status = error.into();
        }
    }
    /// Lists without focus show their active row; focused lists keep their place.
    fn sync_cursors(&mut self) {
        if self.focus != Focus::Accounts {
            self.cursor.account = self.account;
        }
        if self.focus != Focus::Servers {
            self.cursor.server = self.server;
        }
        if self.focus != Focus::Channels {
            self.cursor.room = self.room;
        }
        self.cursor.account = self
            .cursor
            .account
            .min(self.providers.len().saturating_sub(1));
        self.cursor.server = self
            .cursor
            .server
            .min(self.groups().len().saturating_sub(1));
        self.cursor.room = self.cursor.room.min(self.rooms().len().saturating_sub(1));
        self.cursor.member = self
            .cursor
            .member
            .min(self.members().len().saturating_sub(1));
    }
    fn queue(&mut self, tx: &Requests, request: Request) -> bool {
        match tx.try_send(request) {
            Ok(()) => true,
            Err(error) => {
                self.status = error.into();
                false
            }
        }
    }
    /// Inserts pasted text into the focused input without sending it.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if let Some(form) = &mut self.login {
            if let Err(error) = form.paste(&text) {
                self.status = error;
            }
            return;
        }
        if let Some(switcher) = &mut self.switcher {
            for c in text.chars().filter(|c| !c.is_control()) {
                if switcher.query.len() + c.len_utf8() > QUERY_LIMIT {
                    break;
                }
                switcher.query.push(c);
            }
            switcher.cursor = 0;
            return;
        }
        self.help = false;
        if self.target().is_some() {
            self.focus = Focus::Compose;
        }
        let Some(draft) = self.draft_mut() else {
            return;
        };
        for c in text.chars() {
            if draft.len() + c.len_utf8() > DRAFT_LIMIT {
                self.status = "Message is too long; paste was cut short".into();
                return;
            }
            if c == '\n' || c == '\t' || !c.is_control() {
                draft.push(c);
            }
        }
    }
    pub fn update(&mut self, update: Update, tx: &Requests) {
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
                // Restarts change a provider's instance, so match by name and ids.
                let old = self.target();
                let account = self
                    .providers
                    .get(self.account)
                    .map(|p| p.provider.name.clone());
                let organization = self
                    .groups()
                    .get(self.server)
                    .map(|g| g.organization.map(str::to_owned));
                self.release(snapshot.blob_ids().into_iter().collect(), tx);
                self.providers = snapshot.providers;
                let found = account
                    .and_then(|name| self.providers.iter().position(|p| p.provider.name == name));
                self.account = found.unwrap_or(0);
                self.server = organization
                    .filter(|_| found.is_some())
                    .and_then(|organization| {
                        self.groups()
                            .iter()
                            .position(|g| g.organization == organization.as_deref())
                    })
                    .unwrap_or(0);
                self.room = old
                    .as_ref()
                    .filter(|_| found.is_some())
                    .and_then(|old| {
                        self.rooms()
                            .iter()
                            .position(|r| r.target.channel == old.channel)
                    })
                    .unwrap_or(0);
                self.sync_cursors();
                // A new instance needs its own Select, and its history replaces the old.
                if self.target() != old {
                    self.select(tx);
                }
            }
            Update::History(target, page, older) => {
                let before = blobs(&self.timeline.messages);
                let mut unused = HashSet::new();
                if self.target() == Some(target) {
                    if older {
                        self.timeline.prepend(page);
                        self.loading_older = false;
                    } else if self.scroll == 0 {
                        self.timeline.refresh(page);
                    } else {
                        unused = blobs(&page.messages);
                    }
                } else {
                    unused = blobs(&page.messages);
                }
                let shown = blobs(&self.timeline.messages);
                unused.extend(before.difference(&shown).cloned());
                // Prepending drops overlapping copies, whose handles would leak.
                self.release(unused.difference(&shown).cloned().collect(), tx);
            }
            Update::Media(key, result) => self.loaded_media(key, result),
            Update::Sent(target) => {
                if self.pending.as_ref().is_some_and(|(t, _)| *t == target) {
                    self.pending = None;
                }
                if self.target() == Some(target) {
                    self.scroll = 0;
                    self.focused = None;
                }
                self.status.clear();
                self.queue(tx, Request::Refresh);
            }
            Update::SendFailed(target, error) => {
                if self.pending.as_ref().is_some_and(|(t, _)| *t == target) {
                    let (target, text) = self.pending.take().unwrap();
                    // Put the message back ahead of anything typed since.
                    let draft = self.drafts.entry(target.stable()).or_default();
                    *draft = if draft.is_empty() {
                        text
                    } else {
                        format!("{text}\n{draft}")
                    };
                }
                self.status = error;
            }
        }
    }
    /// Panels Tab and h / l visit, in screen order. Insert mode is entered with `i`.
    fn ring(&self) -> Vec<Focus> {
        let frame = self.frame.get();
        let mut ring = vec![Focus::Servers, Focus::Channels, Focus::Messages];
        if frame.accounts {
            ring.push(Focus::Accounts);
        }
        if frame.members {
            ring.push(Focus::Members);
        }
        ring
    }
    fn cycle(&mut self, forward: bool) {
        let ring = self.ring();
        let from = if self.focus == Focus::Compose {
            Focus::Messages
        } else {
            self.focus
        };
        let index = ring.iter().position(|f| *f == from).unwrap_or(0);
        let next = if forward {
            (index + 1) % ring.len()
        } else {
            (index + ring.len() - 1) % ring.len()
        };
        self.focus = ring[next];
        self.sync_cursors();
    }
    pub fn key(&mut self, key: Key, tx: &Requests) -> bool {
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
        if self.switcher.is_some() {
            self.switcher_key(key, tx);
            return true;
        }
        if self.viewer.is_some() {
            self.viewer_key(key);
            return true;
        }
        let g = std::mem::take(&mut self.pending_g);
        match key {
            Key::F(1) | Key::Ctrl('/' | '_' | '7') => self.help = true,
            Key::Ctrl('k') => {
                self.switcher = Some(Switcher {
                    query: String::new(),
                    cursor: 0,
                })
            }
            Key::Ctrl('l') if !self.demo && !self.login_busy => self.sign_in(tx),
            Key::Ctrl('r') if !self.demo => {
                self.queue(tx, Request::Refresh);
            }
            Key::PageUp => self.scroll_by(self.page() as isize, tx),
            Key::PageDown => self.scroll_by(-(self.page() as isize), tx),
            _ if self.focus == Focus::Compose => self.insert(key, tx),
            Key::Char('\t') | Key::Char('l') | Key::Char('L') => self.cycle(true),
            Key::BackTab | Key::Char('h') | Key::Char('H') => self.cycle(false),
            Key::Char('i') => {
                if self.target().is_some() {
                    self.focus = Focus::Compose;
                }
            }
            Key::Char('+') => {
                self.switcher = Some(Switcher {
                    query: String::new(),
                    cursor: 0,
                })
            }
            _ if self.focus == Focus::Messages => self.messages_key(key, g, tx),
            Key::Char('j') | Key::Down => self.move_cursor(1),
            Key::Char('k') | Key::Up => self.move_cursor(-1),
            Key::Home => self.move_cursor(isize::MIN / 2),
            Key::End => self.move_cursor(isize::MAX / 2),
            Key::Char('\n') => self.activate(tx),
            _ => {}
        }
        true
    }
    fn sign_in(&mut self, tx: &Requests) {
        let Some(provider) = self.providers.get(self.account) else {
            return;
        };
        if let ProviderStatus::LoginRequired(step) = &provider.status {
            if let Some(function) = &step.function {
                let request = Request::Login(provider.provider.clone(), function.clone());
                self.login_busy = self.queue(tx, request);
            }
        } else {
            self.status = "This account does not need sign-in".into();
        }
    }
    fn insert(&mut self, key: Key, tx: &Requests) {
        match key {
            Key::Esc => self.focus = Focus::Messages,
            Key::Char('\t') => {
                self.focus = Focus::Messages;
                self.cycle(true);
            }
            Key::BackTab => {
                self.focus = Focus::Messages;
                self.cycle(false);
            }
            Key::Char('\n') => self.send(tx),
            key => {
                let Some(draft) = self.draft_mut() else {
                    return;
                };
                match key {
                    Key::Backspace => {
                        if let Some((offset, _)) = draft.grapheme_indices(true).next_back() {
                            draft.truncate(offset);
                        }
                    }
                    Key::Ctrl('u') => draft.clear(),
                    Key::Alt('\n' | '\r') => draft.push('\n'),
                    Key::Char(c) if !c.is_control() && draft.len() < DRAFT_LIMIT => {
                        draft.push(c);
                        // Vim users can leave insert mode by typing ;q.
                        if draft.ends_with(";q") {
                            draft.truncate(draft.len() - 2);
                            self.focus = Focus::Messages;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    fn messages_key(&mut self, key: Key, g: bool, tx: &Requests) {
        match key {
            Key::Char('j') => self.scroll_by(-1, tx),
            Key::Char('k') => self.scroll_by(1, tx),
            Key::Char('J') | Key::Down => self.move_focus(1, tx),
            Key::Char('K') | Key::Up => self.move_focus(-1, tx),
            Key::Char('g') if g => {
                self.focused = None;
                self.scroll = self.max_scroll.get();
                self.older(tx);
            }
            Key::Char('g') => self.pending_g = true,
            Key::Char('G') | Key::End | Key::Esc => {
                self.focused = None;
                self.scroll = 0;
            }
            Key::Home => {
                self.scroll = self.max_scroll.get();
                self.older(tx);
            }
            Key::Char('o') => self.open_media(false),
            Key::Char('p') => self.open_media(true),
            _ => {}
        }
    }
    /// Moves the highlighted row of the focused list.
    fn move_cursor(&mut self, delta: isize) {
        let (count, index) = match self.focus {
            Focus::Accounts => (self.providers.len(), &mut self.cursor.account),
            Focus::Servers => (
                self.providers
                    .get(self.account)
                    .map_or(0, |p| p.groups().len()),
                &mut self.cursor.server,
            ),
            Focus::Channels => (
                self.providers
                    .get(self.account)
                    .map_or(0, |p| p.rooms(self.server).len()),
                &mut self.cursor.room,
            ),
            Focus::Members => (self.members().len(), &mut self.cursor.member),
            Focus::Messages | Focus::Compose => return,
        };
        let last = count.saturating_sub(1) as isize;
        *index = (*index as isize).saturating_add(delta).clamp(0, last) as usize;
    }
    /// Enter opens the highlighted server, channel or account.
    fn activate(&mut self, tx: &Requests) {
        let cursor = self.cursor;
        match self.focus {
            Focus::Accounts if cursor.account != self.account => {
                self.account = cursor.account;
                self.server = 0;
                self.room = 0;
                self.select(tx);
            }
            Focus::Servers if cursor.server != self.server => {
                self.server = cursor.server;
                self.room = 0;
                self.select(tx);
            }
            Focus::Channels if cursor.room != self.room => {
                self.room = cursor.room;
                self.select(tx);
            }
            _ => {}
        }
    }
    fn page(&self) -> usize {
        self.frame.get().rows.saturating_sub(1).max(1)
    }
    /// Positive amounts scroll toward older messages.
    fn scroll_by(&mut self, amount: isize, tx: &Requests) {
        self.scroll =
            (self.scroll as isize + amount).clamp(0, self.max_scroll.get() as isize) as usize;
        if amount > 0 && self.scroll == self.max_scroll.get() {
            self.older(tx);
        }
    }
    /// J / K: steps between messages, starting from the newest.
    fn move_focus(&mut self, delta: isize, tx: &Requests) {
        let count = self.timeline.messages.len();
        if count == 0 {
            return;
        }
        let current = self
            .focused
            .as_ref()
            .and_then(|id| self.timeline.messages.iter().position(|m| m.id == *id));
        let next = match current {
            None => count - 1,
            Some(index) => {
                let next = index as isize + delta;
                if next < 0 {
                    self.older(tx);
                }
                next.clamp(0, count as isize - 1) as usize
            }
        };
        self.focused = Some(self.timeline.messages[next].id.clone());
        self.reveal(next);
    }
    /// Scrolls so the whole message is on screen, or at least its top.
    fn reveal(&mut self, message: usize) {
        let frame = self.frame.get();
        if frame.rows == 0 {
            return;
        }
        let lines = self.lines(self.selected().as_ref(), frame.width);
        let Some(first) = lines.iter().position(|l| l.message == Some(message)) else {
            return;
        };
        let last = lines
            .iter()
            .rposition(|l| l.message == Some(message))
            .unwrap();
        let total = lines.len();
        let max = total.saturating_sub(frame.rows);
        let end = total - self.scroll.min(max);
        let start = end.saturating_sub(frame.rows);
        if last >= end {
            self.scroll = total - last - 1;
        }
        if first < start || last + 1 - first > frame.rows {
            self.scroll = total.saturating_sub(first + frame.rows);
        }
        self.scroll = self.scroll.min(max);
    }
    /// Channels across every account, filtered by each word of the query.
    fn switch_targets(&self, query: &str) -> Vec<(usize, usize, usize, String)> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut targets = vec![];
        for (account, provider) in self.providers.iter().enumerate() {
            for (server, group) in provider.groups().iter().enumerate() {
                for (room, r) in group.rooms(&provider.provider).iter().enumerate() {
                    let label = format!(
                        "{} {} · {} · {}",
                        glyph(&r.channel.kind),
                        r.channel.name,
                        group.name,
                        provider.provider.name
                    );
                    let lower = label.to_lowercase();
                    if words.iter().all(|w| lower.contains(w)) {
                        targets.push((account, server, room, label));
                    }
                }
            }
        }
        targets
    }
    fn switcher_key(&mut self, key: Key, tx: &Requests) {
        let switcher = self.switcher.as_mut().unwrap();
        match key {
            Key::Esc => self.switcher = None,
            Key::Up | Key::Ctrl('p') => switcher.cursor = switcher.cursor.saturating_sub(1),
            Key::Down | Key::Ctrl('n') | Key::Char('\t') => switcher.cursor += 1,
            Key::Backspace => {
                if let Some((offset, _)) = switcher.query.grapheme_indices(true).next_back() {
                    switcher.query.truncate(offset);
                }
                switcher.cursor = 0;
            }
            Key::Ctrl('u') => {
                switcher.query.clear();
                switcher.cursor = 0;
            }
            Key::Char('\n') => {
                let query = switcher.query.clone();
                let cursor = switcher.cursor;
                let targets = self.switch_targets(&query);
                if let Some(&(account, server, room, _)) =
                    targets.get(cursor.min(targets.len().saturating_sub(1)))
                {
                    self.switcher = None;
                    let changed = (account, server, room) != (self.account, self.server, self.room);
                    (self.account, self.server, self.room) = (account, server, room);
                    self.focus = Focus::Messages;
                    if changed {
                        self.select(tx);
                    }
                    self.sync_cursors();
                }
            }
            Key::Char(c) if !c.is_control() && switcher.query.len() < QUERY_LIMIT => {
                switcher.query.push(c);
                switcher.cursor = 0;
            }
            _ => {}
        }
    }
    /// Identifies a message's media across refreshes, which change blob ids.
    fn media_key(&self, message: &Message, index: usize) -> String {
        let account = self
            .providers
            .get(self.account)
            .map_or("", |p| p.provider.name.as_str());
        let channel = &message.channel;
        format!(
            "{account}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{index}",
            channel.organization.as_deref().unwrap_or(""),
            channel.channel,
            message.id
        )
    }
    fn media_label(&self, message: &Message, index: usize, content: &Content) -> String {
        let label = attachment(content);
        let Some((kind, media)) = media::kind(content) else {
            return label;
        };
        if media.metadata.spoiler {
            return match kind {
                Kind::Image => "[spoiler image]".into(),
                Kind::Video => "[spoiler video]".into(),
            };
        }
        let key = self.media_key(message, index);
        if let Some(error) = self.failed.get(&key) {
            format!("{label} · {error}")
        } else if media::expired(media, crate::client::now()) {
            format!("{label} · link expired")
        } else if self.loading.contains_key(&key) {
            format!("{label} · loading…")
        } else if kind == Kind::Video && self.graphics {
            format!("▶ {label}")
        } else {
            label
        }
    }
    /// Gives blob handles back, except those a load is still reading.
    fn release(&mut self, ids: HashSet<String>, tx: &Requests) {
        let reading: HashSet<&String> = self.loading.values().flatten().collect();
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| !reading.contains(id) && !self.consumed.remove(id))
            .collect();
        if !self.demo && !ids.is_empty() {
            // Unreleased handles expire on their own, so a full queue is fine.
            let _ = tx.try_send(Request::Release(ids));
        }
    }
    fn loaded_media(&mut self, key: String, result: Result<Loaded, String>) {
        if let Some(Some(blob)) = self.loading.remove(&key) {
            self.consumed.insert(blob);
        }
        let viewing = self.viewer.as_ref().is_some_and(|v| v.key == key);
        let error = match result {
            Ok(Loaded::Image(image)) => {
                self.loaded.insert(key.clone());
                self.images.get_mut().insert(key, image);
                return;
            }
            Ok(Loaded::Video(file)) if viewing => match Video::from_path(file.path()) {
                Ok(video) => {
                    let viewer = self.viewer.as_mut().unwrap();
                    viewer.video = Some(video);
                    viewer.file = Some(file);
                    return;
                }
                Err(_) => "Could not open video".into(),
            },
            Ok(Loaded::Video(_)) => return,
            Err(error) => error,
        };
        if self.failed.len() >= 256 {
            self.failed.clear();
        }
        self.failed.insert(key, error);
    }
    /// Starts loads for the open viewer and for images on screen. Returns
    /// whether anything started, so labels can show it.
    pub fn tick(&mut self, tx: &Requests) -> bool {
        if !self.graphics {
            return false;
        }
        let idle = |key: &String| !self.loading.contains_key(key) && !self.failed.contains_key(key);
        let mut wanted = vec![];
        if let Some(viewer) = &self.viewer
            && idle(&viewer.key)
            && match viewer.kind {
                Kind::Image => self.images.borrow_mut().get(&viewer.key).is_none(),
                Kind::Video => viewer.file.is_none(),
            }
            && let Some(message) = self
                .timeline
                .messages
                .iter()
                .find(|m| m.id == viewer.message)
            && let Some(&(part, kind)) = media_parts(message).get(viewer.index)
            && let Some((_, media)) = media::kind(&message.content[part])
        {
            wanted.push((viewer.key.clone(), media.clone(), kind));
        }
        let now = crate::client::now();
        let (start, end) = self.frame.get().visible;
        let end = end.min(self.timeline.messages.len());
        for message in &self.timeline.messages[start.min(end)..end] {
            for (index, part) in message.content.iter().enumerate() {
                // Expired links wait for a refresh to bring a new one.
                if let Some((Kind::Image, media)) = media::kind(part)
                    && !media.metadata.spoiler
                    && !media::expired(media, now)
                {
                    let key = self.media_key(message, index);
                    if idle(&key) && !self.loaded.contains(&key) {
                        wanted.push((key, media.clone(), Kind::Image));
                    }
                }
            }
        }
        let mut started = false;
        for (key, media, kind) in wanted {
            if self.loading.len() >= MEDIA_LOADS {
                break;
            }
            started = true;
            if self.demo {
                self.loading.insert(key.clone(), None);
                self.loaded_media(key, media::load_local(&media, kind));
                continue;
            }
            let blob = match &media.source {
                MediaSource::Blob(id) => Some(id.clone()),
                _ => None,
            };
            if tx
                .try_send(Request::Media {
                    key: key.clone(),
                    media,
                    kind,
                })
                .is_ok()
            {
                self.loading.insert(key, blob);
            }
        }
        started
    }
    /// Closes a video the player could not continue.
    pub fn stop_video(&mut self, error: &std::io::Error) {
        self.viewer = None;
        self.status = format!("Video stopped: {error}");
    }
    /// Whether a video is playing, so the caller keeps drawing its frames.
    pub fn playing(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.video.is_some())
    }
    /// `o` opens the focused message's first media; `p` its first video. With
    /// no focused message, the newest message that has one.
    fn open_media(&mut self, video: bool) {
        let messages = &self.timeline.messages;
        let focused = self
            .focused
            .as_ref()
            .and_then(|id| messages.iter().position(|m| m.id == *id));
        let candidates: Vec<usize> = match focused {
            Some(index) => vec![index],
            None => (0..messages.len()).rev().collect(),
        };
        let found = candidates.into_iter().find_map(|message| {
            let index = media_parts(&messages[message])
                .iter()
                .position(|(_, kind)| !video || *kind == Kind::Video)?;
            Some((message, index))
        });
        let Some((message, index)) = found else {
            self.status = if video {
                "No video in this message".into()
            } else {
                "No image or video in this message".into()
            };
            return;
        };
        if !self.graphics {
            self.status = "Images and video need kitty, Ghostty or WezTerm".into();
            return;
        }
        self.show(message, index);
    }
    fn show(&mut self, message: usize, index: usize) {
        let message = &self.timeline.messages[message];
        let (part, kind) = media_parts(message)[index];
        let key = self.media_key(message, part);
        let name = match media::kind(&message.content[part]) {
            Some((
                _,
                Media {
                    name: Some(name), ..
                },
            )) => name.clone(),
            _ if kind == Kind::Video => "Video".into(),
            _ => "Image".into(),
        };
        // Opening again retries a failed load.
        self.failed.remove(&key);
        self.viewer = Some(Viewer {
            key,
            kind,
            name,
            message: message.id.clone(),
            index,
            video: None,
            file: None,
        });
    }
    fn viewer_key(&mut self, key: Key) {
        let viewer = self.viewer.as_ref().unwrap();
        let video = viewer.video.as_ref();
        let step = match key {
            Key::Esc | Key::Char('q' | 'o' | 'p') => {
                self.viewer = None;
                return;
            }
            Key::Char(' ') => {
                video.inspect(|v| v.toggle_pause());
                return;
            }
            Key::Char('r') => {
                if let Some(video) = video {
                    video.controls().seek_to(0.0);
                    video.play();
                }
                return;
            }
            Key::Left | Key::Char('h') => -1,
            Key::Right | Key::Char('l') => 1,
            _ => return,
        };
        match video {
            Some(video) if step < 0 => video.seek_backward(),
            Some(video) => video.seek_forward(),
            // Arrows move between a message's images.
            None if viewer.kind == Kind::Image => {
                let Some(message) = self
                    .timeline
                    .messages
                    .iter()
                    .position(|m| m.id == viewer.message)
                else {
                    return;
                };
                let count = media_parts(&self.timeline.messages[message]).len();
                let next = viewer.index as isize + step;
                if (0..count as isize).contains(&next) {
                    self.show(message, next as usize);
                }
            }
            None => {}
        }
    }
    fn view_media(&self, nodes: &mut Vec<Node>, viewer: &Viewer, w: u16, h: u16) {
        let mut border = Border::plain(ACCENT)
            .title(format!(" {} ", viewer.name))
            .title_color(ACCENT);
        if let Some(video) = &viewer.video {
            border = border.media_controls(video.controls());
        }
        nodes.push(
            Container::new()
                .style(
                    Style::new()
                        .offset(1, 0)
                        .width(w - 2)
                        .height(h)
                        .background(PANEL)
                        .border(border),
                )
                .into(),
        );
        let area = Style::new().offset(2, 1).width(w - 4).height(h - 2);
        let node: Option<Node> = match (&viewer.video, viewer.kind) {
            (Some(video), _) => Some(video.clone().fit(ImageFit::Contain).style(area).into()),
            (None, Kind::Image) => self
                .images
                .borrow_mut()
                .get(&viewer.key)
                .map(|image| image.clone().fit(ImageFit::Contain).style(area).into()),
            (None, Kind::Video) => None,
        };
        match node {
            Some(node) => nodes.push(node),
            None => {
                let status = self
                    .failed
                    .get(&viewer.key)
                    .map_or("Loading…", String::as_str);
                nodes.push(text(status, 3, 1, w - 6, 1, MUTED, PANEL));
            }
        }
    }
    fn send(&mut self, tx: &Requests) {
        let Some(room) = self.selected() else {
            return;
        };
        let text = self.draft();
        if text.trim().is_empty() {
            return;
        }
        let problem = if self.pending.is_some() {
            Some("Still sending the last message…".into())
        } else if self.demo {
            Some("Demo mode · messages are not sent".into())
        } else {
            room.channel
                .check(&[Content::Text(text.into())])
                .err()
                .map(|error| format!("Cannot send: {error:?}"))
        };
        if let Some(problem) = problem {
            self.status = problem;
            return;
        }
        let target = room.target;
        let text = self.drafts.remove(&target.stable()).unwrap_or_default();
        if self.queue(tx, Request::Send(target.clone(), text.clone())) {
            self.pending = Some((target, text));
            self.status = "Sending…".into();
        } else {
            self.drafts.insert(target.stable(), text);
        }
    }
    fn older(&mut self, tx: &Requests) {
        if self.demo || self.loading_older {
            return;
        }
        if let (Some(target), Some(cursor)) = (self.target(), self.timeline.cursor.clone())
            && self.queue(tx, Request::Older(target, cursor))
        {
            self.loading_older = true;
        }
    }
    /// The message pane's content, oldest first.
    fn lines(&self, room: Option<&Room>, width: usize) -> Vec<Line> {
        let mut lines = Vec::new();
        if let Some(provider) = self.providers.get(self.account)
            && let ProviderStatus::LoginRequired(step) = &provider.status
        {
            for line in wrap(&step.message, width) {
                lines.push(Line::plain(line, ACCENT, None));
            }
            if let Some(url) = &step.link {
                for line in wrap(url, width) {
                    lines.push(Line::plain(line, ACCENT, None));
                }
            }
            lines.push(Line::plain("Press Ctrl+L to sign in.".into(), MUTED, None));
            lines.push(Line::plain(String::new(), INK, None));
        }
        let Some(room) = room else {
            if lines.is_empty() {
                lines.push(Line::plain("Select a channel.".into(), MUTED, None));
            }
            return lines;
        };
        if let Some(topic) = &room.channel.topic {
            for line in wrap(topic, width) {
                lines.push(Line::plain(line, MUTED, None));
            }
            lines.push(Line::plain(String::new(), INK, None));
        }
        let messages = &self.timeline.messages;
        if messages.is_empty() {
            lines.push(Line::plain("No messages yet.".into(), MUTED, None));
        }
        let name = |id: &str| {
            room.users
                .get(id)
                .map_or(id, |u| u.name.as_str())
                .to_owned()
        };
        for (index, message) in messages.iter().enumerate() {
            let previous = index.checked_sub(1).map(|i| &messages[i]);
            let grouped = previous.is_some_and(|p| {
                p.author == message.author
                    && message.reply_to.is_none()
                    && message.sent_at.saturating_sub(p.sent_at) < GROUP_MS
            });
            if !grouped {
                if previous.is_some() {
                    lines.push(Line::plain(String::new(), INK, None));
                }
                if let Some(reply) = &message.reply_to {
                    let quoted = messages[..index].iter().rev().find(|m| m.id == *reply);
                    let text = match quoted {
                        Some(m) => format!("╭ {}: {}", name(&m.author), summary(m)),
                        None => "╭ reply".into(),
                    };
                    let mut row = wrap(&text, width);
                    row.truncate(1);
                    for line in row {
                        lines.push(Line::plain(line, MUTED, Some(index)));
                    }
                }
                let minutes = message.sent_at / 60_000;
                let author: String = wrap(&name(&message.author), width.saturating_sub(8))
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                lines.push(Line {
                    image: None,
                    text: author,
                    color: ACCENT,
                    extra: Some((
                        format!(
                            "{:02}:{:02}{}",
                            minutes / 60 % 24,
                            minutes % 60,
                            if message.edited_at.is_some() {
                                " (edited)"
                            } else {
                                ""
                            }
                        ),
                        MUTED,
                    )),
                    message: Some(index),
                });
            }
            for (part_index, part) in message.content.iter().enumerate() {
                let (body, color) = match part {
                    Content::Text(s) => (s.clone(), INK),
                    Content::Formatted(s) => (s.text.clone(), INK),
                    other => (self.media_label(message, part_index, other), MUTED),
                };
                let image = media::kind(part)
                    .filter(|(kind, media)| {
                        *kind == Kind::Image && self.graphics && !media.metadata.spoiler
                    })
                    .and_then(|(_, media)| {
                        let key = self.media_key(message, part_index);
                        let size = self
                            .images
                            .borrow_mut()
                            .get(&key)
                            .and_then(|image| image.dimensions())
                            .or(media.metadata.width.zip(media.metadata.height))?;
                        let (cols, rows) = fit(
                            size,
                            width.min(IMAGE_COLS),
                            IMAGE_ROWS,
                            self.frame.get().size,
                        );
                        Some((key, cols, rows))
                    });
                if let Some((key, cols, rows)) = image {
                    lines.push(Line {
                        image: Some((key, cols, rows)),
                        ..Line::plain(body, color, Some(index))
                    });
                    for _ in 1..rows {
                        lines.push(Line::plain(String::new(), color, Some(index)));
                    }
                    continue;
                }
                for line in wrap(&body, width) {
                    lines.push(Line::plain(line, color, Some(index)));
                }
            }
        }
        lines
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
        // Servers | Channels | Messages over [Accounts | Composer] | Members.
        let servers_w = if w >= 100 { 18 } else { 12 };
        let channels_w = if w >= 100 { 24 } else { 18 };
        let members_w = if w >= 120 { 24 } else { 0 };
        let chat_x = servers_w + channels_w;
        let chat_w = w - chat_x - members_w;
        let composer_h = 5;
        let accounts_w = if chat_w >= 60 { 18 } else { 0 };
        let groups = self.groups();
        let focus = self.focus;

        panel(
            &mut nodes,
            " Servers ",
            0,
            0,
            servers_w,
            h,
            focus == Focus::Servers,
        );
        let rows: Vec<(String, Color, bool)> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                (
                    g.name.to_owned(),
                    if i == self.server { ACCENT } else { INK },
                    true,
                )
            })
            .collect();
        let empty = if self.providers.is_empty() {
            "No accounts"
        } else {
            "None"
        };
        list(
            &mut nodes,
            &rows,
            (1, 1, servers_w - 2, h - 2),
            focus == Focus::Servers,
            self.cursor.server,
            empty,
        );

        let group = groups.get(self.server);
        let title = group.map_or(" Channels ".into(), |g| format!(" {} ", g.name));
        panel(
            &mut nodes,
            &title,
            servers_w,
            0,
            channels_w,
            h,
            focus == Focus::Channels,
        );
        let rows: Vec<(String, Color, bool)> = group
            .map(Group::rows)
            .unwrap_or_default()
            .iter()
            .map(|row| {
                let indent = "  ".repeat(row.depth.min(4));
                match row.room {
                    None => (format!("{indent}▾ {}", row.channel.name), MUTED, false),
                    Some(room) => (
                        format!("{indent}{} {}", glyph(&row.channel.kind), row.channel.name),
                        if room == self.room { ACCENT } else { INK },
                        true,
                    ),
                }
            })
            .collect();
        let cursor_row = nth_selectable(&rows, self.cursor.room);
        list(
            &mut nodes,
            &rows,
            (servers_w + 1, 1, channels_w - 2, h - 2),
            focus == Focus::Channels,
            cursor_row,
            "No channels",
        );

        let selected = self.selected();
        let title = selected
            .as_ref()
            .map(|r| format!(" {} {} ", glyph(&r.channel.kind), r.channel.name))
            .unwrap_or_else(|| " Messages ".into());
        let chat_h = h - composer_h;
        panel(
            &mut nodes,
            &title,
            chat_x,
            0,
            chat_w,
            chat_h,
            focus == Focus::Messages,
        );
        let content_w = chat_w - 4;
        let rows = usize::from(chat_h - 2);
        self.frame.set(Frame {
            width: usize::from(content_w),
            rows,
            members: members_w > 0,
            accounts: accounts_w > 0,
            size,
            ..self.frame.get()
        });
        let lines = self.lines(selected.as_ref(), usize::from(content_w));
        self.max_scroll.set(lines.len().saturating_sub(rows));
        let end = lines.len() - self.scroll.min(self.max_scroll.get());
        let start = end.saturating_sub(rows);
        let focused = self
            .focused
            .as_ref()
            .and_then(|id| self.timeline.messages.iter().position(|m| m.id == *id));
        let window = &lines[start..end];
        let mut frame = self.frame.get();
        frame.visible = (
            window.iter().find_map(|l| l.message).unwrap_or(0),
            window
                .iter()
                .rev()
                .find_map(|l| l.message)
                .map_or(0, |m| m + 1),
        );
        self.frame.set(frame);
        // Kitty images draw over text, so overlays hide them.
        let overlay =
            self.help || self.switcher.is_some() || self.login.is_some() || self.viewer.is_some();
        for (row, line) in window.iter().enumerate() {
            let bg = if focused.is_some() && line.message == focused {
                SELECT
            } else {
                PANEL
            };
            let y = 1 + row as u16;
            let image = line
                .image
                .as_ref()
                .filter(|(_, _, rows)| !overlay && row + usize::from(*rows) <= window.len())
                .and_then(|(key, cols, rows)| {
                    let image = self.images.borrow_mut().get(key)?.clone();
                    Some((image, *cols, *rows))
                });
            if let Some((image, cols, rows)) = image {
                nodes.push(text("", chat_x + 1, y, content_w + 2, 1, line.color, bg));
                nodes.push(
                    image
                        .fit(ImageFit::Contain)
                        .style(Style::new().offset(chat_x + 2, y).width(cols).height(rows))
                        .into(),
                );
                continue;
            }
            nodes.push(text(
                &format!(" {}", line.text),
                chat_x + 1,
                y,
                content_w + 2,
                1,
                line.color,
                bg,
            ));
            if let Some((extra, color)) = &line.extra {
                let x = chat_x + 2 + line.text.width() as u16 + 2;
                let width = (chat_x + 2 + content_w).saturating_sub(x);
                nodes.push(text(extra, x, y, width, 1, *color, bg));
            }
        }
        if accounts_w > 0 {
            panel(
                &mut nodes,
                " Accounts ",
                chat_x,
                chat_h,
                accounts_w,
                composer_h,
                focus == Focus::Accounts,
            );
            let rows: Vec<(String, Color, bool)> = self
                .providers
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let status = match p.status {
                        ProviderStatus::Ready => "●",
                        ProviderStatus::LoginRequired(_) => "!",
                        _ => "○",
                    };
                    (
                        format!("{status} {}", p.provider.name),
                        if i == self.account { ACCENT } else { INK },
                        true,
                    )
                })
                .collect();
            list(
                &mut nodes,
                &rows,
                (chat_x + 1, chat_h + 1, accounts_w - 2, composer_h - 2),
                focus == Focus::Accounts,
                self.cursor.account,
                "None",
            );
        }
        let composer_x = chat_x + accounts_w;
        let composer_w = chat_w - accounts_w;
        let title = if self.pending.is_some() {
            " Sending… "
        } else {
            " Composer "
        };
        panel(
            &mut nodes,
            title,
            composer_x,
            chat_h,
            composer_w,
            composer_h,
            focus == Focus::Compose,
        );
        let inner = composer_w - 4;
        let draft = self.draft();
        if draft.is_empty() && focus != Focus::Compose {
            let placeholder = selected
                .as_ref()
                .map(|r| format!("Message {} {}", glyph(&r.channel.kind), r.channel.name))
                .unwrap_or_default();
            nodes.push(text(
                &placeholder,
                composer_x + 2,
                chat_h + 1,
                inner,
                1,
                MUTED,
                PANEL,
            ));
        } else {
            let cursor = if focus == Focus::Compose { "▏" } else { "" };
            let wrapped = wrap(&format!("{draft}{cursor}"), usize::from(inner));
            let start = wrapped.len().saturating_sub(2);
            nodes.push(text(
                &wrapped[start..].join("\n"),
                composer_x + 2,
                chat_h + 1,
                inner,
                2,
                INK,
                PANEL,
            ));
        }
        let (status, color) = if self.status.is_empty() {
            ("F1 help".to_owned(), MUTED)
        } else {
            (
                wrap(&self.status, usize::from(inner))
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                ACCENT,
            )
        };
        let used = status.width() as u16;
        let x = if self.status.is_empty() {
            composer_x + 2 + inner - used
        } else {
            composer_x + 2
        };
        nodes.push(text(&status, x, chat_h + 3, used, 1, color, PANEL));

        if members_w > 0 {
            let x = w - members_w;
            panel(
                &mut nodes,
                " Members ",
                x,
                0,
                members_w,
                h,
                focus == Focus::Members,
            );
            let rows: Vec<(String, Color, bool)> = self
                .members()
                .iter()
                .map(|u| (u.name.clone(), INK, true))
                .collect();
            list(
                &mut nodes,
                &rows,
                (x + 1, 1, members_w - 2, h - 2),
                focus == Focus::Members,
                self.cursor.member,
                "No members",
            );
        }

        if let Some(viewer) = &self.viewer {
            self.view_media(&mut nodes, viewer, w, h);
        }
        if self.help {
            let mw = 62.min(w - 4);
            let mh = (HELP.len() as u16 + 2).min(h);
            let mx = (w - mw) / 2;
            let my = (h - mh) / 2;
            panel(&mut nodes, " Keyboard help ", mx, my, mw, mh, true);
            let body: Vec<String> = HELP.iter().map(|(k, v)| format!("{k:<18}{v}")).collect();
            nodes.push(text(
                &body.join("\n"),
                mx + 2,
                my + 1,
                mw - 4,
                mh - 2,
                INK,
                PANEL,
            ));
        }
        if let Some(switcher) = &self.switcher {
            let mw = 64.min(w - 4);
            let mh = 16.min(h - 2);
            let mx = (w - mw) / 2;
            let my = (h - mh) / 2;
            panel(&mut nodes, " Switch channel ", mx, my, mw, mh, true);
            nodes.push(text(
                &format!("› {}▏", switcher.query),
                mx + 2,
                my + 1,
                mw - 4,
                1,
                INK,
                PANEL,
            ));
            let targets = self.switch_targets(&switcher.query);
            let rows: Vec<(String, Color, bool)> =
                targets.into_iter().map(|t| (t.3, INK, true)).collect();
            let cursor = switcher.cursor.min(rows.len().saturating_sub(1));
            list(
                &mut nodes,
                &rows,
                (mx + 1, my + 2, mw - 2, mh - 3),
                true,
                cursor,
                "No matches",
            );
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
/// Draws list rows with one cell of padding; the focused list shows its cursor.
fn list(
    nodes: &mut Vec<Node>,
    rows: &[(String, Color, bool)],
    (x, y, w, h): (u16, u16, u16, u16),
    focused: bool,
    cursor: usize,
    empty: &str,
) {
    if rows.is_empty() {
        nodes.push(text(empty, x + 1, y, w.saturating_sub(2), 1, MUTED, PANEL));
        return;
    }
    let visible = usize::from(h);
    let start = cursor.saturating_sub(visible.saturating_sub(1));
    for (row, (index, (label, color, _))) in rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let bg = if focused && index == cursor {
            SELECT
        } else {
            PANEL
        };
        nodes.push(text(
            &format!(" {label}"),
            x,
            y + row as u16,
            w,
            1,
            *color,
            bg,
        ));
    }
}
/// The row index of the `n`th selectable row.
fn nth_selectable(rows: &[(String, Color, bool)], n: usize) -> usize {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| r.2)
        .nth(n)
        .map_or(0, |(i, _)| i)
}
/// Content indices of a message's images and videos.
fn media_parts(message: &Message) -> Vec<(usize, Kind)> {
    message
        .content
        .iter()
        .enumerate()
        .filter_map(|(index, part)| media::kind(part).map(|(kind, _)| (index, kind)))
        .collect()
}
fn blobs(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .flat_map(media::blob_ids)
        .map(str::to_owned)
        .collect()
}
/// The cells an image of `size` pixels takes, never enlarged.
fn fit(size: (u32, u32), cols: usize, rows: usize, terminal: TerminalSize) -> (u16, u16) {
    let cell = |pixels: u16, cells: u16, fallback| {
        if pixels > 0 && cells > 0 {
            f64::from(pixels) / f64::from(cells)
        } else {
            fallback
        }
    };
    let (cols, rows) = (cols.max(1) as f64, rows.max(1) as f64);
    let width = f64::from(size.0.max(1)) / cell(terminal.pixel_width, terminal.cols, 8.0);
    let height = f64::from(size.1.max(1)) / cell(terminal.pixel_height, terminal.rows, 16.0);
    let scale = (cols / width).min(rows / height).min(1.0);
    (
        (width * scale).ceil().clamp(1.0, cols) as u16,
        (height * scale).ceil().clamp(1.0, rows) as u16,
    )
}
fn glyph(kind: &ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Direct | ChannelKind::Group => "@",
        ChannelKind::Voice | ChannelKind::Video => "♪",
        ChannelKind::Forum => "≡",
        ChannelKind::Thread => "↳",
        _ => "#",
    }
}
fn attachment(content: &Content) -> String {
    let (kind, media) = match content {
        Content::Image(m) => ("image", m),
        Content::Video(m) => ("video", m),
        Content::Audio(m) => ("audio", m),
        Content::File(m) => ("file", m),
        Content::Custom { kind, .. } => return format!("[{kind}]"),
        Content::Text(_) | Content::Formatted(_) => return String::new(),
    };
    format!("[{kind}: {}]", media.name.as_deref().unwrap_or("unnamed"))
}
/// The first line of a message, for reply previews.
fn summary(message: &Message) -> String {
    message
        .content
        .iter()
        .map(|part| match part {
            Content::Text(s) => s.clone(),
            Content::Formatted(s) => s.text.clone(),
            other => attachment(other),
        })
        .find(|s| !s.trim().is_empty())
        .and_then(|s| s.lines().next().map(str::to_owned))
        .unwrap_or_default()
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
/// Wraps at spaces, breaking only words wider than a line. Drops control characters.
pub fn wrap(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut lines = Vec::new();
    for line in value.split('\n') {
        let mut row = Row {
            text: String::new(),
            used: 0,
            width,
        };
        let mut word = Vec::new();
        for grapheme in line.graphemes(true) {
            if grapheme.chars().any(char::is_control) {
                continue;
            }
            let n = grapheme.width();
            if n > width {
                continue;
            }
            if grapheme.chars().all(char::is_whitespace) {
                row.place(&mut word, &mut lines);
                if row.used + n > width {
                    row.flush(&mut lines);
                } else {
                    row.text.push_str(grapheme);
                    row.used += n;
                }
            } else {
                word.push((grapheme, n));
            }
        }
        row.place(&mut word, &mut lines);
        lines.push(row.text);
    }
    lines
}
struct Row {
    text: String,
    used: usize,
    width: usize,
}
impl Row {
    fn flush(&mut self, lines: &mut Vec<String>) {
        lines.push(self.text.trim_end().to_owned());
        self.text.clear();
        self.used = 0;
    }
    /// Adds a word, starting a new line first when it fits on one.
    fn place(&mut self, word: &mut Vec<(&str, usize)>, lines: &mut Vec<String>) {
        let total: usize = word.iter().map(|g| g.1).sum();
        if self.used > 0 && self.used + total > self.width && total <= self.width {
            self.flush(lines);
        }
        for (grapheme, n) in word.drain(..) {
            if self.used + n > self.width {
                self.flush(lines);
            }
            self.text.push_str(grapheme);
            self.used += n;
        }
    }
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
            Key::Char(c) if !c.is_control() && self.values[self.field].len() < FIELD_LIMIT => {
                self.values[self.field].push(c)
            }
            _ => {}
        }
    }
    fn paste(&mut self, text: &str) -> Result<(), String> {
        let Some(field) = self.function.inputs.get(self.field) else {
            return Ok(());
        };
        let multiline = matches!(field.hint, qmsg_types::InputHint::Multiline);
        // A copied token often ends in a newline; any other line break is a mistake.
        let text = if multiline {
            text
        } else {
            text.trim_end_matches('\n')
        };
        if !multiline && text.contains('\n') {
            return Err(format!("{} takes a single line", field.label));
        }
        let value = &mut self.values[self.field];
        for c in text.chars() {
            if value.len() + c.len_utf8() > FIELD_LIMIT {
                break;
            }
            if c == '\n' || !c.is_control() {
                value.push(c);
            }
        }
        Ok(())
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
    use crate::{
        client::{Requests, Update, requests},
        demo_state,
        model::Snapshot,
    };
    use qmsg_types::HistoryPage;

    /// Sends each character as a key; Esc is `\x1b`.
    fn press(state: &mut State, tx: &Requests, keys: &str) {
        for c in keys.chars() {
            state.key(if c == '\x1b' { Key::Esc } else { Key::Char(c) }, tx);
        }
    }

    #[test]
    fn drafts_stay_with_their_channel() {
        let mut state = demo_state();
        let (tx, _rx, _selection) = requests(8);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('é'), &tx);
        press(&mut state, &tx, "\x1bhj\n");
        assert_eq!(state.selected().unwrap().channel.name, "design");
        assert_eq!(state.draft(), "");
        press(&mut state, &tx, "k\n");
        assert_eq!(state.draft(), "é");
    }

    #[test]
    fn stale_history_cannot_replace_another_channel() {
        let mut state = demo_state();
        let (tx, _rx, _selection) = requests(8);
        let old = state.selected().unwrap().target;
        press(&mut state, &tx, "j\n");
        let page = HistoryPage {
            messages: vec![Message::new("old", old.channel.clone(), "user", 0, vec![])],
            next_cursor: None,
        };
        state.update(Update::History(old, page, false), &tx);
        assert!(state.timeline.messages.is_empty());
    }

    /// The demo state, but able to send text so requests reach the queue.
    fn live() -> State {
        let mut state = demo_state();
        state.demo = false;
        for channel in state.providers[0].organizations[0]
            .scope
            .channels
            .values_mut()
        {
            *channel = channel.clone().accepting([qmsg_types::ContentKind::Text]);
        }
        state
    }

    fn sent(rx: &mut tokio::sync::mpsc::Receiver<Request>) -> Vec<String> {
        let mut sent = vec![];
        while let Ok(request) = rx.try_recv() {
            if let Request::Send(_, text) = request {
                sent.push(text);
            }
        }
        sent
    }

    #[test]
    fn typing_while_sending_starts_a_new_draft_that_success_keeps() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(8);
        let target = state.selected().unwrap().target;
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('a'), &tx);
        state.key(Key::Char('\n'), &tx);
        assert_eq!(sent(&mut rx), vec!["a"]);
        assert_eq!(state.draft(), "");
        state.key(Key::Char('b'), &tx);
        state.key(Key::Char('\n'), &tx);
        assert!(sent(&mut rx).is_empty(), "second send while one is pending");
        state.update(Update::Sent(target), &tx);
        assert!(state.pending.is_none());
        assert_eq!(state.draft(), "b");
    }

    #[test]
    fn failed_send_restores_text_ahead_of_newer_draft_in_its_channel() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(8);
        let target = state.selected().unwrap().target;
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('a'), &tx);
        state.key(Key::Char('\n'), &tx);
        state.key(Key::Char('b'), &tx);
        press(&mut state, &tx, "\x1bhj\niz");
        state.update(Update::SendFailed(target, "offline".into()), &tx);
        assert_eq!(state.status, "offline");
        assert_eq!(state.draft(), "z");
        press(&mut state, &tx, "\x1bhk\n");
        assert_eq!(state.draft(), "a\nb");
        assert!(state.pending.is_none());
    }

    #[test]
    fn full_queue_keeps_the_draft() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(1);
        tx.try_send(Request::Refresh).unwrap();
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('a'), &tx);
        state.key(Key::Char('\n'), &tx);
        assert!(state.pending.is_none());
        assert_eq!(state.draft(), "a");
    }

    #[test]
    fn paste_inserts_lines_without_sending_even_while_pending() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(8);
        state.paste("one\r\ntwo\x1b");
        assert_eq!(state.draft(), "one\ntwo");
        assert!(sent(&mut rx).is_empty());
        state.key(Key::Char('\n'), &tx);
        assert_eq!(sent(&mut rx), vec!["one\ntwo"]);
        state.paste("three\nfour");
        assert_eq!(state.draft(), "three\nfour");
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn login_paste_keeps_single_line_fields_single_line() {
        use qmsg_types::*;
        let mut state = demo_state();
        let mut function = Function::action("login", "Sign in", ActionKind::Login);
        function
            .inputs
            .push(FunctionInput::new("token", "Token", ValueType::Text));
        let mut notes = FunctionInput::new("notes", "Notes", ValueType::Text);
        notes.hint = InputHint::Multiline;
        function.inputs.push(notes);
        state.login = Some(LoginForm {
            provider: state.providers[0].provider.clone(),
            function,
            values: vec![String::new(); 2],
            field: 0,
        });
        state.paste("abc\n");
        state.paste("d\ne");
        assert_eq!(state.status, "Token takes a single line");
        state.login.as_mut().unwrap().field = 1;
        state.paste("d\r\ne");
        assert_eq!(state.login.as_ref().unwrap().values, vec!["abc", "d\ne"]);
        assert!(state.draft().is_empty());
    }

    #[test]
    fn sent_returns_to_latest_only_in_its_channel() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(8);
        let size = TerminalSize {
            cols: 60,
            rows: 16,
            ..TerminalSize::default()
        };
        let mut other = state.selected().unwrap().target;
        other.channel.channel = "random".into();
        state.view(size);
        state.focus = Focus::Messages;
        state.key(Key::Char('k'), &tx);
        assert_eq!(state.scroll, 1);
        state.update(Update::Sent(other), &tx);
        assert_eq!(state.scroll, 1);
        state.update(Update::Sent(state.selected().unwrap().target), &tx);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn scrolling_stops_at_the_oldest_rendered_line() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(8);
        state.view(TerminalSize {
            cols: 60,
            rows: 16,
            ..TerminalSize::default()
        });
        let top = state.max_scroll.get();
        assert!(top > 0);
        state.focus = Focus::Messages;
        for _ in 0..20 {
            state.key(Key::PageUp, &tx);
        }
        assert_eq!(state.scroll, top);
        state.key(Key::PageDown, &tx);
        assert_eq!(state.scroll, top.saturating_sub(state.page()));
    }

    #[test]
    fn provider_restart_keeps_selection_and_draft() {
        let mut state = live();
        let (tx, _rx, mut selection) = requests(8);
        press(&mut state, &tx, "j\n");
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('x'), &tx);
        let old = state.selected().unwrap().target;
        let mut providers = state.providers.clone();
        providers[0].provider.instance = 2;
        state.update(Update::Snapshot(Snapshot { providers }), &tx);
        let new = state.selected().unwrap().target;
        assert_eq!(new.channel, old.channel);
        assert_eq!(new.provider.instance, 2);
        assert_eq!(state.draft(), "x");
        assert_eq!(selection.borrow_and_update().as_ref(), Some(&new));
        // History from before the restart no longer applies.
        let page = HistoryPage {
            messages: vec![Message::new("old", old.channel.clone(), "u", 0, vec![])],
            next_cursor: None,
        };
        state.update(Update::History(old, page, false), &tx);
        assert!(state.timeline.messages.is_empty());
    }

    #[test]
    fn authors_show_names_from_the_channel_scope() {
        let mut state = demo_state();
        state.providers[0].organizations[0].scope.users.insert(
            "Ana".into(),
            qmsg_types::User {
                id: "Ana".into(),
                name: "Ana Lima".into(),
                username: None,
                avatar: None,
            },
        );
        let tree = state.view(TerminalSize {
            cols: 100,
            rows: 40,
            ..TerminalSize::default()
        });
        assert!(format!("{tree:?}").contains("Ana Lima"));
    }

    #[test]
    fn refreshing_latest_history_keeps_loaded_older_messages() {
        let mut state = demo_state();
        let (tx, _rx, _selection) = requests(8);
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
        assert_eq!(wrap("hello big world", 9), vec!["hello big", "world"]);
        assert_eq!(wrap("ab cdefghijk", 4), vec!["ab c", "defg", "hijk"]);
        assert_eq!(wrap("one\n\ntwo  ", 10), vec!["one", "", "two  "]);
        let mut state = demo_state();
        let (tx, _rx, _selection) = requests(8);
        state.key(Key::Char('i'), &tx);
        state.key(Key::Char('e'), &tx);
        state.key(Key::Char('\u{301}'), &tx);
        state.key(Key::Backspace, &tx);
        assert!(state.draft().is_empty());
    }

    #[test]
    fn alt_enter_adds_a_line_without_sending() {
        let mut state = demo_state();
        let (tx, mut rx, _selection) = requests(8);
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

    fn size(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize {
            cols,
            rows,
            ..TerminalSize::default()
        }
    }

    #[test]
    fn list_motion_highlights_and_only_enter_opens() {
        let mut state = live();
        let (tx, _rx, mut selection) = requests(8);
        assert_eq!(state.focus, Focus::Channels);
        press(&mut state, &tx, "jj");
        assert_eq!(state.selected().unwrap().channel.name, "general");
        assert!(!selection.has_changed().unwrap());
        press(&mut state, &tx, "k\n");
        assert_eq!(state.selected().unwrap().channel.name, "design");
        let picked = selection.borrow_and_update().clone().unwrap();
        assert_eq!(picked.channel.channel, "design");
    }

    #[test]
    fn normal_mode_keys_never_type_into_the_draft() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(8);
        state.view(size(100, 30));
        press(&mut state, &tx, "lxyz");
        assert_eq!(state.focus, Focus::Messages);
        assert_eq!(state.draft(), "");
        press(&mut state, &tx, "iab\x1bxz");
        assert_eq!(state.draft(), "ab");
        assert_eq!(state.focus, Focus::Messages);
        press(&mut state, &tx, "icd;q");
        assert_eq!(state.draft(), "abcd");
        assert_eq!(state.focus, Focus::Messages);
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn panel_ring_follows_the_visible_panes() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(8);
        let visit = |state: &mut State, cols| {
            state.view(size(cols, 30));
            state.focus = Focus::Servers;
            let mut seen = vec![];
            for _ in 0..6 {
                state.key(Key::Char('l'), &tx);
                seen.push(state.focus);
            }
            seen
        };
        use Focus::*;
        assert_eq!(
            visit(&mut state, 80),
            [Channels, Messages, Servers, Channels, Messages, Servers]
        );
        assert_eq!(
            visit(&mut state, 140),
            [Channels, Messages, Accounts, Members, Servers, Channels]
        );
        state.key(Key::Char('H'), &tx);
        assert_eq!(state.focus, Servers);
        state.key(Key::BackTab, &tx);
        assert_eq!(state.focus, Members);
    }

    #[test]
    fn message_focus_steps_and_keeps_the_message_on_screen() {
        let mut state = live();
        let (tx, _rx, _selection) = requests(8);
        let channel = state.selected().unwrap().target.channel;
        state.timeline.messages = (0..30)
            .map(|i| {
                Message::new(
                    i.to_string(),
                    channel.clone(),
                    if i % 2 == 0 { "Ana" } else { "Claudio" },
                    i * 60_000,
                    vec![Content::Text(format!("message {i}"))],
                )
            })
            .collect();
        state.view(size(80, 16));
        state.focus = Focus::Messages;
        state.key(Key::Char('K'), &tx);
        assert_eq!(state.focused.as_deref(), Some("29"));
        assert_eq!(state.scroll, 0);
        for _ in 0..10 {
            state.key(Key::Char('K'), &tx);
        }
        assert_eq!(state.focused.as_deref(), Some("19"));
        let frame = state.frame.get();
        let lines = state.lines(state.selected().as_ref(), frame.width);
        let first = lines.iter().position(|l| l.message == Some(19)).unwrap();
        let end = lines.len() - state.scroll;
        assert!(first >= end - frame.rows && first < end);
        state.key(Key::Char('J'), &tx);
        assert_eq!(state.focused.as_deref(), Some("20"));
        state.key(Key::Char('G'), &tx);
        assert_eq!((state.focused.as_deref(), state.scroll), (None, 0));
    }

    #[test]
    fn gg_jumps_to_the_oldest_loaded_line_and_asks_for_more() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(8);
        state.timeline.cursor = Some("older".into());
        state.view(size(60, 16));
        state.focus = Focus::Messages;
        press(&mut state, &tx, "g");
        assert_eq!(state.scroll, 0);
        press(&mut state, &tx, "g");
        assert_eq!(state.scroll, state.max_scroll.get());
        assert!(matches!(rx.try_recv(), Ok(Request::Older(_, cursor)) if cursor == "older"));
        // A lone g followed by another key does not jump later.
        press(&mut state, &tx, "Ggjg");
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn quick_switcher_opens_a_channel_in_another_account() {
        let mut state = live();
        let (tx, _rx, mut selection) = requests(8);
        let mut other = state.providers[0].clone();
        other.provider.name = "Work".into();
        other.organizations[0].name = "Office".into();
        state.providers.push(other);
        state.key(Key::Ctrl('k'), &tx);
        state.paste("rand office");
        state.key(Key::Char('\n'), &tx);
        let room = state.selected().unwrap();
        assert_eq!(room.channel.name, "random");
        assert_eq!(room.target.provider.name, "Work");
        assert!(state.switcher.is_none());
        assert_eq!(
            selection
                .borrow_and_update()
                .as_ref()
                .unwrap()
                .provider
                .name,
            "Work"
        );
        state.key(Key::Ctrl('k'), &tx);
        press(&mut state, &tx, "nothing-matches\n");
        assert!(state.switcher.is_some());
        state.key(Key::Esc, &tx);
        assert!(state.switcher.is_none());
    }

    #[test]
    fn help_fits_the_smallest_terminal() {
        let mut state = demo_state();
        let (tx, _rx, _selection) = requests(8);
        state.key(Key::F(1), &tx);
        let nodes = state.view(size(60, 16));
        let body = nodes
            .iter()
            .find_map(|node| match node {
                Node::Text(t) if t.content.starts_with(HELP[0].0) => Some(t),
                _ => None,
            })
            .expect("help text is drawn");
        let (w, h) = (body.style.width.unwrap(), body.style.height.unwrap());
        assert_eq!(body.content.lines().count(), HELP.len());
        assert!(HELP.len() <= usize::from(h), "{} rows in {h}", HELP.len());
        for line in body.content.lines() {
            assert!(line.width() <= usize::from(w), "{line:?} is wider than {w}");
        }
        state.key(Key::Char('x'), &tx);
        assert!(!state.help);
    }

    fn picture(source: MediaSource, width: u32, height: u32) -> Media {
        let mut media = Media {
            name: Some("picture.png".into()),
            mime: Some("image/png".into()),
            size: None,
            source,
            metadata: Default::default(),
        };
        media.metadata.width = Some(width);
        media.metadata.height = Some(height);
        media
    }

    fn with_media(state: &mut State, sources: Vec<Content>) {
        let channel = state.selected().unwrap().target.channel;
        state.timeline.messages = sources
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                Message::new(
                    i.to_string(),
                    channel.clone(),
                    "Ana",
                    i as u64,
                    vec![content],
                )
            })
            .collect();
    }

    fn drain(rx: &mut tokio::sync::mpsc::Receiver<Request>) -> (Vec<(String, Media)>, Vec<String>) {
        let (mut loads, mut released) = (vec![], vec![]);
        while let Ok(request) = rx.try_recv() {
            match request {
                Request::Media { key, media, .. } => loads.push((key, media)),
                Request::Release(ids) => released.extend(ids),
                _ => {}
            }
        }
        (loads, released)
    }

    fn rgba() -> fxcode::Image {
        fxcode::Image::from_rgba(2, 2, vec![0; 16]).unwrap()
    }

    #[test]
    fn visible_images_load_four_at_a_time_and_never_loop() {
        let mut state = live();
        state.graphics = true;
        let (tx, mut rx, _selection) = requests(16);
        let images = (0..6)
            .map(|i| {
                Content::Image(picture(
                    MediaSource::Url(format!("https://cdn.test/{i}.png")),
                    40,
                    20,
                ))
            })
            .collect();
        with_media(&mut state, images);
        state.view(size(100, 60));
        assert!(state.tick(&tx));
        let (loads, _) = drain(&mut rx);
        assert_eq!(loads.len(), MEDIA_LOADS);
        assert!(!state.tick(&tx), "the quota is full");
        state.update(
            Update::Media(loads[0].0.clone(), Ok(Loaded::Image(rgba()))),
            &tx,
        );
        state.update(
            Update::Media(loads[1].0.clone(), Err("Download failed".into())),
            &tx,
        );
        state.tick(&tx);
        let (more, _) = drain(&mut rx);
        assert_eq!(more.len(), 2);
        assert!(
            more.iter()
                .all(|(key, _)| loads.iter().all(|(old, _)| old != key))
        );
        assert!(format!("{:?}", state.view(size(100, 60))).contains("Download failed"));
        // An image the cache dropped stays a placeholder instead of loading again.
        state.images = RefCell::new(media::Cache::default());
        for (key, _) in loads.iter().skip(2).chain(&more) {
            state.update(Update::Media(key.clone(), Ok(Loaded::Image(rgba()))), &tx);
        }
        state.images = RefCell::new(media::Cache::default());
        state.view(size(100, 60));
        assert!(!state.tick(&tx));
    }

    #[test]
    fn expired_links_wait_for_a_refreshed_message() {
        let mut state = live();
        state.graphics = true;
        let (tx, mut rx, _selection) = requests(16);
        let target = state.selected().unwrap().target;
        let link = |expires| {
            let mut media = picture(
                MediaSource::Url("https://cdn.test/a.png?ex=1".into()),
                40,
                20,
            );
            media.metadata.expires_at_ms = Some(expires);
            Content::Image(media)
        };
        with_media(&mut state, vec![link(1)]);
        assert!(format!("{:?}", state.view(size(100, 30))).contains("link expired"));
        assert!(!state.tick(&tx));
        let page = HistoryPage {
            messages: vec![Message::new(
                "0",
                target.channel.clone(),
                "Ana",
                0,
                vec![link(u64::MAX)],
            )],
            next_cursor: None,
        };
        state.update(Update::History(target, page, false), &tx);
        state.view(size(100, 30));
        assert!(state.tick(&tx));
        assert_eq!(drain(&mut rx).0.len(), 1);
    }

    #[test]
    fn terminals_without_graphics_show_names_and_load_nothing() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(16);
        with_media(
            &mut state,
            vec![Content::Image(picture(
                MediaSource::Url("https://cdn.test/a.png".into()),
                40,
                20,
            ))],
        );
        let tree = format!("{:?}", state.view(size(100, 30)));
        assert!(tree.contains("[image: picture.png]"));
        assert!(!state.tick(&tx));
        state.focus = Focus::Messages;
        state.key(Key::Char('o'), &tx);
        assert!(state.viewer.is_none());
        assert!(state.status.contains("need kitty"));
        assert!(drain(&mut rx).0.is_empty());
    }

    #[test]
    fn blobs_are_released_when_unused_but_never_while_read() {
        let mut state = live();
        state.graphics = true;
        let (tx, mut rx, _selection) = requests(16);
        let target = state.selected().unwrap().target;
        let blob = |id: &str| Content::Image(picture(MediaSource::Blob(id.into()), 40, 20));
        with_media(&mut state, vec![blob("a")]);
        state.view(size(100, 30));
        state.tick(&tx);
        assert_eq!(drain(&mut rx).0.len(), 1);
        let page = |id: &str| HistoryPage {
            messages: vec![Message::new(
                "0",
                target.channel.clone(),
                "Ana",
                0,
                vec![blob(id)],
            )],
            next_cursor: None,
        };
        state.update(Update::History(target.clone(), page("b"), false), &tx);
        assert!(drain(&mut rx).1.is_empty(), "a is still being read");
        let key = state.media_key(&state.timeline.messages[0], 0);
        state.update(Update::Media(key, Ok(Loaded::Image(rgba()))), &tx);
        state.update(Update::History(target.clone(), page("c"), false), &tx);
        assert_eq!(
            drain(&mut rx).1,
            ["b"],
            "the backend released a after reading it"
        );
        let mut other = target.clone();
        other.channel.channel = "random".into();
        state.update(Update::History(other, page("d"), false), &tx);
        assert_eq!(drain(&mut rx).1, ["d"]);
        state.key(Key::Char('j'), &tx);
        state.key(Key::Char('\n'), &tx);
        assert_eq!(
            drain(&mut rx).1,
            ["c"],
            "leaving the channel releases its blobs"
        );
        // A finished read on screen is not released a second time.
        let target = state.selected().unwrap().target;
        with_media(&mut state, vec![blob("x")]);
        state.view(size(100, 30));
        state.tick(&tx);
        let (loads, _) = drain(&mut rx);
        state.update(
            Update::Media(loads[0].0.clone(), Ok(Loaded::Image(rgba()))),
            &tx,
        );
        state.update(Update::History(target, page("y"), false), &tx);
        assert!(drain(&mut rx).1.is_empty());
    }

    #[test]
    fn snapshot_icons_and_avatars_are_released() {
        let mut state = live();
        let (tx, mut rx, _selection) = requests(16);
        let mut providers = state.providers.clone();
        providers[0].organizations[0].icon =
            Some(Box::new(picture(MediaSource::Blob("icon".into()), 1, 1)));
        providers[0].organizations[0].scope.users.insert(
            "Ana".into(),
            User {
                id: "Ana".into(),
                name: "Ana".into(),
                username: None,
                avatar: Some(Box::new(picture(MediaSource::Blob("avatar".into()), 1, 1))),
            },
        );
        state.update(Update::Snapshot(Snapshot { providers }), &tx);
        let mut released = drain(&mut rx).1;
        released.sort();
        assert_eq!(released, ["avatar", "icon"]);
    }

    #[test]
    fn viewer_plays_the_focused_video_and_deletes_it_on_close() {
        let mut state = live();
        state.graphics = true;
        let (tx, mut rx, _selection) = requests(16);
        let mut clip = picture(MediaSource::Url("https://cdn.test/clip.mp4".into()), 64, 36);
        clip.mime = Some("video/mp4".into());
        with_media(
            &mut state,
            vec![Content::Video(clip), Content::Text("no media".into())],
        );
        state.view(size(100, 30));
        state.focus = Focus::Messages;
        press(&mut state, &tx, "K");
        state.key(Key::Char('p'), &tx);
        assert_eq!(state.status, "No video in this message");
        press(&mut state, &tx, "Kp");
        assert!(state.viewer.is_some() && !state.playing());
        state.tick(&tx);
        let (loads, _) = drain(&mut rx);
        assert_eq!(loads.len(), 1);
        let file =
            Arc::new(media::VideoFile::create(b"not really a video", Some("video/mp4")).unwrap());
        let path = file.path().to_owned();
        state.update(
            Update::Media(loads[0].0.clone(), Ok(Loaded::Video(file))),
            &tx,
        );
        assert!(state.playing());
        state.view(size(100, 30));
        state.key(Key::Char(' '), &tx);
        assert!(
            state
                .viewer
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .is_paused()
        );
        state.key(Key::Char('q'), &tx);
        assert!(state.viewer.is_none() && !path.exists());
    }

    #[test]
    fn inline_images_reserve_rows_from_their_dimensions() {
        let mut state = live();
        state.graphics = true;
        with_media(
            &mut state,
            vec![Content::Image(picture(
                MediaSource::Url("https://cdn.test/a.png".into()),
                800,
                450,
            ))],
        );
        state.frame.set(Frame {
            size: TerminalSize {
                cols: 100,
                rows: 40,
                pixel_width: 1000,
                pixel_height: 800,
            },
            ..Frame::default()
        });
        let lines = state.lines(state.selected().as_ref(), 60);
        let (index, (_, cols, rows)) = lines
            .iter()
            .enumerate()
            .find_map(|(i, l)| l.image.clone().map(|image| (i, image)))
            .unwrap();
        assert_eq!((cols, rows), (43, 12));
        assert_eq!(lines.len(), index + 12);
        assert_eq!(
            fit((4, 4), 60, 12, TerminalSize::default()),
            (1, 1),
            "small images stay small"
        );
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
