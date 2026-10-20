//! All rendering. Every clickable region registers a `(Rect, Action)` pair on the app
//! while it is drawn, so the hit map always matches what is on screen.

use std::sync::OnceLock;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use crate::app::*;
use crate::config::Config;
use crate::text::{char_width, fit, human_bytes, human_eta, truncate, width, wrap_text};
use crate::types::*;
use crate::ytdlp;

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 22;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const SELECTED_BG: Color = Color::Indexed(236);

fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}

/// Style for a small button-like element: plain, hovered, or held down.
fn button_style(app: &App, action: Action, base: Style, accent: Color) -> Style {
    if app.pressed == Some(action) {
        Style::new().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)
    } else if app.hover == Some(action) {
        Style::new().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// Icons for the job controls. Set YTGET_ASCII=1 if your terminal draws the emoji badly.
fn ascii_icons() -> bool {
    static ASCII: OnceLock<bool> = OnceLock::new();
    *ASCII.get_or_init(|| std::env::var_os("YTGET_ASCII").is_some_and(|v| !v.is_empty() && v != "0"))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small.\nNeed at least {MIN_WIDTH}x{MIN_HEIGHT}, have {}x{}.",
            area.width, area.height
        );
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center).style(fg(Color::Yellow)), area);
        return;
    }
    let [tabs_a, body_a, foot_a] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)]).areas(area);
    draw_tabs(f, app, tabs_a);
    match app.tab {
        Tab::Download => draw_download(f, app, body_a),
        Tab::Downloads => draw_downloads(f, app, body_a),
        Tab::Settings => draw_settings(f, app, body_a),
        Tab::Help => draw_help(f, app, body_a),
    }
    draw_footer(f, app, foot_a);
}

// ---------------------------------------------------------------------------------------
// Tab bar and footer
// ---------------------------------------------------------------------------------------

fn draw_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    for tab in Tab::ALL {
        let title = app.tab_title(tab);
        let w = (width(&title) + 6) as u16;
        let rect = Rect::new(x, area.y, w, 3).intersection(area);
        if rect.width < 4 {
            break;
        }
        let action = Action::Tab(tab);
        let active = app.tab == tab;
        let (text_style, border) = if app.pressed == Some(action) {
            (Style::new().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD), Color::White)
        } else if active {
            (Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD), ACCENT)
        } else if app.hover == Some(action) {
            (Style::new().fg(Color::White).bg(Color::Indexed(238)).add_modifier(Modifier::BOLD), Color::White)
        } else {
            (fg(Color::Gray), DIM)
        };
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(fg(border));
        f.render_widget(Paragraph::new(title).alignment(Alignment::Center).style(text_style).block(block), rect);
        app.register(rect, action);
        x = rect.right() + 1;
    }
    if x + 14 < area.right() {
        let r = Rect::new(x, area.y + 1, area.right() - x, 1);
        f.render_widget(Paragraph::new("ytget-rs ").alignment(Alignment::Right).style(fg(DIM)), r);
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let line = match &app.status {
        Some(s) => {
            let color = match s.level {
                Level::Info => ACCENT,
                Level::Ok => Color::Green,
                Level::Warn => Color::Yellow,
                Level::Error => Color::Red,
            };
            Line::from(Span::styled(format!(" {}", truncate(&s.text, area.width as usize - 2)), bold(color)))
        }
        None => {
            let hints = if app.editing.is_some() {
                "Type to edit | Enter save | Esc cancel | click elsewhere saves, click the field again cancels"
            } else {
                match app.tab {
                    Tab::Download => {
                        "Tab/Up/Down move | Left/Right change | Enter start | Esc unfocus | F1-F4 tabs | q quit"
                    }
                    Tab::Downloads => {
                        "Up/Down select | c cancel | r retry | d remove | C clear finished | PgUp/PgDn details"
                    }
                    Tab::Settings => "Up/Down select | Left/Right change | Enter edit or flip | F1-F4 tabs | q quit",
                    Tab::Help => "Up/Down/PgUp/PgDn/Home/End scroll | F1-F4 tabs | q quit",
                }
            };
            Line::from(Span::styled(format!(" {}", truncate(hints, area.width as usize - 2)), fg(DIM)))
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------------------
// Download tab
// ---------------------------------------------------------------------------------------

fn draw_download(f: &mut Frame, app: &mut App, area: Rect) {
    let [url_a, grid_a, preview_a] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(9), Constraint::Min(4)]).areas(area);
    draw_url(f, app, url_a);

    let rows = Layout::vertical([Constraint::Length(3); 3]).split(grid_a);
    let cols: Vec<_> = rows
        .iter()
        .map(|r| Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(*r))
        .collect();

    let audio_only = app.form.mode == Mode::AudioOnly;
    let f_ = app.form;
    let pickers = [
        (FormRow::Mode, "Mode", f_.mode.label(), false, cols[0][0]),
        (FormRow::VideoQuality, "Video quality", f_.video_quality.label(), audio_only, cols[0][1]),
        (FormRow::VideoCodec, "Video codec", f_.video_codec.label(), audio_only, cols[1][0]),
        (FormRow::AudioQuality, "Audio quality", f_.audio_quality.label(), false, cols[1][1]),
        (FormRow::Container, "Container", f_.container.label(), audio_only, cols[2][0]),
    ];
    for (row, label, value, disabled, rect) in pickers {
        draw_picker(f, app, rect, row, label, value, disabled);
    }
    draw_start_button(f, app, cols[2][1]);
    draw_preview(f, app, preview_a);
}

fn draw_url(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.form_focus == Some(FormRow::Url);
    let title_style = if focused { bold(ACCENT) } else { fg(Color::Gray) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(if focused { ACCENT } else { DIM }))
        .title(Span::styled(" URL  (paste or type, Enter starts the download) ", title_style));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.register(area, Action::FocusUrl);
    if inner.height == 0 || inner.width < 4 {
        return;
    }

    let text_x = inner.x + 1;
    let avail = inner.width.saturating_sub(2) as usize;
    let chars: Vec<char> = app.url.text.chars().collect();
    let cursor = app.url.cursor.min(chars.len());
    let span_width = |s: &[char]| s.iter().map(|c| char_width(*c)).sum::<usize>();

    // Scroll horizontally so the caret stays visible, keeping one cell free for it.
    let mut start = app.url_view_start.min(cursor);
    while start < cursor && span_width(&chars[start..cursor]) >= avail {
        start += 1;
    }
    app.url_view_start = start;
    app.url_text_x = text_x;

    let mut shown = String::new();
    let mut used = 0;
    for &c in &chars[start..] {
        let w = char_width(c);
        if used + w > avail {
            break;
        }
        shown.push(c);
        used += w;
    }

    let text_rect = Rect::new(text_x, inner.y, avail as u16, 1);
    if chars.is_empty() && !focused {
        f.render_widget(Paragraph::new("https://www.youtube.com/watch?v=...").style(fg(DIM)), text_rect);
    } else {
        f.render_widget(Paragraph::new(shown), text_rect);
    }
    if focused && app.tab == Tab::Download {
        let cx = text_x + span_width(&chars[start..cursor]) as u16;
        f.set_cursor_position((cx.min(inner.right().saturating_sub(1)), inner.y));
    }
}

fn draw_picker(f: &mut Frame, app: &mut App, rect: Rect, row: FormRow, label: &str, value: &str, disabled: bool) {
    let focused = app.form_focus == Some(row);
    let title_style = if focused { bold(ACCENT) } else { fg(Color::Gray) };
    let title = if disabled { format!(" {label} (not used) ") } else { format!(" {label} ") };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(if focused { ACCENT } else { DIM }))
        .title(Span::styled(title, title_style));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.height == 0 || rect.width < 16 {
        return;
    }

    // The arrow zones are 6 cells wide and as tall as the card, so they are easy to hit.
    let zone = 6u16;
    let left = Rect::new(rect.x, rect.y, zone, rect.height);
    let right = Rect::new(rect.right() - zone, rect.y, zone, rect.height);
    let mid = Rect::new(rect.x + zone, rect.y, rect.width - 2 * zone, rect.height);
    let y = inner.y;

    let arrow_base = if disabled { fg(DIM) } else { bold(ACCENT) };
    for (zone_rect, glyph, action) in [(left, " < ", Action::Prev(row)), (right, " > ", Action::Next(row))] {
        let style = button_style(app, action, arrow_base, ACCENT);
        let x = zone_rect.x + 1 + (zone_rect.width - 1).saturating_sub(3) / 2;
        f.render_widget(Paragraph::new(glyph).style(style), Rect::new(x, y, 3, 1));
        app.register(zone_rect, action);
    }

    let value_action = Action::Value(row);
    let base = if disabled { fg(DIM) } else { Style::new().add_modifier(Modifier::BOLD) };
    let style = if app.pressed == Some(value_action) {
        Style::new().add_modifier(Modifier::REVERSED)
    } else if app.hover == Some(value_action) {
        base.fg(ACCENT).add_modifier(Modifier::UNDERLINED)
    } else {
        base
    };
    let text = truncate(value, mid.width as usize);
    f.render_widget(Paragraph::new(text).alignment(Alignment::Center).style(style), Rect::new(mid.x, y, mid.width, 1));
    app.register(mid, value_action);
    app.register(rect, Action::Wheel(Wheel::Picker(row)));
}

