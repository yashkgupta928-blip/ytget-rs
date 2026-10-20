//! Application state and input handling. Rendering lives in `ui.rs`; the UI fills in
//! the hit map (`App::hits`) on every frame and this module resolves clicks against it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::config::{Config, DEFAULT_FFMPEG, DEFAULT_YTDLP, MAX_CONCURRENT_LIMIT, resolve_binary};
use crate::download::{self, Exit, Msg, Tag};
use crate::text::char_index_at;
use crate::types::*;
use crate::ytdlp::{self, DlOptions};

const LOG_LIMIT: usize = 300;
const WHEEL_LINES: isize = 3;

// ---------------------------------------------------------------------------------------
// Identifiers for things on screen
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Download,
    Downloads,
    Settings,
    Help,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Download, Tab::Downloads, Tab::Settings, Tab::Help];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Download => "Download",
            Tab::Downloads => "Downloads",
            Tab::Settings => "Settings",
            Tab::Help => "Help",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn next(self) -> Tab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Rows of the form on the Download tab, in keyboard focus order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormRow {
    Url,
    Mode,
    VideoQuality,
    VideoCodec,
    AudioQuality,
    Container,
    Start,
}

impl FormRow {
    pub const ALL: [FormRow; 7] = [
        FormRow::Url,
        FormRow::Mode,
        FormRow::VideoQuality,
        FormRow::VideoCodec,
        FormRow::AudioQuality,
        FormRow::Container,
        FormRow::Start,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    pub fn is_picker(self) -> bool {
        !matches!(self, FormRow::Url | FormRow::Start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    OutputDir,
    Mode,
    VideoQuality,
    VideoCodec,
    AudioQuality,
    Container,
    MaxConcurrent,
    EmbedThumbnail,
    EmbedMetadata,
    WriteSubs,
    Playlist,
    CookiesBrowser,
    ExtraArgs,
    YtdlpPath,
    FfmpegPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Text,
    Choice,
    Bool,
    Int,
}

impl SettingKey {
    pub const ALL: [SettingKey; 15] = [
        SettingKey::OutputDir,
        SettingKey::Mode,
        SettingKey::VideoQuality,
        SettingKey::VideoCodec,
        SettingKey::AudioQuality,
        SettingKey::Container,
        SettingKey::MaxConcurrent,
        SettingKey::EmbedThumbnail,
        SettingKey::EmbedMetadata,
        SettingKey::WriteSubs,
        SettingKey::Playlist,
        SettingKey::CookiesBrowser,
        SettingKey::ExtraArgs,
        SettingKey::YtdlpPath,
        SettingKey::FfmpegPath,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingKey::OutputDir => "Output folder",
            SettingKey::Mode => "Default mode",
            SettingKey::VideoQuality => "Default video quality",
            SettingKey::VideoCodec => "Default video codec",
            SettingKey::AudioQuality => "Default audio quality",
            SettingKey::Container => "Default container",
            SettingKey::MaxConcurrent => "Max concurrent downloads",
            SettingKey::EmbedThumbnail => "Embed thumbnail",
            SettingKey::EmbedMetadata => "Embed metadata",
            SettingKey::WriteSubs => "Write subtitles",
            SettingKey::Playlist => "Playlist handling",
            SettingKey::CookiesBrowser => "Cookies from browser",
            SettingKey::ExtraArgs => "Extra yt-dlp args",
            SettingKey::YtdlpPath => "yt-dlp path",
            SettingKey::FfmpegPath => "ffmpeg path",
        }
    }

    pub fn kind(self) -> SettingKind {
        match self {
            SettingKey::OutputDir
            | SettingKey::CookiesBrowser
            | SettingKey::ExtraArgs
            | SettingKey::YtdlpPath
            | SettingKey::FfmpegPath => SettingKind::Text,
            SettingKey::Mode
            | SettingKey::VideoQuality
            | SettingKey::VideoCodec
            | SettingKey::AudioQuality
            | SettingKey::Container
            | SettingKey::Playlist => SettingKind::Choice,
            SettingKey::EmbedThumbnail | SettingKey::EmbedMetadata | SettingKey::WriteSubs => SettingKind::Bool,
            SettingKey::MaxConcurrent => SettingKind::Int,
        }
    }

    /// One-line explanation shown under the list, including the yt-dlp flag involved.
    pub fn hint(self) -> &'static str {
        match self {
            SettingKey::OutputDir => "Where finished files are written. Passed as -P. A leading ~ is expanded.",
            SettingKey::Mode => "What the Download tab starts with: video + audio, or audio only.",
            SettingKey::VideoQuality => "Starting value of the video quality picker. Becomes [height<=N] in -f.",
            SettingKey::VideoCodec => "Starting value of the video codec picker. Becomes a [vcodec...] filter in -f.",
            SettingKey::AudioQuality => {
                "Starting value of the audio picker. Audio-only mode: --audio-format / --audio-quality."
            }
            SettingKey::Container => "Starting value of the container picker. Passed as --merge-output-format.",
            SettingKey::MaxConcurrent => "How many downloads run at once (1 to 8). The rest wait in the queue.",
            SettingKey::EmbedThumbnail => "Adds --embed-thumbnail (needs ffmpeg).",
            SettingKey::EmbedMetadata => "Adds --embed-metadata (title, uploader, date, and so on).",
            SettingKey::WriteSubs => "Adds --write-subs --sub-langs en.*,en (subtitle files next to the video).",
            SettingKey::Playlist => "--no-playlist (only the video in the link) or --yes-playlist (everything).",
            SettingKey::CookiesBrowser => {
                "Adds --cookies-from-browser NAME, e.g. firefox or chrome:Profile 1. Empty = off."
            }
            SettingKey::ExtraArgs => "Appended after all other options, so they win. Quotes are understood.",
            SettingKey::YtdlpPath => "Name on PATH or full path to yt-dlp.",
            SettingKey::FfmpegPath => "Name on PATH, or a path to ffmpeg or its folder (passed as --ffmpeg-location).",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    Picker(FormRow),
    Preview,
    JobList,
    Details,
    Settings,
    Help,
}

/// What a clickable region does. `Wheel` regions only react to the scroll wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Tab(Tab),
    FocusUrl,
    Prev(FormRow),
    Next(FormRow),
    Value(FormRow),
    Start,
    SelectJob(u64),
    Cancel(u64),
    Retry(u64),
    Remove(u64),
    ClearFinished,
    SettingSelect(SettingKey),
    SettingPrev(SettingKey),
    SettingNext(SettingKey),
    SettingValue(SettingKey),
    Wheel(Wheel),
}

#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}

pub fn rect_contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y.saturating_add(r.height)
}

// ---------------------------------------------------------------------------------------
// Text input
// ---------------------------------------------------------------------------------------

/// A single-line text buffer. `cursor` counts characters, not bytes.
#[derive(Debug, Clone, Default)]
pub struct TextEdit {
    pub text: String,
    pub cursor: usize,
}

impl TextEdit {
    pub fn new(s: &str) -> Self {
        Self { text: s.to_string(), cursor: s.chars().count() }
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(self.text.len())
    }

