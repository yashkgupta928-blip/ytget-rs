//! Builds yt-dlp command lines and parses its output. No UI code in here.

use crate::config::{Config, DEFAULT_FFMPEG, expand_tilde};
use crate::types::*;

pub const PROGRESS_TEMPLATE: &str = "download:PROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.title)s";
pub const PRINT_FINAL: &str = "after_move:FINAL|%(filepath)s";
pub const OUTPUT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// Everything that decides what a single download looks like. A copy is stored on each
/// queued job so later changes in Settings do not alter jobs that are already queued.
#[derive(Debug, Clone)]
pub struct DlOptions {
    pub sel: Selection,
    pub output_dir: String,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub write_subs: bool,
    pub playlist: PlaylistMode,
    pub cookies_from_browser: String,
    pub extra_args: String,
    pub ffmpeg_path: String,
}

impl DlOptions {
    pub fn new(sel: Selection, cfg: &Config) -> Self {
        Self {
            sel,
            output_dir: cfg.output_dir.clone(),
            embed_thumbnail: cfg.embed_thumbnail,
            embed_metadata: cfg.embed_metadata,
            write_subs: cfg.write_subs,
            playlist: cfg.playlist,
            cookies_from_browser: cfg.cookies_from_browser.clone(),
            extra_args: cfg.extra_args.clone(),
            ffmpeg_path: cfg.ffmpeg_path.clone(),
        }
    }

    /// One-line description for the details pane.
    pub fn summary(&self) -> String {
        let s = &self.sel;
        match s.mode {
            Mode::VideoAudio => format!(
                "{} | {} | {} | audio {} | {}",
                s.mode.label(),
                s.video_quality.label(),
                s.video_codec.label(),
                s.audio_quality.label(),
                s.container.label()
            ),
            Mode::AudioOnly => format!("{} | {}", s.mode.label(), s.audio_quality.label()),
        }
    }
}

/// The `-f` selector for video + audio mode. With the defaults this is exactly
/// `bestvideo[height<=1080][vcodec^=avc1]+bestaudio[acodec^=mp4a]/bestvideo[height<=1080][vcodec^=avc1]+bestaudio/best[height<=1080]`.
pub fn format_selector(sel: &Selection) -> String {
    let height = sel.video_quality.height().map(|h| format!("[height<={h}]")).unwrap_or_default();
    let vcodec = sel.video_codec.filter().unwrap_or("");
    let acodec = sel.audio_quality.stream_filter().unwrap_or("");
    let video = format!("bestvideo{height}{vcodec}");
    let mut alternatives = Vec::new();
    if !acodec.is_empty() {
        alternatives.push(format!("{video}+bestaudio{acodec}"));
    }
    alternatives.push(format!("{video}+bestaudio"));
    alternatives.push(format!("best{height}"));
    alternatives.join("/")
}

pub fn build_args(o: &DlOptions, url: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut add = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));

    // Machine-readable progress and the final file path.
    add(&["--newline", "--no-quiet", "--progress", "--progress-template", PROGRESS_TEMPLATE]);
    add(&["--print", PRINT_FINAL, "--no-simulate", "--no-colors"]);

    let dir = expand_tilde(&o.output_dir);
    if !dir.is_empty() {
        add(&["-P", &dir]);
    }
    add(&["-o", OUTPUT_TEMPLATE]);
    add(&[if o.playlist == PlaylistMode::Whole { "--yes-playlist" } else { "--no-playlist" }]);

    match o.sel.mode {
        Mode::VideoAudio => {
            add(&["-f", &format_selector(&o.sel)]);
            add(&["--merge-output-format", o.sel.container.ext()]);
        }
        Mode::AudioOnly => {
            let (format, quality) = o.sel.audio_quality.extract_args();
            add(&["-x", "--audio-format", format, "--audio-quality", quality]);
        }
    }

    if o.embed_thumbnail {
        add(&["--embed-thumbnail"]);
    }
    if o.embed_metadata {
        add(&["--embed-metadata"]);
    }
    if o.write_subs {
        add(&["--write-subs", "--sub-langs", "en.*,en"]);
    }
    let cookies = o.cookies_from_browser.trim();
    if !cookies.is_empty() {
        add(&["--cookies-from-browser", cookies]);
    }
    let ffmpeg = o.ffmpeg_path.trim();
    if !ffmpeg.is_empty() && ffmpeg != DEFAULT_FFMPEG {
        add(&["--ffmpeg-location", &expand_tilde(ffmpeg)]);
    }

    // Extra arguments come last so they can override anything above.
    for extra in split_args(&o.extra_args) {
        a.push(extra);
    }
    a.push("--".into());
    a.push(url.to_string());
    a
}