fn draw_start_button(f: &mut Frame, app: &mut App, rect: Rect) {
    let focused = app.form_focus == Some(FormRow::Start);
    let style = button_style(app, Action::Start, bold(Color::Green), Color::Green);
    let block = Block::bordered()
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(fg(Color::Green));
    f.render_widget(Paragraph::new("> Start download").alignment(Alignment::Center).style(style).block(block), rect);
    app.register(rect, Action::Start);
}

fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(DIM))
        .title(Span::styled(" yt-dlp command (live preview, wheel scrolls) ", fg(Color::Gray)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.register(area, Action::Wheel(Wheel::Preview));
    if inner.height == 0 {
        return;
    }

    let warning = ytdlp::compat_warning(&app.form);
    let warn_h = if warning.is_some() { 2.min(inner.height.saturating_sub(1)) } else { 0 };
    let [text_a, warn_a] = Layout::vertical([Constraint::Min(1), Constraint::Length(warn_h)]).areas(inner);
    let text_a = text_a.inner(Margin { vertical: 0, horizontal: 1 });
    let warn_a = warn_a.inner(Margin { vertical: 0, horizontal: 1 });

    let url = split_urls(&app.url.text).into_iter().next().unwrap_or_else(|| "<URL>".into());
    let command = ytdlp::command_preview(&app.config.ytdlp_path, &app.form_options(), &url);
    let lines = wrap_text(&command, text_a.width as usize);
    let max = lines.len().saturating_sub(text_a.height as usize);
    app.preview_scroll = app.preview_scroll.min(max);
    let visible: Vec<Line> = lines
        .iter()
        .skip(app.preview_scroll)
        .take(text_a.height as usize)
        .map(|l| Line::from(Span::styled(l.clone(), fg(Color::Gray))))
        .collect();
    f.render_widget(Paragraph::new(visible), text_a);
    draw_scrollbar(f, area, max, app.preview_scroll);

    if let Some(w) = warning {
        f.render_widget(Paragraph::new(w).style(fg(Color::Yellow)).wrap(Wrap { trim: true }), warn_a);
    }
}

fn draw_scrollbar(f: &mut Frame, outer: Rect, max_scroll: usize, pos: usize) {
    if max_scroll == 0 || outer.height < 4 {
        return;
    }
    let mut state = ScrollbarState::new(max_scroll + 1).position(pos);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None);
    f.render_stateful_widget(bar, outer.inner(Margin { vertical: 1, horizontal: 0 }), &mut state);
}

// ---------------------------------------------------------------------------------------
// Downloads tab
// ---------------------------------------------------------------------------------------

struct RowView {
    id: u64,
    title: String,
    state: DownloadState,
    phase: String,
    percent: Option<f64>,
    speed: Option<f64>,
    eta: Option<u64>,
    error: Option<String>,
}

fn draw_downloads(f: &mut Frame, app: &mut App, area: Rect) {
    let [list_a, details_a] = Layout::vertical([Constraint::Percentage(55), Constraint::Min(7)]).areas(area);
    draw_job_list(f, app, list_a);
    draw_details(f, app, details_a);
}