    pub fn insert_str(&mut self, s: &str) {
        let b = self.byte_at(self.cursor);
        self.text.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    pub fn insert(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut buf));
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let b = self.byte_at(self.cursor - 1);
            self.text.remove(b);
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let b = self.byte_at(self.cursor);
            self.text.remove(b);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char('u') if ctrl => self.clear(),
            KeyCode::Char('a') if ctrl => self.home(),
            KeyCode::Char('e') if ctrl => self.end(),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => self.insert(c),
            _ => return false,
        }
        true
    }
}

pub struct Editing {
    pub key: SettingKey,
    pub edit: TextEdit,
}

// ---------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------

pub struct Job {
    pub id: u64,
    pub run: u32,
    pub url: String,
    pub opts: DlOptions,
    pub state: DownloadState,
    pub title: Option<String>,
    pub downloaded: f64,
    pub total: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub phase: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
    pub log: VecDeque<String>,
    pub command: String,
    pub cancel: Arc<AtomicBool>,
    /// True from the moment a worker thread starts until its `Done` message arrives.
    pub worker_alive: bool,
}

impl Job {
    fn new(id: u64, url: String, opts: DlOptions) -> Self {
        Self {
            id,
            run: 0,
            url,
            opts,
            state: DownloadState::Queued,
            title: None,
            downloaded: 0.0,
            total: None,
            speed: None,
            eta: None,
            phase: String::new(),
            output: None,
            error: None,
            exit_code: None,
            log: VecDeque::new(),
            command: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            worker_alive: false,
        }
    }

    pub fn percent(&self) -> Option<f64> {
        match (self.state, self.total) {
            (DownloadState::Finished, _) => Some(100.0),
            (_, Some(t)) if t > 0.0 => Some((self.downloaded / t * 100.0).clamp(0.0, 100.0)),
            _ => None,
        }
    }

    fn reset_progress(&mut self) {
        self.downloaded = 0.0;
        self.total = None;
        self.speed = None;
        self.eta = None;
        self.phase.clear();
        self.error = None;
        self.exit_code = None;
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LIMIT {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, DownloadState::Queued | DownloadState::Running)
    }
}

// ---------------------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

pub struct Status {
    pub text: String,
    pub level: Level,
    pub at: Instant,
}

pub struct App {
    pub should_quit: bool,
    pub tab: Tab,
    pub config: Config,
    persist: bool,

    // Hit testing, rebuilt by the UI on every frame.
    pub hits: Vec<Hit>,
    pub hover: Option<Action>,
    pub pressed: Option<Action>,
    drag_url: bool,

    // Download tab.
    pub form: Selection,
    pub form_focus: Option<FormRow>,
    pub url: TextEdit,
    pub url_view_start: usize,
    pub url_text_x: u16,
    pub preview_scroll: usize,