/// What the user would paste into a POSIX shell to get the same download.
pub fn command_preview(program: &str, o: &DlOptions, url: &str) -> String {
    std::iter::once(program.to_string())
        .chain(build_args(o, url))
        .map(|s| shell_quote(&s))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A hint when the chosen container cannot hold the chosen streams, because yt-dlp
/// quietly falls back to MKV in that case.
pub fn compat_warning(sel: &Selection) -> Option<String> {
    if sel.mode != Mode::VideoAudio {
        return None;
    }
    let aq = sel.audio_quality;
    match sel.container {
        Container::Webm
            if matches!(sel.video_codec, VideoCodec::H264 | VideoCodec::H265) || aq.is_aac() || aq.is_mp3() =>
        {
            Some(
                "WebM only holds VP9/AV1 video with Opus audio. With these picks yt-dlp will merge into MKV instead."
                    .into(),
            )
        }
        Container::Mp4 if sel.video_codec == VideoCodec::Vp9 || aq == AudioQuality::Opus160 => Some(
            "MP4 does not take VP9 or Opus streams from YouTube. With these picks yt-dlp will merge into MKV instead."
                .into(),
        ),
        _ => None,
    }
}

pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

/// Split a string into arguments, honouring single and double quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub downloaded: Option<f64>,
    pub total: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Progress(Progress),
    Final(String),
    /// Anything else. `phase` is set when the line tells us what yt-dlp is doing now.
    Text {
        text: String,
        phase: Option<&'static str>,
    },
}

pub fn parse_line(raw: &str) -> Parsed {
    let line = clean_line(raw);
    if let Some(rest) = line.strip_prefix("PROG|") {
        let mut parts = rest.splitn(6, '|');
        let downloaded = parts.next().and_then(num);
        let total = parts.next().and_then(num);
        let estimate = parts.next().and_then(num);
        let speed = parts.next().and_then(num);
        let eta = parts.next().and_then(num).map(|v| v.max(0.0) as u64);
        let title = parts.next().map(str::trim).filter(|t| !t.is_empty() && *t != "NA").map(String::from);
        return Parsed::Progress(Progress { downloaded, total: total.or(estimate), speed, eta, title });
    }
    if let Some(path) = line.strip_prefix("FINAL|") {
        return Parsed::Final(path.trim().to_string());
    }
    let phase = phase_of(&line);
    Parsed::Text { text: line, phase }
}

fn num(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s == "NA" || s == "None" { None } else { s.parse().ok() }
}

fn phase_of(line: &str) -> Option<&'static str> {
    const TABLE: &[(&str, &str)] = &[
        ("[Merger]", "Merging"),
        ("[ExtractAudio]", "Extracting audio"),
        ("[VideoConvertor]", "Converting"),
        ("[EmbedThumbnail]", "Embedding thumbnail"),
        ("[ThumbnailsConvertor]", "Embedding thumbnail"),
        ("[Metadata]", "Writing metadata"),
        ("[EmbedSubtitle]", "Embedding subtitles"),
        ("[MoveFiles]", "Moving files"),
        ("[download] Destination", "Downloading"),
        ("[info]", "Fetching info"),
        ("[youtube]", "Fetching info"),
    ];
    TABLE.iter().find(|(prefix, _)| line.starts_with(prefix)).map(|(_, p)| *p)
}