fn draw_job_list(f: &mut Frame, app: &mut App, area: Rect) {
    let n = app.jobs.len();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(DIM))
        .title(Span::styled(format!(" Queue ({n}) "), fg(Color::Gray)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.register(area, Action::Wheel(Wheel::JobList));
    if inner.height < 3 {
        return;
    }

    // Header line with the "Clear finished" button.
    let label = " Clear finished ";
    let btn_w = width(label) as u16;
    let btn = Rect::new(inner.right().saturating_sub(btn_w + 1), inner.y, btn_w, 1);
    f.render_widget(Paragraph::new("  Title").style(fg(DIM)), Rect::new(inner.x, inner.y, inner.width, 1));
    let btn_style =
        button_style(app, Action::ClearFinished, fg(Color::Gray).add_modifier(Modifier::REVERSED), Color::Green);
    f.render_widget(Paragraph::new(label).style(btn_style), btn);
    app.register(btn, Action::ClearFinished);

    let list_a = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    if n == 0 {
        let msg = "Nothing queued yet.\nOpen the Download tab (F1), paste a URL and click Start download.";
        f.render_widget(
            Paragraph::new(msg).alignment(Alignment::Center).style(fg(DIM)),
            Rect::new(list_a.x, list_a.y + 1, list_a.width, list_a.height.saturating_sub(1)),
        );
        return;
    }

    let visible = (list_a.height / 2).max(1) as usize;
    if app.follow_sel {
        if app.sel_job < app.list_scroll {
            app.list_scroll = app.sel_job;
        } else if app.sel_job >= app.list_scroll + visible {
            app.list_scroll = app.sel_job + 1 - visible;
        }
        app.follow_sel = false;
    }
    let max_scroll = n.saturating_sub(visible);
    app.list_scroll = app.list_scroll.min(max_scroll);

    let rows: Vec<RowView> = app
        .jobs
        .iter()
        .skip(app.list_scroll)
        .take(visible)
        .map(|j| RowView {
            id: j.id,
            title: j.title.clone().unwrap_or_else(|| j.url.clone()),
            state: j.state,
            phase: j.phase.clone(),
            percent: j.percent(),
            speed: j.speed,
            eta: j.eta,
            error: j.error.clone(),
        })
        .collect();
    for (i, view) in rows.iter().enumerate() {
        let rect = Rect::new(list_a.x, list_a.y + (i as u16) * 2, list_a.width, 2);
        let selected = app.list_scroll + i == app.sel_job;
        draw_job_row(f, app, rect, view, selected);
    }
    draw_scrollbar(f, area, max_scroll, app.list_scroll);
}

fn state_color(s: DownloadState) -> Color {
    match s {
        DownloadState::Queued => Color::Gray,
        DownloadState::Running => ACCENT,
        DownloadState::Finished => Color::Green,
        DownloadState::Failed => Color::Red,
        DownloadState::Cancelled => Color::Yellow,
    }
}

fn draw_job_row(f: &mut Frame, app: &mut App, rect: Rect, v: &RowView, selected: bool) {
    const CTL_W: u16 = 17; // three 5-wide buttons with a 1-cell gap
    if rect.width < CTL_W + 24 {
        return;
    }
    if selected {
        f.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), rect);
    }
    app.register(rect, Action::SelectJob(v.id));

    let left_w = (rect.width - CTL_W - 1) as usize;
    let color = state_color(v.state);
    let spinner = ["|", "/", "-", "\\"][(app.tick / 2 % 4) as usize];
    let icon = match v.state {
        DownloadState::Queued => ".",
        DownloadState::Running => spinner,
        DownloadState::Finished => "+",
        DownloadState::Failed => "x",
        DownloadState::Cancelled => "-",
    };
    let state_text = if v.state == DownloadState::Running && !v.phase.is_empty() {
        v.phase.clone()
    } else {
        v.state.label().to_string()
    };
    let state_w = 20.min(left_w / 3);
    let title_w = left_w.saturating_sub(4 + state_w);
    let marker = if selected { ">" } else { " " };
    let line1 = Line::from(vec![
        Span::styled(format!("{marker} "), bold(ACCENT)),
        Span::styled(format!("{icon} "), bold(color)),
        Span::styled(
            fit(&v.title, title_w),
            if selected { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() },
        ),
        Span::styled(format!("{:>w$}", truncate(&state_text, state_w), w = state_w), fg(color)),
    ]);
    f.render_widget(Paragraph::new(line1), Rect::new(rect.x, rect.y, left_w as u16, 1));

    // Second line: bar, percentage, speed and ETA (or the error for failed jobs).
    let line2 = match v.state {
        DownloadState::Failed => {
            let msg = v.error.clone().unwrap_or_else(|| "failed".into());
            Line::from(vec![Span::raw("    "), Span::styled(truncate(&msg, left_w.saturating_sub(4)), fg(Color::Red))])
        }
        DownloadState::Cancelled => Line::from(Span::styled("    cancelled", fg(DIM))),
        DownloadState::Queued => Line::from(Span::styled("    waiting for a free slot", fg(DIM))),
        _ => {
            let pct = v.percent.map(|p| format!("{p:5.1}%")).unwrap_or_else(|| "  ...".into());
            let speed = v.speed.map(|s| format!("{}/s", human_bytes(s))).unwrap_or_default();
            let eta = v.eta.map(|e| format!("ETA {}", human_eta(e))).unwrap_or_default();
            let tail = format!(" {pct:>6}  {speed:>11}  {eta:<9}");
            let bar_w = left_w.saturating_sub(4 + width(&tail)).max(6);
            let mut spans = vec![Span::raw("    ")];
            spans.extend(bar_spans(bar_w, v.percent, app.tick, v.state, color));
            spans.push(Span::raw(tail));
            Line::from(spans)
        }
    };
    f.render_widget(Paragraph::new(line2), Rect::new(rect.x, rect.y + 1, left_w as u16, 1));

    // Control buttons, 5x2 cells each, only drawn (and clickable) when they apply.
    let ascii = ascii_icons();
    let can_cancel = v.state == DownloadState::Queued || v.state == DownloadState::Running;
    let controls: [(Action, &str, &str, bool, Color); 3] = [
        (Action::Cancel(v.id), if ascii { "x" } else { "✕" }, "stop", can_cancel, Color::Red),
        (Action::Retry(v.id), if ascii { "r" } else { "↻" }, "retry", !can_cancel, Color::Yellow),
        (Action::Remove(v.id), if ascii { "d" } else { "🗑" }, "del", true, Color::Magenta),
    ];
    for (i, (action, glyph, caption, enabled, accent)) in controls.into_iter().enumerate() {
        let r = Rect::new(rect.right() - CTL_W + (i as u16) * 6, rect.y, 5, 2);
        if enabled {
            let style = button_style(app, action, Style::new().fg(Color::White).bg(Color::Indexed(238)), accent);
            let text = vec![
                Line::from(format!(" {glyph} ")).alignment(Alignment::Center),
                Line::from(caption).alignment(Alignment::Center),
            ];
            f.render_widget(Paragraph::new(text).style(style), r);
            app.register(r, action);
        }
    }
}