    // Downloads tab.
    pub jobs: Vec<Job>,
    pub sel_job: usize,
    pub list_scroll: usize,
    pub follow_sel: bool,
    pub details_scroll: usize,
    pub details_pin: bool,
    pub details_max: usize,
    next_id: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    // Settings tab.
    pub settings_sel: usize,
    pub settings_scroll: usize,
    pub editing: Option<Editing>,

    // Help tab.
    pub help_scroll: usize,

    // Misc.
    pub status: Option<Status>,
    pub tick: u64,
    pub resolved_ytdlp: Option<PathBuf>,
    pub resolved_ffmpeg: Option<PathBuf>,
    quit_armed_until: Option<Instant>,
}

impl App {
    pub fn new() -> Self {
        let (config, warning) = Config::load();
        let mut app = Self::with_config(config, true);
        if let Some(w) = warning {
            app.set_status(w, Level::Warn);
        }
        app.refresh_tools();
        app
    }

    pub fn with_config(config: Config, persist: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let form = config.selection();
        let mut app = Self {
            should_quit: false,
            tab: Tab::Download,
            config,
            persist,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            drag_url: false,
            form,
            form_focus: Some(FormRow::Url),
            url: TextEdit::default(),
            url_view_start: 0,
            url_text_x: 0,
            preview_scroll: 0,
            jobs: Vec::new(),
            sel_job: 0,
            list_scroll: 0,
            follow_sel: true,
            details_scroll: 0,
            details_pin: false,
            details_max: 0,
            next_id: 1,
            tx,
            rx,
            settings_sel: 0,
            settings_scroll: 0,
            editing: None,
            help_scroll: 0,
            status: None,
            tick: 0,
            resolved_ytdlp: None,
            resolved_ffmpeg: None,
            quit_armed_until: None,
        };
        app.refresh_tools();
        app
    }

    // ----- status line -----------------------------------------------------------------

    pub fn set_status(&mut self, text: impl Into<String>, level: Level) {
        self.status = Some(Status { text: text.into(), level, at: Instant::now() });
    }

    fn info(&mut self, t: impl Into<String>) {
        self.set_status(t, Level::Info);
    }
    fn ok(&mut self, t: impl Into<String>) {
        self.set_status(t, Level::Ok);
    }
    fn warn(&mut self, t: impl Into<String>) {
        self.set_status(t, Level::Warn);
    }
    fn error(&mut self, t: impl Into<String>) {
        self.set_status(t, Level::Error);
    }

    pub fn refresh_tools(&mut self) {
        self.resolved_ytdlp = resolve_binary(&self.config.ytdlp_path, DEFAULT_YTDLP);
        self.resolved_ffmpeg = resolve_binary(&self.config.ffmpeg_path, DEFAULT_FFMPEG);
        if self.resolved_ytdlp.is_none() {
            self.warn("yt-dlp was not found. Set its path in Settings (F3) or install it.");
        } else if self.resolved_ffmpeg.is_none() {
            self.warn("ffmpeg was not found. Merging and audio conversion need it (see Help, F4).");
        }
    }