/// Strip ANSI escape sequences and control characters from a line of tool output.
pub fn clean_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_control() && c != '\t' {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(sel: Selection) -> DlOptions {
        let cfg = Config { output_dir: "/tmp/out".into(), embed_metadata: false, ..Config::default() };
        DlOptions::new(sel, &cfg)
    }

    #[test]
    fn default_format_selector_matches_the_spec_exactly() {
        let sel = Config::default().selection();
        assert_eq!(
            format_selector(&sel),
            "bestvideo[height<=1080][vcodec^=avc1]+bestaudio[acodec^=mp4a]/bestvideo[height<=1080][vcodec^=avc1]+bestaudio/best[height<=1080]"
        );
    }

    #[test]
    fn unrestricted_selector_is_short() {
        let sel = Selection {
            video_quality: VideoQuality::Best,
            video_codec: VideoCodec::Any,
            audio_quality: AudioQuality::Best,
            ..Config::default().selection()
        };
        assert_eq!(format_selector(&sel), "bestvideo+bestaudio/best");
    }

    #[test]
    fn video_args_have_merge_progress_and_print_flags() {
        let args = build_args(&opts(Config::default().selection()), "https://youtu.be/x");
        let joined = args.join(" ");
        assert!(joined.contains("--merge-output-format mkv"));
        assert!(joined.contains("--newline --no-quiet --progress --progress-template"));
        assert!(joined.contains("--print after_move:FINAL|%(filepath)s --no-simulate"));
        assert!(joined.contains("-P /tmp/out"));
        assert!(joined.ends_with("-- https://youtu.be/x"));
        assert!(!joined.contains("--embed-metadata"));
    }

    #[test]
    fn audio_only_default_uses_aac_192k() {
        let sel = Selection { mode: Mode::AudioOnly, ..Config::default().selection() };
        let joined = build_args(&opts(sel), "u").join(" ");
        assert!(joined.contains("-x --audio-format aac --audio-quality 192K"));
        assert!(!joined.contains("--merge-output-format"));
    }

    #[test]
    fn extra_args_come_last_before_the_url() {
        let mut o = opts(Config::default().selection());
        o.extra_args = "--limit-rate 2M --add-header 'X-Test: a b'".into();
        let args = build_args(&o, "u");
        let n = args.len();
        assert_eq!(&args[n - 6..], ["--limit-rate", "2M", "--add-header", "X-Test: a b", "--", "u"]);
    }

    #[test]
    fn quoting_for_the_preview() {
        assert_eq!(shell_quote("abc-1.2"), "abc-1.2");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn parses_progress_with_missing_values_and_pipes_in_title() {
        let p = parse_line("PROG|1000|NA|2000|500.5|3|Some | Title");
        assert_eq!(
            p,
            Parsed::Progress(Progress {
                downloaded: Some(1000.0),
                total: Some(2000.0),
                speed: Some(500.5),
                eta: Some(3),
                title: Some("Some | Title".into()),
            })
        );
        let p = parse_line("PROG|0|NA|NA|NA|NA|NA");
        assert_eq!(p, Parsed::Progress(Progress { downloaded: Some(0.0), ..Default::default() }));
    }

    #[test]
    fn parses_final_path_and_phases() {
        assert_eq!(parse_line("FINAL|/tmp/a b.mkv"), Parsed::Final("/tmp/a b.mkv".into()));
        match parse_line("[Merger] Merging formats into \"x.mkv\"") {
            Parsed::Text { phase, .. } => assert_eq!(phase, Some("Merging")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn ansi_codes_are_stripped() {
        assert_eq!(clean_line("\u{1b}[0;31mERROR:\u{1b}[0m boom"), "ERROR: boom");
    }

    #[test]
    fn container_warnings() {
        let mut sel = Config::default().selection();
        assert!(compat_warning(&sel).is_none());
        sel.container = Container::Webm;
        assert!(compat_warning(&sel).is_some());
        sel.video_codec = VideoCodec::Vp9;
        sel.audio_quality = AudioQuality::Opus160;
        assert!(compat_warning(&sel).is_none());
    }
}