fn bar_spans(w: usize, percent: Option<f64>, tick: u64, state: DownloadState, color: Color) -> Vec<Span<'static>> {
    match percent {
        Some(p) => {
            let filled = ((p / 100.0) * w as f64).round() as usize;
            let filled = filled.min(w);
            vec![Span::styled("█".repeat(filled), fg(color)), Span::styled("░".repeat(w - filled), fg(DIM))]
        }
        None if state == DownloadState::Running && w > 6 => {
            // Unknown total size: a short block sweeps back and forth.
            let seg = 4usize;
            let span = w - seg;
            let t = (tick / 2) as usize % (span * 2);
            let pos = if t <= span { t } else { span * 2 - t };
            vec![
                Span::styled("░".repeat(pos), fg(DIM)),
                Span::styled("█".repeat(seg), fg(color)),
                Span::styled("░".repeat(w - seg - pos), fg(DIM)),
            ]
        }
        None => vec![Span::styled("░".repeat(w), fg(DIM))],
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(DIM))
        .title(Span::styled(" Details (wheel or PgUp/PgDn scrolls) ", fg(Color::Gray)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.register(area, Action::Wheel(Wheel::Details));
    if inner.height == 0 {
        return;
    }
    let lines = match app.jobs.get(app.sel_job) {
        Some(job) => details_lines(job, inner.width as usize),
        None => {
            app.details_max = 0;
            f.render_widget(Paragraph::new("Select a download to see its details.").style(fg(DIM)), inner);
            return;
        }
    };
    let max = lines.len().saturating_sub(inner.height as usize);
    app.details_max = max;
    if app.details_pin {
        app.details_scroll = max;
    }
    app.details_scroll = app.details_scroll.min(max);
    let visible: Vec<Line> = lines.into_iter().skip(app.details_scroll).take(inner.height as usize).collect();
    f.render_widget(Paragraph::new(visible), inner);
    draw_scrollbar(f, area, max, app.details_scroll);
}

fn push_kv(lines: &mut Vec<Line<'static>>, label: &str, value: &str, w: usize, style: Style) {
    const LABEL_W: usize = 10;
    let val_w = w.saturating_sub(LABEL_W).max(8);
    for (i, chunk) in wrap_text(value, val_w).into_iter().enumerate() {
        let lab = if i == 0 { format!("{label:<LABEL_W$}") } else { " ".repeat(LABEL_W) };
        lines.push(Line::from(vec![Span::styled(lab, fg(DIM)), Span::styled(chunk, style)]));
    }
}

fn details_lines(job: &Job, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_kv(&mut lines, "URL", &job.url, w, Style::new());
    push_kv(&mut lines, "Title", job.title.as_deref().unwrap_or("(not known yet)"), w, Style::new());
    let mut state = job.state.label().to_string();
    if job.state == DownloadState::Running && !job.phase.is_empty() {
        state = format!("{state}: {}", job.phase);
    }
    if job.run > 1 {
        state = format!("{state} (run {})", job.run);
    }
    push_kv(&mut lines, "State", &state, w, bold(state_color(job.state)));
    push_kv(&mut lines, "Options", &job.opts.summary(), w, Style::new());
    let mut progress = match job.percent() {
        Some(p) => format!("{p:.1}%"),
        None => "unknown".into(),
    };
    if job.downloaded > 0.0 {
        progress.push_str(&format!("  {}", human_bytes(job.downloaded)));
        if let Some(t) = job.total {
            progress.push_str(&format!(" of {}", human_bytes(t)));
        }
    }
    if let Some(s) = job.speed {
        progress.push_str(&format!("  {}/s", human_bytes(s)));
    }
    if let Some(e) = job.eta {
        progress.push_str(&format!("  ETA {}", human_eta(e)));
    }
    push_kv(&mut lines, "Progress", &progress, w, Style::new());
    push_kv(&mut lines, "Output", job.output.as_deref().unwrap_or("(not reported yet)"), w, Style::new());
    if let Some(e) = &job.error {
        push_kv(&mut lines, "Error", e, w, fg(Color::Red));
    }
    if !job.command.is_empty() {
        push_kv(&mut lines, "Command", &job.command, w, fg(DIM));
    }
    lines.push(Line::from(Span::styled("Recent log", bold(Color::Gray))));
    if job.log.is_empty() {
        lines.push(Line::from(Span::styled("  (nothing yet)", fg(DIM))));
    }
    for entry in job.log.iter().rev().take(60).rev() {
        let style = if entry.starts_with("ERROR") {
            fg(Color::Red)
        } else if entry.starts_with("WARNING") {
            fg(Color::Yellow)
        } else {
            fg(Color::Gray)
        };
        for chunk in wrap_text(entry, w.saturating_sub(2)) {
            lines.push(Line::from(Span::styled(format!("  {chunk}"), style)));
        }
    }
    lines
}

// ---------------------------------------------------------------------------------------
// Settings tab
// ---------------------------------------------------------------------------------------

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let [list_a, hint_a] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(DIM))
        .title(Span::styled(" Settings (saved as you change them) ", fg(Color::Gray)));
    let inner = block.inner(list_a);
    f.render_widget(block, list_a);
    app.register(list_a, Action::Wheel(Wheel::Settings));

    let n = SettingKey::ALL.len();
    let visible = (inner.height as usize).max(1);
    app.settings_sel = app.settings_sel.min(n - 1);
    if app.settings_sel < app.settings_scroll {
        app.settings_scroll = app.settings_sel;
    } else if app.settings_sel >= app.settings_scroll + visible {
        app.settings_scroll = app.settings_sel + 1 - visible;
    }
    app.settings_scroll = app.settings_scroll.min(n.saturating_sub(visible));

    const LABEL_W: u16 = 28;
    let value_w = inner.width.saturating_sub(LABEL_W + 2).min(48);
    for (i, key) in SettingKey::ALL.iter().copied().enumerate().skip(app.settings_scroll).take(visible) {
        let y = inner.y + (i - app.settings_scroll) as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = i == app.settings_sel;
        if selected {
            f.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        app.register(row, Action::SettingSelect(key));
        let marker = if selected { "> " } else { "  " };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker, bold(ACCENT)),
                Span::styled(
                    fit(key.label(), LABEL_W as usize - 2),
                    if selected { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() },
                ),
            ])),
            Rect::new(row.x, y, LABEL_W.min(row.width), 1),
        );
        if value_w >= 14 {
            draw_setting_value(f, app, key, Rect::new(inner.x + LABEL_W, y, value_w, 1));
        }
    }
    draw_scrollbar(f, list_a, n.saturating_sub(visible), app.settings_scroll);

    // Explanation of the selected row.
    let key = SettingKey::ALL[app.settings_sel];
    let hint_block = Block::bordered().border_type(BorderType::Rounded).border_style(fg(DIM));
    let hint_inner = hint_block.inner(hint_a);
    f.render_widget(hint_block, hint_a);
    let mut lines: Vec<Line> = wrap_text(key.hint(), hint_inner.width as usize)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, fg(Color::Gray))))
        .collect();
    if key.kind() == SettingKind::Text {
        lines.push(Line::from(Span::styled(
            "Click the value or press Enter to edit. Enter saves, Esc cancels.",
            fg(DIM),
        )));
    }
    if let Some(p) = Config::path() {
        lines.push(Line::from(Span::styled(
            truncate(&format!("Config file: {}", p.display()), hint_inner.width as usize),
            fg(DIM),
        )));
    }
    f.render_widget(Paragraph::new(lines), hint_inner);
}