    pub fn tab_title(&self, tab: Tab) -> String {
        if tab == Tab::Downloads {
            let active = self.active_count();
            if active > 0 {
                return format!("Downloads ({active})");
            }
        }
        tab.title().to_string()
    }

    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.is_active()).count()
    }

    fn running_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.state == DownloadState::Running).count()
    }

    // ----- hit map ---------------------------------------------------------------------

    pub fn register(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(Hit { rect, action });
        }
    }

    /// The smallest registered region under the pointer. With `wheel` set, only scroll
    /// regions are considered; otherwise only click regions are. Among equal sizes the
    /// one registered last (drawn on top) wins.
    pub fn resolve(&self, col: u16, row: u16, wheel: bool) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for h in &self.hits {
            if matches!(h.action, Action::Wheel(_)) != wheel || !rect_contains(h.rect, col, row) {
                continue;
            }
            let area = |r: Rect| r.width as u32 * r.height as u32;
            match best {
                Some(b) if area(b.rect) < area(h.rect) => {}
                _ => best = Some(*h),
            }
        }
        best
    }

    // ----- events from the main loop ---------------------------------------------------

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        while let Ok(msg) = self.rx.try_recv() {
            self.apply_msg(msg);
        }
        self.schedule();
        if self.status.as_ref().is_some_and(|s| s.at.elapsed() > Duration::from_secs(6)) {
            self.status = None;
        }
    }

    pub fn on_mouse(&mut self, ev: MouseEvent) {
        let (col, row) = (ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Moved => {
                self.hover = self.resolve(col, row, false).map(|h| h.action);
            }
            MouseEventKind::Down(MouseButton::Left) => self.left_down(col, row),
            MouseEventKind::Down(MouseButton::Right) => {
                self.hover = self.resolve(col, row, false).map(|h| h.action);
                if self.escape() {
                    self.info("Right click: focus cleared / edit cancelled (same as Esc).");
                }
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                self.info(
                    "Middle click does nothing. Left click acts, right click clears focus, wheel scrolls or cycles.",
                );
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.pressed = None;
                self.drag_url = false;
            }
            MouseEventKind::Up(MouseButton::Right) | MouseEventKind::Up(MouseButton::Middle) => {}
            MouseEventKind::Drag(MouseButton::Left) => {
                self.hover = self.resolve(col, row, false).map(|h| h.action);
                if self.drag_url {
                    self.place_url_caret(col);
                }
            }
            MouseEventKind::Drag(MouseButton::Right) | MouseEventKind::Drag(MouseButton::Middle) => {}
            MouseEventKind::ScrollUp => self.wheel(col, row, -1),
            MouseEventKind::ScrollDown => self.wheel(col, row, 1),
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {}
        }
    }

    pub fn on_paste(&mut self, text: String) {
        let flat = text.replace(['\r', '\n'], " ");
        if let Some(ed) = self.editing.as_mut() {
            ed.edit.insert_str(flat.trim());
            return;
        }
        if self.tab == Tab::Download {
            if self.form_focus != Some(FormRow::Url) {
                self.form_focus = Some(FormRow::Url);
            }
            self.url.insert_str(flat.trim());
        }
    }

    // ----- mouse helpers ---------------------------------------------------------------

    fn left_down(&mut self, col: u16, row: u16) {
        let hit = self.resolve(col, row, false);
        self.hover = hit.map(|h| h.action);

        // An open inline editor: clicking the same field cancels, clicking anywhere else commits.
        if let Some(ed) = &self.editing {
            let key = ed.key;
            if matches!(hit, Some(Hit { action: Action::SettingValue(k), .. }) if k == key) {
                self.cancel_edit();
                self.info("Edit cancelled.");
                return;
            }
            self.commit_edit();
        }

        let Some(hit) = hit else { return }; // decorative border or empty space
        self.pressed = Some(hit.action);
        self.activate(hit, col);
    }

    fn wheel(&mut self, col: u16, row: u16, dir: isize) {
        if self.editing.is_some() {
            return;
        }
        let Some(Hit { action: Action::Wheel(target), .. }) = self.resolve(col, row, true) else {
            return;
        };
        match target {
            Wheel::Picker(r) => {
                self.form_focus = Some(r);
                self.form_step(r, dir as i32);
            }
            Wheel::Preview => self.preview_scroll = shift(self.preview_scroll, dir * WHEEL_LINES),
            Wheel::JobList => {
                self.follow_sel = false;
                self.list_scroll = shift(self.list_scroll, dir);
            }
            Wheel::Details => self.scroll_details(dir * WHEEL_LINES),
            Wheel::Settings => self.move_setting(dir),
            Wheel::Help => self.help_scroll = shift(self.help_scroll, dir * WHEEL_LINES),
        }
    }

    fn place_url_caret(&mut self, col: u16) {
        let rel = col.saturating_sub(self.url_text_x) as usize;
        self.url.cursor = char_index_at(&self.url.text, self.url_view_start, rel);
    }

    /// Carry out the action behind a click. Every branch has a keyboard twin in `on_key`.
    fn activate(&mut self, hit: Hit, col: u16) {
        match hit.action {
            Action::Tab(t) => self.switch_tab(t),
            Action::FocusUrl => {
                self.form_focus = Some(FormRow::Url);
                self.place_url_caret(col);
                self.drag_url = true;
            }
            Action::Prev(r) => {
                self.form_focus = Some(r);
                self.form_step(r, -1);
            }
            Action::Next(r) | Action::Value(r) => {
                self.form_focus = Some(r);
                self.form_step(r, 1);
            }
            Action::Start => {
                self.form_focus = Some(FormRow::Start);
                self.start_download();
            }
            Action::SelectJob(id) => self.select_job_id(id),
            Action::Cancel(id) => {
                self.select_job_id(id);
                self.cancel_job_id(id);
            }
            Action::Retry(id) => {
                self.select_job_id(id);
                self.retry_job_id(id);
            }
            Action::Remove(id) => self.remove_job_id(id),
            Action::ClearFinished => self.clear_finished(),
            Action::SettingSelect(k) => self.settings_sel = k.index(),
            Action::SettingPrev(k) => {
                self.settings_sel = k.index();
                self.step_setting(k, -1);
            }
            Action::SettingNext(k) => {
                self.settings_sel = k.index();
                self.step_setting(k, 1);
            }
            Action::SettingValue(k) => {
                self.settings_sel = k.index();
                self.activate_setting(k);
            }
            Action::Wheel(_) => {}
        }
    }

    /// Esc, and right click. Returns true if anything changed.
    pub fn escape(&mut self) -> bool {
        if self.editing.is_some() {
            self.cancel_edit();
            return true;
        }
        if self.tab == Tab::Download && self.form_focus.is_some() {
            self.form_focus = None;
            return true;
        }
        false
    }

    pub fn switch_tab(&mut self, tab: Tab) {
        if self.editing.is_some() {
            self.commit_edit();
        }
        self.tab = tab;
    }

    // ----- keyboard --------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // Tab switching works everywhere, even while typing.
        let target = match key.code {
            KeyCode::F(n @ 1..=4) => Some(Tab::ALL[(n - 1) as usize]),
            KeyCode::Left if alt => Some(self.tab.prev()),
            KeyCode::Right if alt => Some(self.tab.next()),
            _ => None,
        };
        if let Some(t) = target {
            self.switch_tab(t);
            return;
        }

        if self.editing.is_some() {
            self.key_editing(key);
            return;
        }

        if ctrl && key.code == KeyCode::Char('q') {
            self.request_quit();
            return;
        }

        let typing = self.tab == Tab::Download && self.form_focus == Some(FormRow::Url);
        if !typing && !ctrl && !alt {
            match key.code {
                KeyCode::Char('q') => return self.request_quit(),
                KeyCode::Char(c @ '1'..='4') => return self.switch_tab(Tab::ALL[c as usize - '1' as usize]),
                KeyCode::Char('?') => return self.switch_tab(Tab::Help),
                _ => {}
            }
        }

        match self.tab {
            Tab::Download => self.key_download(key),
            Tab::Downloads => self.key_downloads(key),
            Tab::Settings => self.key_settings(key),
            Tab::Help => self.key_help(key),
        }
    }

    fn request_quit(&mut self) {
        let active = self.active_count();
        let armed = self.quit_armed_until.is_some_and(|t| Instant::now() < t);
        if active > 0 && !armed {
            self.quit_armed_until = Some(Instant::now() + Duration::from_secs(3));
            self.warn(format!("{active} download(s) still active. Press q again to quit and cancel them."));
        } else {
            self.should_quit = true;
        }
    }

    fn key_download(&mut self, key: KeyEvent) {
        let focus = self.form_focus;
        let url_focused = focus == Some(FormRow::Url);

        if url_focused
            && !matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Tab
                    | KeyCode::BackTab
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Enter
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        {
            self.url.handle_key(&key);
            return;
        }

        match key.code {
            KeyCode::Esc => self.form_focus = None,
            KeyCode::Tab | KeyCode::Down => self.focus_step(1),
            KeyCode::BackTab | KeyCode::Up => self.focus_step(-1),
            KeyCode::PageUp => self.preview_scroll = shift(self.preview_scroll, -WHEEL_LINES),
            KeyCode::PageDown => self.preview_scroll = shift(self.preview_scroll, WHEEL_LINES),
            KeyCode::Enter => match focus {
                None => self.form_focus = Some(FormRow::Url),
                Some(FormRow::Url) | Some(FormRow::Start) => self.start_download(),
                Some(r) => self.form_step(r, 1),
            },
            KeyCode::Left => {
                if let Some(r) = focus.filter(|r| r.is_picker()) {
                    self.form_step(r, -1);
                }
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                if let Some(r) = focus.filter(|r| r.is_picker()) {
                    self.form_step(r, 1);
                }
            }
            KeyCode::Char('i') if focus.is_none() => self.form_focus = Some(FormRow::Url),
            _ => {}
        }
    }

    fn focus_step(&mut self, delta: i32) {
        let n = FormRow::ALL.len() as i32;
        let next = match self.form_focus {
            None if delta > 0 => 0,
            None => n - 1,
            Some(r) => (r.index() as i32 + delta).rem_euclid(n),
        };
        self.form_focus = Some(FormRow::ALL[next as usize]);
    }

    fn key_downloads(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_job(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_job(1),
            KeyCode::Home => self.set_sel(0),
            KeyCode::End => self.set_sel(self.jobs.len().saturating_sub(1)),
            KeyCode::PageUp => self.scroll_details(-5),
            KeyCode::PageDown => self.scroll_details(5),
            KeyCode::Char('C') if !ctrl => self.clear_finished(),
            KeyCode::Char('c') | KeyCode::Char('x') if !ctrl => {
                if let Some(id) = self.selected_id() {
                    self.cancel_job_id(id);
                }
            }
            KeyCode::Char('r') if !ctrl => {
                if let Some(id) = self.selected_id() {
                    self.retry_job_id(id);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if !ctrl => {
                if let Some(id) = self.selected_id() {
                    self.remove_job_id(id);
                }
            }
            _ => {}
        }
    }

    fn key_settings(&mut self, key: KeyEvent) {
        let k = SettingKey::ALL[self.settings_sel.min(SettingKey::ALL.len() - 1)];
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_setting(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_setting(1),
            KeyCode::PageUp => self.move_setting(-5),
            KeyCode::PageDown => self.move_setting(5),
            KeyCode::Home => self.settings_sel = 0,
            KeyCode::End => self.settings_sel = SettingKey::ALL.len() - 1,
            KeyCode::Left | KeyCode::Char('h') => self.step_setting(k, -1),
            KeyCode::Right | KeyCode::Char('l') => self.step_setting(k, 1),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_setting(k),
            _ => {}
        }
    }

    fn key_help(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.help_scroll = shift(self.help_scroll, -1),
            KeyCode::Down | KeyCode::Char('j') => self.help_scroll = shift(self.help_scroll, 1),
            KeyCode::PageUp => self.help_scroll = shift(self.help_scroll, -10),
            KeyCode::PageDown => self.help_scroll = shift(self.help_scroll, 10),
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = usize::MAX / 2,
            _ => {}
        }
    }

    fn key_editing(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Esc => {
                self.cancel_edit();
            }
            _ => {
                if let Some(ed) = self.editing.as_mut() {
                    ed.edit.handle_key(&key);
                }
            }
        }
    }

    // ----- Download tab ----------------------------------------------------------------

    pub fn form_step(&mut self, row: FormRow, delta: i32) {
        let f = &mut self.form;
        match row {
            FormRow::Mode => f.mode = f.mode.step(delta),
            FormRow::VideoQuality => f.video_quality = f.video_quality.step(delta),
            FormRow::VideoCodec => f.video_codec = f.video_codec.step(delta),
            FormRow::AudioQuality => f.audio_quality = f.audio_quality.step(delta),
            FormRow::Container => f.container = f.container.step(delta),
            FormRow::Url | FormRow::Start => {}
        }
    }

    pub fn form_options(&self) -> DlOptions {
        DlOptions::new(self.form, &self.config)
    }

    fn start_download(&mut self) {
        let text = self.url.text.trim().to_string();
        if text.is_empty() {
            self.form_focus = Some(FormRow::Url);
            self.error("Type or paste a URL first.");
            return;
        }
        if self.resolved_ytdlp.is_none() {
            self.refresh_tools();
            if self.resolved_ytdlp.is_none() {
                self.error("yt-dlp was not found. Set its path in Settings (F3).");
                return;
            }
        }
        let urls = split_urls(&text);
        let n = urls.len();
        for url in urls {
            self.enqueue(url);
        }
        self.url.clear();
        self.url_view_start = 0;
        self.schedule();
        self.ok(format!(
            "Queued {n} download{}. Watch progress on the Downloads tab (F2).",
            if n == 1 { "" } else { "s" }
        ));
    }

    pub fn enqueue(&mut self, url: String) {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job::new(id, url, self.form_options()));
        self.set_sel(self.jobs.len() - 1);
    }

    // ----- jobs ------------------------------------------------------------------------

    fn index_of(&self, id: u64) -> Option<usize> {
        self.jobs.iter().position(|j| j.id == id)
    }

    pub fn selected_id(&self) -> Option<u64> {
        self.jobs.get(self.sel_job).map(|j| j.id)
    }

    fn set_sel(&mut self, idx: usize) {
        let idx = idx.min(self.jobs.len().saturating_sub(1));
        if idx != self.sel_job {
            self.details_scroll = 0;
            self.details_pin = false;
        }
        self.sel_job = idx;
        self.follow_sel = true;
    }

    fn select_job_id(&mut self, id: u64) {
        if let Some(i) = self.index_of(id) {
            self.set_sel(i);
        }
    }

    fn move_job(&mut self, delta: isize) {
        if !self.jobs.is_empty() {
            self.set_sel(shift(self.sel_job, delta));
        }
    }

    pub fn scroll_details(&mut self, delta: isize) {
        let current = if self.details_pin { self.details_max } else { self.details_scroll };
        let next = shift(current, delta).min(self.details_max);
        self.details_scroll = next;
        self.details_pin = self.details_max > 0 && next >= self.details_max;
    }

    /// Start queued jobs until the concurrency limit is reached.
    fn schedule(&mut self) {
        let limit = self.config.max_concurrent.clamp(1, MAX_CONCURRENT_LIMIT);
        while self.running_count() < limit {
            let Some(idx) = self.jobs.iter().position(|j| j.state == DownloadState::Queued) else { break };
            self.start_job(idx);
        }
    }

    fn start_job(&mut self, idx: usize) {
        let program = self
            .resolved_ytdlp
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.config.ytdlp_path.clone());
        let tx = self.tx.clone();
        let job = &mut self.jobs[idx];
        job.run += 1;
        job.state = DownloadState::Running;
        job.reset_progress();
        job.phase = "Starting".into();
        job.cancel = Arc::new(AtomicBool::new(false));
        job.worker_alive = true;
        let args = ytdlp::build_args(&job.opts, &job.url);
        job.command = ytdlp::command_preview(&program, &job.opts, &job.url);
        let tag = Tag { id: job.id, run: job.run };
        download::spawn_download(tag, program, args, tx, job.cancel.clone());
    }

    fn apply_msg(&mut self, msg: Msg) {
        let tag = msg.tag();
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == tag.id) else { return };
        if job.run != tag.run {
            return; // a late message from an earlier run of this job
        }
        match msg {
            Msg::Progress(_, p) => {
                if job.state == DownloadState::Running {
                    if let Some(d) = p.downloaded {
                        job.downloaded = d;
                    }
                    job.total = p.total.or(job.total);
                    job.speed = p.speed;
                    job.eta = p.eta;
                    if let Some(t) = p.title {
                        job.title = Some(t);
                    }
                    if job.phase != "Downloading" {
                        job.phase = "Downloading".into();
                    }
                }
            }
            Msg::Final(_, path) => job.output = Some(path),
            Msg::Phase(_, phase) => {
                if job.state == DownloadState::Running {
                    job.phase = phase.to_string();
                }
            }
            Msg::Log { line, is_stderr, .. } => {
                if is_stderr && line.starts_with("ERROR") {
                    job.error = Some(line.clone());
                }
                job.push_log(line);
            }
            Msg::Done(_, exit) => {
                job.worker_alive = false;
                if job.state != DownloadState::Running {
                    return; // already cancelled by the user
                }
                job.speed = None;
                job.eta = None;
                job.phase.clear();
                match exit {
                    Exit::Success => {
                        job.state = DownloadState::Finished;
                        job.exit_code = Some(0);
                    }
                    Exit::Failed(code) => {
                        job.state = DownloadState::Failed;
                        job.exit_code = code;
                        if job.error.is_none() {
                            job.error = Some(match code {
                                Some(c) => format!("yt-dlp exited with code {c}"),
                                None => "yt-dlp was stopped by a signal".into(),
                            });
                        }
                    }
                    Exit::Cancelled => job.state = DownloadState::Cancelled,
                    Exit::SpawnFailed(m) => {
                        job.state = DownloadState::Failed;
                        job.error = Some(m);
                    }
                }
            }
        }
    }

    pub fn cancel_job_id(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        let job = &mut self.jobs[i];
        match job.state {
            DownloadState::Queued => {
                job.state = DownloadState::Cancelled;
                job.push_log("Cancelled before it started.".into());
            }
            DownloadState::Running => {
                job.cancel.store(true, Ordering::SeqCst);
                job.state = DownloadState::Cancelled;
                job.speed = None;
                job.eta = None;
                job.phase.clear();
                job.push_log("Cancelled by user.".into());
            }
            _ => {}
        }
    }

    pub fn retry_job_id(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        let job = &mut self.jobs[i];
        if job.is_active() {
            return;
        }
        job.state = DownloadState::Queued;
        job.reset_progress();
        job.push_log("--- retry ---".into());
        self.schedule();
    }

    pub fn remove_job_id(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        // Stop it first if it is running; the worker kills the process on its own thread.
        self.jobs[i].cancel.store(true, Ordering::SeqCst);
        self.jobs.remove(i);
        if self.sel_job > i || self.sel_job >= self.jobs.len() {
            self.sel_job = self.sel_job.saturating_sub(1).min(self.jobs.len().saturating_sub(1));
        }
        self.details_scroll = 0;
        self.details_pin = false;
        self.follow_sel = true;
    }

    pub fn clear_finished(&mut self) {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.state != DownloadState::Finished);
        let removed = before - self.jobs.len();
        self.sel_job = self.sel_job.min(self.jobs.len().saturating_sub(1));
        self.details_scroll = 0;
        self.details_pin = false;
        self.info(if removed == 0 {
            "No finished downloads to clear.".to_string()
        } else {
            format!("Cleared {removed} finished download(s).")
        });
    }

    /// Cancel everything and give the workers a moment to kill their child processes.
    pub fn shutdown(&mut self) {
        for j in &self.jobs {
            if j.worker_alive {
                j.cancel.store(true, Ordering::SeqCst);
            }
        }
        let deadline = Instant::now() + Duration::from_millis(2000);
        while Instant::now() < deadline && self.jobs.iter().any(|j| j.worker_alive) {
            if let Ok(msg) = self.rx.recv_timeout(Duration::from_millis(50)) {
                self.apply_msg(msg);
            }
        }
    }

    // ----- Settings --------------------------------------------------------------------

    pub fn setting_text(&self, k: SettingKey) -> String {
        let c = &self.config;
        match k {
            SettingKey::OutputDir => c.output_dir.clone(),
            SettingKey::Mode => c.mode.label().into(),
            SettingKey::VideoQuality => c.video_quality.label().into(),
            SettingKey::VideoCodec => c.video_codec.label().into(),
            SettingKey::AudioQuality => c.audio_quality.label().into(),
            SettingKey::Container => c.container.label().into(),
            SettingKey::MaxConcurrent => c.max_concurrent.to_string(),
            SettingKey::EmbedThumbnail => on_off(c.embed_thumbnail),
            SettingKey::EmbedMetadata => on_off(c.embed_metadata),
            SettingKey::WriteSubs => on_off(c.write_subs),
            SettingKey::Playlist => c.playlist.label().into(),
            SettingKey::CookiesBrowser => c.cookies_from_browser.clone(),
            SettingKey::ExtraArgs => c.extra_args.clone(),
            SettingKey::YtdlpPath => c.ytdlp_path.clone(),
            SettingKey::FfmpegPath => c.ffmpeg_path.clone(),
        }
    }

    pub fn setting_is_on(&self, k: SettingKey) -> bool {
        match k {
            SettingKey::EmbedThumbnail => self.config.embed_thumbnail,
            SettingKey::EmbedMetadata => self.config.embed_metadata,
            SettingKey::WriteSubs => self.config.write_subs,
            _ => false,
        }
    }

    fn move_setting(&mut self, delta: isize) {
        self.settings_sel = shift(self.settings_sel, delta).min(SettingKey::ALL.len() - 1);
    }

    /// The arrows (and Left/Right keys). Text fields ignore them.
    fn step_setting(&mut self, k: SettingKey, delta: i32) {
        let c = &mut self.config;
        match k {
            SettingKey::Mode => {
                c.mode = c.mode.step(delta);
                self.form.mode = c.mode;
            }
            SettingKey::VideoQuality => {
                c.video_quality = c.video_quality.step(delta);
                self.form.video_quality = c.video_quality;
            }
            SettingKey::VideoCodec => {
                c.video_codec = c.video_codec.step(delta);
                self.form.video_codec = c.video_codec;
            }
            SettingKey::AudioQuality => {
                c.audio_quality = c.audio_quality.step(delta);
                self.form.audio_quality = c.audio_quality;
            }
            SettingKey::Container => {
                c.container = c.container.step(delta);
                self.form.container = c.container;
            }
            SettingKey::Playlist => c.playlist = c.playlist.step(delta),
            SettingKey::MaxConcurrent => {
                let n = MAX_CONCURRENT_LIMIT as i32;
                c.max_concurrent = (c.max_concurrent as i32 - 1 + delta).rem_euclid(n) as usize + 1;
            }
            SettingKey::EmbedThumbnail => c.embed_thumbnail = !c.embed_thumbnail,
            SettingKey::EmbedMetadata => c.embed_metadata = !c.embed_metadata,
            SettingKey::WriteSubs => c.write_subs = !c.write_subs,
            SettingKey::OutputDir
            | SettingKey::CookiesBrowser
            | SettingKey::ExtraArgs
            | SettingKey::YtdlpPath
            | SettingKey::FfmpegPath => return,
        }
        self.save_config();
    }

    /// Clicking a value, or Enter / Space: advance a picker, flip a switch, open an editor.
    fn activate_setting(&mut self, k: SettingKey) {
        if k.kind() == SettingKind::Text {
            self.editing = Some(Editing { key: k, edit: TextEdit::new(&self.setting_text(k)) });
        } else {
            self.step_setting(k, 1);
        }
    }

    pub fn commit_edit(&mut self) {
        let Some(Editing { key, edit }) = self.editing.take() else { return };
        let value = edit.text.trim().to_string();
        let c = &mut self.config;
        match key {
            SettingKey::OutputDir if value.is_empty() => {
                self.warn("The output folder cannot be empty. Kept the old value.");
                return;
            }
            SettingKey::OutputDir => c.output_dir = value,
            SettingKey::CookiesBrowser => c.cookies_from_browser = value,
            SettingKey::ExtraArgs => c.extra_args = value,
            SettingKey::YtdlpPath => c.ytdlp_path = if value.is_empty() { DEFAULT_YTDLP.into() } else { value },
            SettingKey::FfmpegPath => c.ffmpeg_path = if value.is_empty() { DEFAULT_FFMPEG.into() } else { value },
            _ => return,
        }
        self.save_config();
        if matches!(key, SettingKey::YtdlpPath | SettingKey::FfmpegPath) {
            self.refresh_tools();
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    fn save_config(&mut self) {
        if !self.persist {
            return;
        }
        if let Err(e) = self.config.save() {
            self.error(format!("Could not save settings: {e:#}"));
        }
    }
}

fn on_off(b: bool) -> String {
    if b { "on".into() } else { "off".into() }
}

/// Move an index by a signed amount without underflow.
pub fn shift(v: usize, delta: isize) -> usize {
    if delta < 0 { v.saturating_sub(delta.unsigned_abs()) } else { v.saturating_add(delta as usize) }
}

/// Several pasted links become several downloads. Anything else (a search such as
/// "ytsearch:cats and dogs") is passed to yt-dlp as a single argument.
pub fn split_urls(text: &str) -> Vec<String> {
    let text = text.trim();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() > 1 && tokens.iter().all(|t| t.contains("://")) {
        tokens.iter().map(|t| t.to_string()).collect()
    } else if text.is_empty() {
        Vec::new()
    } else {
        vec![text.to_string()]
    }
}
