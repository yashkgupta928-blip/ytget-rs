//! Option enums shared by the UI, the config file and the yt-dlp argument builder.

use serde::{Deserialize, Serialize};

/// A value picked from a fixed, ordered list (what the `<` / `>` arrows cycle through).
pub trait Choice: Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    fn label(self) -> &'static str;

    fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }

    /// Move `delta` steps through the list, wrapping at both ends.
    fn step(self, delta: i32) -> Self {
        let n = Self::ALL.len() as i32;
        Self::ALL[(self.index() as i32 + delta).rem_euclid(n) as usize]
    }
}

macro_rules! choice_enum {
    ($name:ident { $($var:ident => $label:expr),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($var),+ }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$var),+];
            fn label(self) -> &'static str {
                match self { $(Self::$var => $label),+ }
            }
        }
    };
}

choice_enum!(Mode {
    VideoAudio => "Video + audio",
    AudioOnly => "Audio only",
});

choice_enum!(VideoQuality {
    Best => "Best",
    P2160 => "2160p",
    P1440 => "1440p",
    P1080 => "1080p",
    P720 => "720p",
    P480 => "480p",
    P360 => "360p",
});

choice_enum!(VideoCodec {
    Any => "Any",
    H264 => "H.264",
    H265 => "H.265",
    Vp9 => "VP9",
    Av1 => "AV1",
});

choice_enum!(AudioQuality {
    Best => "Best",
    Aac128 => "AAC 128 kbps",
    Aac192 => "AAC 192 kbps",
    Aac256 => "AAC 256 kbps",
    Aac320 => "AAC 320 kbps",
    Opus160 => "Opus 160 kbps",
    Mp3192 => "MP3 192 kbps",
    Mp3320 => "MP3 320 kbps",
});

choice_enum!(Container {
    Mkv => "MKV",
    Mp4 => "MP4",
    Webm => "WebM",
});

choice_enum!(PlaylistMode {
    Single => "Single video only",
    Whole => "Whole playlist",
});

impl VideoQuality {
    pub fn height(self) -> Option<u32> {
        match self {
            Self::Best => None,
            Self::P2160 => Some(2160),
            Self::P1440 => Some(1440),
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
            Self::P360 => Some(360),
        }
    }
}

impl VideoCodec {
    /// yt-dlp format filter that restricts video streams to this codec.
    pub fn filter(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::H264 => Some("[vcodec^=avc1]"),
            Self::H265 => Some("[vcodec~='^(hev1|hvc1|hevc)']"),
            Self::Vp9 => Some("[vcodec~='^(vp09|vp9)']"),
            Self::Av1 => Some("[vcodec^=av01]"),
        }
    }
}

impl AudioQuality {
    /// Format filter used in video mode to prefer a matching audio stream.
    pub fn stream_filter(self) -> Option<&'static str> {
        match self {
            Self::Best => None,
            Self::Aac128 | Self::Aac192 | Self::Aac256 | Self::Aac320 => Some("[acodec^=mp4a]"),
            Self::Opus160 => Some("[acodec=opus]"),
            Self::Mp3192 | Self::Mp3320 => Some("[acodec=mp3]"),
        }
    }

    /// `(--audio-format, --audio-quality)` used in audio-only mode.
    pub fn extract_args(self) -> (&'static str, &'static str) {
        match self {
            Self::Best => ("best", "0"),
            Self::Aac128 => ("aac", "128K"),
            Self::Aac192 => ("aac", "192K"),
            Self::Aac256 => ("aac", "256K"),
            Self::Aac320 => ("aac", "320K"),
            Self::Opus160 => ("opus", "160K"),
            Self::Mp3192 => ("mp3", "192K"),
            Self::Mp3320 => ("mp3", "320K"),
        }
    }

    pub fn is_aac(self) -> bool {
        matches!(self, Self::Aac128 | Self::Aac192 | Self::Aac256 | Self::Aac320)
    }

    pub fn is_mp3(self) -> bool {
        matches!(self, Self::Mp3192 | Self::Mp3320)
    }
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }
}

/// The five pickers on the Download tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub mode: Mode,
    pub video_quality: VideoQuality,
    pub video_codec: VideoCodec,
    pub audio_quality: AudioQuality,
    pub container: Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Queued,
    Running,
    Finished,
    Failed,
    Cancelled,
}

impl DownloadState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Finished => "Finished",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}