fn draw_setting_value(f: &mut Frame, app: &mut App, key: SettingKey, zone: Rect) {
    let y = zone.y;
    match key.kind() {
        SettingKind::Choice | SettingKind::Int => {
            let left = Rect::new(zone.x, y, 5, 1);
            let right = Rect::new(zone.right() - 5, y, 5, 1);
            let mid = Rect::new(zone.x + 5, y, zone.width - 10, 1);
            for (r, glyph, action) in
                [(left, " < ", Action::SettingPrev(key)), (right, " > ", Action::SettingNext(key))]
            {
                let style = button_style(app, action, bold(ACCENT), ACCENT);
                f.render_widget(Paragraph::new(glyph).style(style), Rect::new(r.x + 1, y, 3, 1));
                app.register(r, action);
            }
            let action = Action::SettingValue(key);
            let style = button_style(app, action, Style::new().add_modifier(Modifier::BOLD), ACCENT);
            f.render_widget(
                Paragraph::new(truncate(&app.setting_text(key), mid.width as usize))
                    .alignment(Alignment::Center)
                    .style(style),
                mid,
            );
            app.register(mid, action);
        }
        SettingKind::Bool => {
            let on = app.setting_is_on(key);
            let action = Action::SettingValue(key);
            let base = if on { bold(Color::Green) } else { fg(Color::Gray) };
            let style = button_style(app, action, base, ACCENT);
            let text = if on { "[x] on " } else { "[ ] off" };
            let r = Rect::new(zone.x, y, 14.min(zone.width), 1);
            f.render_widget(Paragraph::new(format!("  {text}")).style(style), r);
            app.register(r, action);
        }
        SettingKind::Text => {
            let action = Action::SettingValue(key);
            app.register(zone, action);
            let editing_here = app.editing.as_ref().filter(|e| e.key == key);
            if let Some(ed) = editing_here {
                let chars: Vec<char> = ed.edit.text.chars().collect();
                let cursor = ed.edit.cursor.min(chars.len());
                let avail = zone.width.saturating_sub(2) as usize;
                let cw = |s: &[char]| s.iter().map(|c| char_width(*c)).sum::<usize>();
                let mut start = 0;
                while start < cursor && cw(&chars[start..cursor]) >= avail {
                    start += 1;
                }
                let mut shown = String::new();
                let mut used = 0;
                for &c in &chars[start..] {
                    let w = char_width(c);
                    if used + w > avail {
                        break;
                    }
                    shown.push(c);
                    used += w;
                }
                let caret_x = zone.x + 1 + cw(&chars[start..cursor]) as u16;
                f.render_widget(
                    Paragraph::new(format!(" {shown}")).style(Style::new().bg(Color::Indexed(238)).fg(Color::White)),
                    zone,
                );
                f.set_cursor_position((caret_x.min(zone.right() - 1), y));
            } else {
                let text = app.setting_text(key);
                let (text, base) =
                    if text.is_empty() { ("(empty, click to set)".to_string(), fg(DIM)) } else { (text, Style::new()) };
                let style = button_style(app, action, base, ACCENT);
                f.render_widget(
                    Paragraph::new(format!(" {}", truncate(&text, zone.width as usize - 2))).style(style),
                    zone,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Help tab
// ---------------------------------------------------------------------------------------

fn draw_help(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(DIM))
        .title(Span::styled(" Help (wheel, arrows, PgUp/PgDn scroll) ", fg(Color::Gray)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.register(area, Action::Wheel(Wheel::Help));
    if inner.height == 0 {
        return;
    }
    let lines = help_lines(app, inner.width as usize);
    let max = lines.len().saturating_sub(inner.height as usize);
    app.help_scroll = app.help_scroll.min(max);
    let visible: Vec<Line> = lines.into_iter().skip(app.help_scroll).take(inner.height as usize).collect();
    f.render_widget(Paragraph::new(visible), inner);
    draw_scrollbar(f, area, max, app.help_scroll);
}

fn heading(lines: &mut Vec<Line<'static>>, text: &str) {
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(text.to_string(), bold(ACCENT))));
}

fn entry(lines: &mut Vec<Line<'static>>, key: &str, desc: &str, w: usize) {
    const KEY_W: usize = 26;
    let desc_w = w.saturating_sub(KEY_W + 2).max(10);
    for (i, chunk) in wrap_text(desc, desc_w).into_iter().enumerate() {
        let k = if i == 0 { fit(key, KEY_W) } else { " ".repeat(KEY_W) };
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(k, bold(Color::White)), Span::raw(chunk)]));
    }
}

fn help_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let mut l: Vec<Line<'static>> = Vec::new();

    heading(&mut l, "Keys, everywhere");
    entry(&mut l, "F1 F2 F3 F4", "Download, Downloads, Settings, Help. Alt+Left / Alt+Right also switch tabs.", w);
    entry(&mut l, "1 2 3 4  ?", "Same as F1-F4 and Help, when you are not typing in a text field.", w);
    entry(&mut l, "q or Ctrl+Q", "Quit. With active downloads, press twice (they are cancelled).", w);
    entry(&mut l, "Ctrl+C", "Quit immediately and cancel everything.", w);

    heading(&mut l, "Keys, Download tab");
    entry(
        &mut l,
        "Tab / Down",
        "Next control: URL, Mode, Video quality, Video codec, Audio quality, Container, Start.",
        w,
    );
    entry(&mut l, "Shift+Tab / Up", "Previous control.", w);
    entry(&mut l, "Left / Right", "Step a picker back or forward (moves the caret in the URL field).", w);
    entry(&mut l, "Space", "Next value of the focused picker.", w);
    entry(&mut l, "Enter", "URL or Start: start the download. Picker: next value.", w);
    entry(&mut l, "Esc", "Clear focus (then q and 1-4 work as shortcuts). Tab, arrows, Enter or i focus again.", w);
    entry(&mut l, "Ctrl+U / Ctrl+A / Ctrl+E", "In the URL field: clear, jump to start, jump to end.", w);
    entry(&mut l, "PgUp / PgDn", "Scroll the command preview.", w);

    heading(&mut l, "Keys, Downloads tab");
    entry(&mut l, "Up Down / j k", "Select a download (the list follows).", w);
    entry(&mut l, "c or x", "Cancel the selected download.", w);
    entry(&mut l, "r", "Retry (also re-downloads finished items).", w);
    entry(&mut l, "d or Delete", "Remove from the list (cancels first if running).", w);
    entry(&mut l, "C", "Clear all finished downloads.", w);
    entry(&mut l, "PgUp / PgDn", "Scroll the details pane. Scrolling to the bottom follows new log lines.", w);

    heading(&mut l, "Keys, Settings tab");
    entry(&mut l, "Up Down / j k", "Select a setting. PgUp, PgDn, Home, End jump.", w);
    entry(&mut l, "Left Right / h l", "Change a picker, stepper or switch.", w);
    entry(&mut l, "Enter / Space", "Advance a picker, flip a switch, or open the inline editor for text.", w);
    entry(
        &mut l,
        "In the editor",
        "Enter saves, Esc cancels, arrows/Home/End/Backspace/Delete edit. Pasting works.",
        w,
    );

    heading(&mut l, "Mouse");
    entry(
        &mut l,
        "Left click",
        "Tab titles, < > arrows, values, Start, job rows and their stop/retry/del buttons, Clear finished, settings rows.",
        w,
    );
    entry(&mut l, "Click the URL field", "Focuses it and puts the caret under the pointer. Drag to move the caret.", w);
    entry(&mut l, "Wheel on a picker", "Cycles its value (down = next, up = previous).", w);
    entry(
        &mut l,
        "Wheel on lists",
        "Scrolls the queue, details, preview and help. In Settings it moves the selection.",
        w,
    );
    entry(&mut l, "Right click", "Same as Esc: clears focus or cancels an open edit.", w);
    entry(&mut l, "Middle click", "Does nothing except print a reminder.", w);
    entry(&mut l, "Hover", "Highlights tabs and buttons under the pointer.", w);
    entry(&mut l, "Inline editor", "Click elsewhere to save. Click the same field again to cancel.", w);

    heading(&mut l, "How options become yt-dlp flags");
    entry(&mut l, "Mode: Video + audio", "-f <selector> --merge-output-format <container>", w);
    entry(&mut l, "Mode: Audio only", "-x --audio-format <fmt> --audio-quality <rate>", w);
    entry(&mut l, "Video quality", "[height<=N] on both the video part and the fallback (Best adds no limit).", w);
    entry(&mut l, "Video codec", "H.264 [vcodec^=avc1], H.265 hev1/hvc1, VP9 vp09, AV1 av01. Any adds no filter.", w);
    entry(
        &mut l,
        "Audio quality",
        "Audio only: AAC 128/192/256/320 -> --audio-format aac, Opus 160 -> opus, MP3 -> mp3, with --audio-quality in K. Video mode: prefers a matching stream (mp4a, opus or mp3) and falls back to any audio.",
        w,
    );
    entry(&mut l, "Container", "--merge-output-format mkv, mp4 or webm. Ignored in audio-only mode.", w);
    entry(&mut l, "Output folder", "-P <folder> -o '%(title)s [%(id)s].%(ext)s'", w);
    entry(&mut l, "Embed thumbnail", "--embed-thumbnail", w);
    entry(&mut l, "Embed metadata", "--embed-metadata", w);
    entry(&mut l, "Write subtitles", "--write-subs --sub-langs en.*,en", w);
    entry(&mut l, "Playlist handling", "--no-playlist or --yes-playlist", w);
    entry(&mut l, "Cookies from browser", "--cookies-from-browser <value>", w);
    entry(&mut l, "ffmpeg path", "--ffmpeg-location <path> (only when it differs from plain 'ffmpeg')", w);
    entry(
        &mut l,
        "Always added",
        "--newline --no-quiet --progress --progress-template ... --print after_move:FINAL|%(filepath)s --no-simulate --no-colors",
        w,
    );
    entry(&mut l, "Extra args", "Appended last, before -- and the URL, so they can override anything above.", w);

    heading(&mut l, "Requirements");
    entry(&mut l, "yt-dlp", "Required. Any recent release. Needs a working Python or the standalone build.", w);
    entry(&mut l, "ffmpeg", "Needed to merge video and audio, convert audio, and embed thumbnails or metadata.", w);

    heading(&mut l, "Resolved paths");
    let found = |p: &Option<std::path::PathBuf>| match p {
        Some(p) => p.display().to_string(),
        None => "NOT FOUND (set the path in Settings)".to_string(),
    };
    entry(&mut l, "yt-dlp", &found(&app.resolved_ytdlp), w);
    entry(&mut l, "ffmpeg", &found(&app.resolved_ffmpeg), w);
    entry(
        &mut l,
        "Config file",
        &Config::path().map(|p| p.display().to_string()).unwrap_or_else(|| "(none)".into()),
        w,
    );
    entry(&mut l, "Output folder", &app.config.output_dir, w);
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn new_app() -> App {
        let cfg = Config { output_dir: "/tmp/ytget-test".into(), ..Config::default() };
        App::with_config(cfg, false)
    }

    fn render(app: &mut App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        t
    }

    fn screen(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn rect_of(app: &App, action: Action) -> Rect {
        app.hits
            .iter()
            .rev()
            .find(|h| h.action == action)
            .unwrap_or_else(|| panic!("no hit registered for {action:?}"))
            .rect
    }

    fn mouse(app: &mut App, kind: MouseEventKind, col: u16, row: u16) {
        app.on_mouse(MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE });
    }

    fn click_at(app: &mut App, col: u16, row: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), col, row);
        mouse(app, MouseEventKind::Up(MouseButton::Left), col, row);
    }

    /// Click the middle of a registered region, redrawing first like the real loop does.
    fn click(app: &mut App, action: Action) {
        render(app, 100, 30);
        let r = rect_of(app, action);
        click_at(app, r.x + r.width / 2, r.y + r.height / 2);
    }

    fn key(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn every_tab_renders_at_several_sizes_without_panicking() {
        for (w, h) in [(60, 22), (80, 24), (100, 30), (140, 50), (40, 10), (20, 5)] {
            for tab in Tab::ALL {
                let mut app = new_app();
                app.tab = tab;
                app.enqueue("https://example.com/v".into());
                render(&mut app, w, h);
            }
        }
    }

    #[test]
    fn download_tab_shows_defaults_arrows_and_command() {
        let mut app = new_app();
        let t = render(&mut app, 100, 30);
        let s = screen(&t);
        for needle in ["1080p", "H.264", "AAC 192 kbps", "MKV", "Video + audio", "Start download", " < ", " > "] {
            assert!(s.contains(needle), "missing {needle:?} on screen:\n{s}");
        }
        assert!(s.contains("yt-dlp"), "preview missing:\n{s}");
    }

    #[test]
    fn clicking_tab_titles_switches_tabs() {
        let mut app = new_app();
        for tab in [Tab::Settings, Tab::Help, Tab::Downloads, Tab::Download] {
            click(&mut app, Action::Tab(tab));
            assert_eq!(app.tab, tab);
        }
    }

    #[test]
    fn hovering_a_tab_sets_the_hover_state() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        let r = rect_of(&app, Action::Tab(Tab::Help));
        mouse(&mut app, MouseEventKind::Moved, r.x + 2, r.y + 1);
        assert_eq!(app.hover, Some(Action::Tab(Tab::Help)));
        mouse(&mut app, MouseEventKind::Moved, 0, 29);
        assert_eq!(app.hover, None);
    }

    #[test]
    fn picker_arrows_value_and_wheel_change_the_selection() {
        let mut app = new_app();
        assert_eq!(app.form.video_quality, VideoQuality::P1080);
        click(&mut app, Action::Next(FormRow::VideoQuality));
        assert_eq!(app.form.video_quality, VideoQuality::P720);
        click(&mut app, Action::Prev(FormRow::VideoQuality));
        click(&mut app, Action::Prev(FormRow::VideoQuality));
        assert_eq!(app.form.video_quality, VideoQuality::P1440);
        click(&mut app, Action::Value(FormRow::Container));
        assert_eq!(app.form.container, Container::Mp4);

        render(&mut app, 100, 30);
        let r = rect_of(&app, Action::Wheel(Wheel::Picker(FormRow::VideoCodec)));
        mouse(&mut app, MouseEventKind::ScrollDown, r.x + 10, r.y + 1);
        assert_eq!(app.form.video_codec, VideoCodec::H265);
        mouse(&mut app, MouseEventKind::ScrollUp, r.x + 10, r.y + 1);
        mouse(&mut app, MouseEventKind::ScrollUp, r.x + 10, r.y + 1);
        assert_eq!(app.form.video_codec, VideoCodec::Any);
    }

    #[test]
    fn arrows_wrap_around_at_both_ends() {
        let mut app = new_app();
        app.form.container = Container::Webm;
        click(&mut app, Action::Next(FormRow::Container));
        assert_eq!(app.form.container, Container::Mkv);
        click(&mut app, Action::Prev(FormRow::Container));
        assert_eq!(app.form.container, Container::Webm);
    }

    #[test]
    fn preview_updates_with_the_selection() {
        let mut app = new_app();
        app.form.mode = Mode::AudioOnly;
        let t = render(&mut app, 120, 40);
        assert!(screen(&t).contains("--audio-format"), "{}", screen(&t));
    }

    #[test]
    fn url_click_places_the_caret_and_typing_inserts_there() {
        let mut app = new_app();
        app.url = TextEdit::new("hello world");
        render(&mut app, 100, 30);
        let r = rect_of(&app, Action::FocusUrl);
        let x = app.url_text_x + 5;
        click_at(&mut app, x, r.y + 1);
        assert_eq!(app.form_focus, Some(FormRow::Url));
        assert_eq!(app.url.cursor, 5);
        key(&mut app, KeyCode::Char('X'));
        assert_eq!(app.url.text, "helloX world");
    }

    #[test]
    fn dragging_in_the_url_field_moves_the_caret() {
        let mut app = new_app();
        app.url = TextEdit::new("abcdefghij");
        render(&mut app, 100, 30);
        let r = rect_of(&app, Action::FocusUrl);
        let x0 = app.url_text_x;
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x0 + 2, r.y + 1);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x0 + 7, r.y + 1);
        assert_eq!(app.url.cursor, 7);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x0 + 7, r.y + 1);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x0 + 1, r.y + 1);
        assert_eq!(app.url.cursor, 7, "drag after release must not move the caret");
    }

    #[test]
    fn right_click_clears_focus_and_keyboard_recovers() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Right), 50, 10);
        assert_eq!(app.form_focus, None);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.form_focus, Some(FormRow::Url));
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.form.mode, Mode::AudioOnly);
    }

    #[test]
    fn middle_click_and_horizontal_scroll_are_harmless() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Middle), 50, 10);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Middle), 50, 10);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Middle), 51, 10);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Right), 51, 10);
        mouse(&mut app, MouseEventKind::ScrollLeft, 50, 10);
        mouse(&mut app, MouseEventKind::ScrollRight, 50, 10);
        assert_eq!(app.tab, Tab::Download);
        assert_eq!(app.form, Config::default().selection());
    }

    #[test]
    fn clicks_on_empty_space_and_borders_do_nothing() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        let before = (app.form, app.tab, app.form_focus);
        // Footer row, and the far right of the tab bar, are not registered anywhere.
        click_at(&mut app, 50, 29);
        click_at(&mut app, 99, 1);
        assert_eq!((app.form, app.tab, app.form_focus), before);
        assert!(app.pressed.is_none());
    }

    #[test]
    fn hit_map_follows_window_resizes() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        let small = rect_of(&app, Action::Next(FormRow::Mode));
        render(&mut app, 140, 40);
        let big = rect_of(&app, Action::Next(FormRow::Mode));
        assert_ne!(small, big);
        click_at(&mut app, big.x + 2, big.y + 1);
        assert_eq!(app.form.mode, Mode::AudioOnly);
    }

    #[test]
    fn smallest_rect_wins_when_regions_overlap() {
        let mut app = new_app();
        render(&mut app, 100, 30);
        let card = rect_of(&app, Action::Value(FormRow::Mode));
        // The wheel region covers the whole card but must never swallow a click.
        let hit = app.resolve(card.x + card.width / 2, card.y + 1, false).unwrap();
        assert_eq!(hit.action, Action::Value(FormRow::Mode));
        let wheel = app.resolve(card.x + card.width / 2, card.y + 1, true).unwrap();
        assert_eq!(wheel.action, Action::Wheel(Wheel::Picker(FormRow::Mode)));
    }

    #[test]
    fn click_targets_are_never_single_cells() {
        let mut app = new_app();
        for tab in Tab::ALL {
            app.tab = tab;
            app.enqueue("https://example.com/v".into());
            render(&mut app, 100, 30);
            for h in &app.hits {
                if matches!(h.action, Action::Wheel(_)) {
                    continue;
                }
                assert!(h.rect.width >= 3, "{:?} is too narrow: {:?}", h.action, h.rect);
            }
        }
    }

    #[test]
    fn start_button_without_a_url_reports_an_error() {
        let mut app = new_app();
        click(&mut app, Action::Start);
        assert!(app.jobs.is_empty());
        let s = app.status.as_ref().unwrap();
        assert_eq!(s.level, Level::Error);
    }

    #[test]
    fn start_button_enqueues_the_url_with_current_options() {
        let mut app = new_app();
        app.resolved_ytdlp = Some("/bin/true".into());
        app.config.max_concurrent = 1;
        app.url = TextEdit::new("https://youtu.be/abc");
        app.form.video_quality = VideoQuality::P720;
        click(&mut app, Action::Start);
        assert_eq!(app.jobs.len(), 1);
        assert_eq!(app.jobs[0].url, "https://youtu.be/abc");
        assert_eq!(app.jobs[0].opts.sel.video_quality, VideoQuality::P720);
        assert!(app.url.text.is_empty());
        app.shutdown();
    }

    #[test]
    fn job_row_controls_cancel_retry_remove_and_select() {
        let mut app = new_app();
        app.tab = Tab::Downloads;
        app.enqueue("https://example.com/a".into());
        app.enqueue("https://example.com/b".into());
        let (a, b) = (app.jobs[0].id, app.jobs[1].id);

        click(&mut app, Action::SelectJob(a));
        assert_eq!(app.selected_id(), Some(a));

        click(&mut app, Action::Cancel(b));
        assert_eq!(app.jobs[1].state, DownloadState::Cancelled);
        assert_eq!(app.selected_id(), Some(b));

        render(&mut app, 100, 30);
        assert!(app.hits.iter().all(|h| h.action != Action::Cancel(b)), "cancel must disappear for finished jobs");

        app.resolved_ytdlp = None; // keep retry from starting a process in this test
        app.config.max_concurrent = 1;
        click(&mut app, Action::Retry(b));
        assert_eq!(app.jobs[1].state, DownloadState::Queued);

        click(&mut app, Action::Remove(a));
        assert_eq!(app.jobs.len(), 1);
        assert_eq!(app.jobs[0].id, b);
    }

    #[test]
    fn job_list_wheel_scrolls_without_changing_selection() {
        let mut app = new_app();
        app.tab = Tab::Downloads;
        for i in 0..12 {
            app.enqueue(format!("https://example.com/{i}"));
        }
        render(&mut app, 100, 30);
        let sel = app.sel_job;
        let r = rect_of(&app, Action::Wheel(Wheel::JobList));
        for _ in 0..3 {
            mouse(&mut app, MouseEventKind::ScrollUp, r.x + 5, r.y + 4);
        }
        render(&mut app, 100, 30);
        assert_eq!(app.sel_job, sel);
        assert!(app.list_scroll < sel, "wheel up should move the viewport up");
    }

    #[test]
    fn keyboard_has_the_same_job_controls() {
        let mut app = new_app();
        app.tab = Tab::Downloads;
        app.enqueue("https://example.com/a".into());
        app.enqueue("https://example.com/b".into());
        key(&mut app, KeyCode::Up);
        assert_eq!(app.sel_job, 0);
        key(&mut app, KeyCode::Char('c'));
        assert_eq!(app.jobs[0].state, DownloadState::Cancelled);
        key(&mut app, KeyCode::Char('d'));
        assert_eq!(app.jobs.len(), 1);
    }

    #[test]
    fn settings_click_to_flip_cycle_and_select() {
        let mut app = new_app();
        app.tab = Tab::Settings;
        assert!(!app.config.embed_thumbnail);
        click(&mut app, Action::SettingValue(SettingKey::EmbedThumbnail));
        assert!(app.config.embed_thumbnail);
        assert_eq!(app.settings_sel, SettingKey::EmbedThumbnail.index());

        click(&mut app, Action::SettingNext(SettingKey::Container));
        assert_eq!(app.config.container, Container::Mp4);
        assert_eq!(app.form.container, Container::Mp4, "changing a default also updates the form");
        click(&mut app, Action::SettingPrev(SettingKey::MaxConcurrent));
        assert_eq!(app.config.max_concurrent, 1);
        click(&mut app, Action::SettingPrev(SettingKey::MaxConcurrent));
        assert_eq!(app.config.max_concurrent, 8, "stepper wraps");

        click(&mut app, Action::SettingSelect(SettingKey::WriteSubs));
        assert_eq!(app.settings_sel, SettingKey::WriteSubs.index());
    }

    #[test]
    fn inline_editor_commits_on_click_away_and_cancels_on_second_click() {
        let mut app = new_app();
        app.tab = Tab::Settings;

        click(&mut app, Action::SettingValue(SettingKey::CookiesBrowser));
        assert!(app.editing.is_some());
        for c in "firefox".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        // Click a different row: the edit is committed.
        click(&mut app, Action::SettingSelect(SettingKey::WriteSubs));
        assert!(app.editing.is_none());
        assert_eq!(app.config.cookies_from_browser, "firefox");

        // Open again, type, then click the same field: cancelled, value unchanged.
        click(&mut app, Action::SettingValue(SettingKey::CookiesBrowser));
        key(&mut app, KeyCode::Char('!'));
        click(&mut app, Action::SettingValue(SettingKey::CookiesBrowser));
        assert!(app.editing.is_none());
        assert_eq!(app.config.cookies_from_browser, "firefox");
    }

    #[test]
    fn clicking_another_text_field_commits_and_opens_the_next_editor() {
        let mut app = new_app();
        app.tab = Tab::Settings;
        click(&mut app, Action::SettingValue(SettingKey::ExtraArgs));
        for c in "--limit-rate 1M".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        click(&mut app, Action::SettingValue(SettingKey::CookiesBrowser));
        assert_eq!(app.config.extra_args, "--limit-rate 1M");
        assert_eq!(app.editing.as_ref().map(|e| e.key), Some(SettingKey::CookiesBrowser));
    }

    #[test]
    fn editor_keys_enter_saves_escape_and_right_click_cancel() {
        let mut app = new_app();
        app.tab = Tab::Settings;
        click(&mut app, Action::SettingValue(SettingKey::ExtraArgs));
        key(&mut app, KeyCode::Char('a'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.config.extra_args, "a");

        click(&mut app, Action::SettingValue(SettingKey::ExtraArgs));
        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.config.extra_args, "a");

        click(&mut app, Action::SettingValue(SettingKey::ExtraArgs));
        key(&mut app, KeyCode::Char('c'));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Right), 5, 5);
        assert!(app.editing.is_none());
        assert_eq!(app.config.extra_args, "a");
    }

    #[test]
    fn settings_wheel_moves_the_selection() {
        let mut app = new_app();
        app.tab = Tab::Settings;
        render(&mut app, 100, 30);
        let r = rect_of(&app, Action::Wheel(Wheel::Settings));
        mouse(&mut app, MouseEventKind::ScrollDown, r.x + 5, r.y + 3);
        mouse(&mut app, MouseEventKind::ScrollDown, r.x + 5, r.y + 3);
        assert_eq!(app.settings_sel, 2);
        mouse(&mut app, MouseEventKind::ScrollUp, r.x + 5, r.y + 3);
        assert_eq!(app.settings_sel, 1);
    }

    #[test]
    fn help_wheel_scrolls_the_page() {
        let mut app = new_app();
        app.tab = Tab::Help;
        render(&mut app, 80, 24);
        let r = rect_of(&app, Action::Wheel(Wheel::Help));
        mouse(&mut app, MouseEventKind::ScrollDown, r.x + 5, r.y + 3);
        assert!(app.help_scroll > 0);
        let t = render(&mut app, 80, 24);
        assert!(!screen(&t).contains("Keys, everywhere"), "first heading should have scrolled away");
        mouse(&mut app, MouseEventKind::ScrollUp, r.x + 5, r.y + 3);
        mouse(&mut app, MouseEventKind::ScrollUp, r.x + 5, r.y + 3);
        assert_eq!(app.help_scroll, 0);
    }

    #[test]
    fn help_lists_resolved_paths_and_keymap() {
        let mut app = new_app();
        app.tab = Tab::Help;
        app.resolved_ytdlp = Some("/opt/bin/yt-dlp".into());
        app.help_scroll = usize::MAX / 2;
        let t = render(&mut app, 100, 40);
        let s = screen(&t);
        assert!(s.contains("/opt/bin/yt-dlp") || s.contains("NOT FOUND"), "{s}");
        app.help_scroll = 0;
        let t = render(&mut app, 100, 60);
        assert!(screen(&t).contains("Right click"), "mouse map missing from help");
    }

    #[test]
    fn function_keys_and_digits_switch_tabs_but_not_while_typing() {
        let mut app = new_app();
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(app.tab, Tab::Download, "digit goes into the URL field while it is focused");
        assert_eq!(app.url.text, "2");
        key(&mut app, KeyCode::F(3));
        assert_eq!(app.tab, Tab::Settings);
        key(&mut app, KeyCode::Char('4'));
        assert_eq!(app.tab, Tab::Help);
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(app.tab, Tab::Settings);
    }

    #[test]
    fn quitting_with_active_downloads_needs_confirmation() {
        let mut app = new_app();
        app.form_focus = None;
        app.enqueue("https://example.com/a".into());
        key(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit);
        key(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn too_small_terminal_shows_a_message_and_no_hits() {
        let mut app = new_app();
        let t = render(&mut app, 40, 10);
        assert!(screen(&t).contains("too small"));
        assert!(app.hits.is_empty());
    }
}
