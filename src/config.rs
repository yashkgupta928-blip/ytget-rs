//! Persisted settings (JSON) plus small helpers for locating binaries and paths.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::*;

pub const APP_DIR: &str = "ytget-rs";
pub const DEFAULT_YTDLP: &str = "yt-dlp";
pub const DEFAULT_FFMPEG: &str = "ffmpeg";
pub const MAX_CONCURRENT_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output_dir: String,
    pub mode: Mode,
    pub video_quality: VideoQuality,
    pub video_codec: VideoCodec,
    pub audio_quality: AudioQuality,
    pub container: Container,
    pub max_concurrent: usize,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub write_subs: bool,
    pub playlist: PlaylistMode,
    pub cookies_from_browser: String,
    pub extra_args: String,
    pub ytdlp_path: String,
    pub ffmpeg_path: String,
}

impl Default for Config {
    fn default() -> Self {
        let output_dir = dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();
        Self {
            output_dir,
            mode: Mode::VideoAudio,
            video_quality: VideoQuality::P1080,
            video_codec: VideoCodec::H264,
            audio_quality: AudioQuality::Aac192,
            container: Container::Mkv,
            max_concurrent: 2,
            embed_thumbnail: false,
            embed_metadata: true,
            write_subs: false,
            playlist: PlaylistMode::Single,
            cookies_from_browser: String::new(),
            extra_args: String::new(),
            ytdlp_path: DEFAULT_YTDLP.into(),
            ffmpeg_path: DEFAULT_FFMPEG.into(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_DIR).join("config.json"))
    }

    pub fn selection(&self) -> Selection {
        Selection {
            mode: self.mode,
            video_quality: self.video_quality,
            video_codec: self.video_codec,
            audio_quality: self.audio_quality,
            container: self.container,
        }
    }

    /// Load the config file. A missing file gives defaults silently; an unreadable one
    /// gives defaults plus a warning (the broken file is moved aside, not deleted).
    pub fn load() -> (Self, Option<String>) {
        let Some(path) = Self::path() else {
            return (Self::default(), Some("No config directory found; settings will not be saved.".into()));
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => return (Self::default(), Some(format!("Could not read {}: {e}", path.display()))),
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                cfg.sanitize();
                (cfg, None)
            }
            Err(e) => {
                let bad = path.with_extension("json.bad");
                let moved = std::fs::rename(&path, &bad).is_ok();
                let note = if moved { format!(" (old file kept as {})", bad.display()) } else { String::new() };
                (Self::default(), Some(format!("Config was invalid ({e}); using defaults{note}")))
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no config directory available")?;
        let dir = path.parent().context("config path has no parent")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn sanitize(&mut self) {
        self.max_concurrent = self.max_concurrent.clamp(1, MAX_CONCURRENT_LIMIT);
        if self.ytdlp_path.trim().is_empty() {
            self.ytdlp_path = DEFAULT_YTDLP.into();
        }
        if self.ffmpeg_path.trim().is_empty() {
            self.ffmpeg_path = DEFAULT_FFMPEG.into();
        }
        if self.output_dir.trim().is_empty() {
            self.output_dir = Self::default().output_dir;
        }
    }
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(s: &str) -> String {
    let s = s.trim();
    if s == "~" {
        return dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|| s.into());
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest).to_string_lossy().into_owned();
        }
    }
    s.to_string()
}

/// Find an executable the way a shell would: explicit paths are checked directly,
/// bare names are searched on PATH (with PATHEXT on Windows). A directory is accepted
/// if it contains an executable called `fallback_name`.
pub fn resolve_binary(configured: &str, fallback_name: &str) -> Option<PathBuf> {
    let configured = expand_tilde(configured);
    if configured.is_empty() {
        return None;
    }
    let p = Path::new(&configured);
    if p.is_dir() {
        return search_dir(p, fallback_name);
    }
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| search_dir(&dir, &configured))
}

fn search_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut candidates = vec![dir.join(name)];
    if cfg!(windows) {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        candidates.extend(exts.split(';').filter(|e| !e.is_empty()).map(|e| dir.join(format!("{name}{e}"))));
    }
    candidates.into_iter().find(|c| c.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let c = Config::default();
        assert_eq!(c.container, Container::Mkv);
        assert_eq!(c.video_quality, VideoQuality::P1080);
        assert_eq!(c.video_codec, VideoCodec::H264);
        assert_eq!(c.audio_quality, AudioQuality::Aac192);
    }

    #[test]
    fn partial_json_fills_in_defaults() {
        let c: Config = serde_json::from_str(r#"{"container":"Mp4","max_concurrent":3}"#).unwrap();
        assert_eq!(c.container, Container::Mp4);
        assert_eq!(c.max_concurrent, 3);
        assert_eq!(c.video_quality, VideoQuality::P1080);
    }

    #[test]
    fn sanitize_clamps_values() {
        let mut c = Config { max_concurrent: 99, ytdlp_path: "  ".into(), ..Config::default() };
        c.sanitize();
        assert_eq!(c.max_concurrent, MAX_CONCURRENT_LIMIT);
        assert_eq!(c.ytdlp_path, DEFAULT_YTDLP);
    }
}
