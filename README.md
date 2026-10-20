# ytget-rs

A mouse-friendly terminal UI for downloading video and audio with yt-dlp. It works with any site yt-dlp supports, not only YouTube. The app is a Rust binary, but **end users do not need Rust installed**: GitHub Releases provide prebuilt binaries, and the installer downloads the app and its runtime dependencies.

## Install without Rust

Supported releases are **Windows x64**, **Linux x86_64 / ARM64**, and **macOS Intel / Apple Silicon**. The scripts install the app into a per-user directory and add that directory to your PATH. They do not require `cargo` or a Rust toolchain on the user's machine.

### Before publishing this repository

Replace `YOUR_GITHUB_USERNAME/ytget-rs` in `install.sh`, `update.sh`, `install.ps1`, and `update.ps1` with your actual GitHub `owner/repository` (or have users set `YTGET_REPO=owner/repository`). Push the repository's default branch as `main`; the one-line commands below assume this branch name.

### Windows (PowerShell)

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/install.ps1 | iex
```

To update the app and its managed dependencies:

```powershell
irm https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/update.ps1 | iex
```

The script installs to `%LOCALAPPDATA%\Programs\ytget-rs\bin` and adds that directory to your user PATH. Open a new terminal after installation. Close ytget-rs before running the updater so Windows can replace the executable. Windows builds currently target x64.

### Linux (Bash)

```bash
curl -fsSL https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/install.sh | bash
```

Update everything managed by ytget-rs:

```bash
curl -fsSL https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/update.sh | bash
```

The app and yt-dlp are installed in `~/.local/bin`. FFmpeg is installed or upgraded with the distribution's package manager (`apt`, `dnf`, `yum`, `pacman`, `zypper`, or `apk`); the script uses `sudo` when the package manager needs elevated permissions. Arch Linux package updates require a full system upgrade, as required by Arch's package-management model.

### macOS (Terminal)

```bash
curl -fsSL https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/install.sh | bash
```

Update:

```bash
curl -fsSL https://raw.githubusercontent.com/YOUR_GITHUB_USERNAME/ytget-rs/main/update.sh | bash
```

The app and yt-dlp are installed in `~/.local/bin`. FFmpeg is managed through Homebrew; if Homebrew is missing, the script invokes Homebrew's official installer. Homebrew may require Xcode Command Line Tools. If installation stops for that reason, run `xcode-select --install` and rerun the installer.

### What the scripts install and update

| Component | Source | Installer/updater behavior |
| --- | --- | --- |
| `ytget-rs` | This project's GitHub Releases | Downloads the latest prebuilt binary for the detected OS/architecture and verifies its SHA-256 checksum. |
| `yt-dlp` | [Official yt-dlp GitHub releases](https://github.com/yt-dlp/yt-dlp/releases) | Downloads the platform-specific standalone executable and verifies the checksum published by yt-dlp. |
| FFmpeg + FFprobe | Linux distribution packages; Homebrew on macOS; [Windows builds linked by FFmpeg.org](https://ffmpeg.org/download.html) | Installs missing FFmpeg or upgrades the installed package/build. The Windows script verifies the archive's published SHA-256 checksum. |

FFmpeg's own download page explains that the FFmpeg project publishes source code and links to providers of prebuilt binaries. Consequently, the FFmpeg binary itself is supplied by a platform package or a provider linked from FFmpeg.org, not hosted by this project.

After installing, open a new terminal and run:

```text
ytget-rs
```

The app still needs a terminal that supports mouse events and is at least 60 columns by 22 rows. Inside tmux, run `set -g mouse on`.

## Build from source (developers)

Building from source requires Rust 1.85 or newer (edition 2024). End users should use the prebuilt install scripts above instead.

```bash
cargo build --release
./target/release/ytget-rs
```

To install a locally built copy onto your PATH: `cargo install --path .`.

If the build runs out of memory on a small machine, limit it to one job: `CARGO_BUILD_JOBS=1 cargo build --locked --release`. The release profile turns off LTO and uses multiple codegen units to lower peak memory.

`cargo test --locked` runs the unit tests, including tests that draw every tab to an in-memory terminal and replay clicks, drags and wheel events against it.

## Publish a new release

1. Push the complete repository to GitHub and make sure `main` is the default branch.
2. Replace `YOUR_GITHUB_USERNAME/ytget-rs` in `install.sh`, `update.sh`, `install.ps1`, `update.ps1`, and the example URLs in this README with your actual GitHub `owner/repository`.
3. On GitHub, open **Releases → Draft a new release**. Choose **Create new tag** and enter `v0.1.0`; set the target to `main`, enter a release title and notes, and click **Publish release**.
4. Publishing the release starts the **Build and release** workflow. In **Actions**, wait for all five platform build jobs to succeed. The workflow then attaches the prebuilt archives and `SHA256SUMS` to the release.
5. Check that the release page contains the Windows ZIP, four Unix `.tar.gz` archives, and `SHA256SUMS`. Users can then install without Rust.
6. For later app releases, commit and push your source changes to `main`, then publish a new release using a new tag such as `v0.1.1`. The install and update scripts always target the latest GitHub Release.

The release workflow currently produces Linux x86_64 and ARM64, macOS Intel and Apple Silicon, and Windows x64 builds. The installer scripts independently update yt-dlp and FFmpeg so dependency updates do not require a new ytget-rs release.

## Using it

1. On the Download tab, paste or type a URL.
2. Pick mode, quality, codec, audio quality and container, or leave the defaults (MKV, 1080p, H.264, AAC 192 kbps).
3. Click Start download, or press Enter in the URL field.
4. Watch it on the Downloads tab.

The preview at the bottom of the Download tab shows the exact yt-dlp command your picks produce. If you paste several links separated by spaces, each becomes its own download.

## Keymap

Everywhere:

| Key | Action |
| --- | --- |
| F1 F2 F3 F4 | Download, Downloads, Settings, Help |
| Alt+Left, Alt+Right | Previous or next tab |
| 1 2 3 4, ? | Same as F1 to F4 and Help, when you are not typing in a text field |
| q, Ctrl+Q | Quit. With active downloads, press twice; they are cancelled |
| Ctrl+C | Quit at once and cancel everything |

Download tab. The URL field starts focused, so letters and digits go into it until you press Esc or Tab away.

| Key | Action |
| --- | --- |
| Tab, Down | Next control (URL, Mode, Video quality, Video codec, Audio quality, Container, Start) |
| Shift+Tab, Up | Previous control |
| Left, Right | Step the focused picker; in the URL field, move the caret |
| Space | Next value of the focused picker |
| Enter | URL or Start: begin the download. Picker: next value |
| Esc | Clear focus. After that, q and 1 to 4 work as shortcuts. Tab, arrows, Enter or i focus again |
| Ctrl+U, Ctrl+A, Ctrl+E | In the URL field: clear, start of line, end of line |
| Home, End, Backspace, Delete | Edit the URL |
| PgUp, PgDn | Scroll the command preview |

Downloads tab:

| Key | Action |
| --- | --- |
| Up, Down, j, k | Select a download. The list follows the selection |
| Home, End | First or last download |
| c, x | Cancel the selected download |
| r | Retry. Also re-downloads a finished item |
| d, Delete | Remove from the list. A running download is cancelled first |
| C | Clear all finished downloads |
| PgUp, PgDn | Scroll the details pane. Scrolling to the bottom follows new log lines |

Settings tab:

| Key | Action |
| --- | --- |
| Up, Down, j, k | Select a setting. PgUp, PgDn, Home and End jump |
| Left, Right, h, l | Change a picker or stepper, or flip a switch |
| Enter, Space | Advance a picker, flip a switch, or open the inline editor on a text field |
| In the editor | Enter saves. Esc cancels. Arrows, Home, End, Backspace, Delete edit. Ctrl+U clears. Pasting works |

Help tab: Up, Down, j, k, PgUp, PgDn, Home, End scroll.

## Mouse map

Left click does the action. The scroll wheel acts on whatever is under the pointer.

| Region | Left click | Wheel |
| --- | --- | --- |
| Tab title | Switch to that tab. The tab under the pointer is highlighted | |
| URL field | Focus it and put the caret under the pointer. Drag to move the caret | |
| Picker `<` arrow | Previous value | Over the picker: up is previous, down is next |
| Picker `>` arrow | Next value | |
| Picker value | Next value | |
| Start download | Queue the current URL with the current picks | |
| Command preview | | Scroll |
| Job row | Select it | Scroll the list without changing the selection |
| `✕` stop | Cancel that download (queued or running) | |
| `↻` retry | Queue it again (failed, cancelled or finished) | |
| `🗑` del | Remove it, cancelling first if it is running | |
| Clear finished | Remove all finished downloads | |
| Details pane | | Scroll |
| Settings row | Select it | Move the selection up or down |
| Settings arrows | Previous or next value | |
| Settings value | Advance a picker, flip a switch, or open the inline editor | |
| Help page | | Scroll |
| Right click (anywhere) | Same as Esc: clear focus on the Download tab, or cancel an open edit | |
| Middle click | Does nothing except show a reminder | |
| Moving the pointer | Highlights the tab or button under it | |

Rules the click handling follows:

- Every region registers itself while it is drawn, and a click goes to the smallest region under the pointer. Resizing the window needs no special handling.
- Picker arrows are 6 cells wide and as tall as the card (3 rows). The card border counts as part of the card. Job controls are 5 cells by 2 rows. No target is a single cell.
- Clicks on panel borders and empty space are ignored.
- With a settings text field open, clicking anywhere else saves it, and clicking the same field again cancels it.
- Mouse capture is switched on at startup and off again on every exit path, including a panic.

## How options become yt-dlp flags

| Option | Flags |
| --- | --- |
| Video + audio | `-f <selector> --merge-output-format <mkv, mp4 or webm>` |
| Audio only | `-x --audio-format <aac, opus, mp3 or best> --audio-quality <128K, 160K, 192K, 256K, 320K or 0>` |
| Video quality | `[height<=N]` on the video part and the last fallback. Best adds no limit |
| Video codec | H.264 `[vcodec^=avc1]`, H.265 `[vcodec~='^(hev1\|hvc1\|hevc)']`, VP9 `[vcodec~='^(vp09\|vp9)']`, AV1 `[vcodec^=av01]`. Any adds nothing |
| Audio quality, video mode | Prefers an audio stream with a matching codec (`mp4a` for AAC, `opus`, `mp3`), then falls back to any audio. The bitrate cannot be chosen when yt-dlp is only picking an existing stream |
| Audio quality, audio-only mode | Sets the output codec and bitrate as above |
| Container | `--merge-output-format`. Not used in audio-only mode |
| Output folder | `-P <folder> -o '%(title)s [%(id)s].%(ext)s'` |
| Embed thumbnail | `--embed-thumbnail` |
| Embed metadata | `--embed-metadata` |
| Write subtitles | `--write-subs --sub-langs en.*,en` |
| Playlist handling | `--no-playlist` or `--yes-playlist` |
| Cookies from browser | `--cookies-from-browser <value>` |
| ffmpeg path | `--ffmpeg-location <path>`, only when it is not plain `ffmpeg` |
| Extra args | Appended last, just before `--` and the URL, so they override earlier flags |

With the defaults, the selector is:

```
bestvideo[height<=1080][vcodec^=avc1]+bestaudio[acodec^=mp4a]/bestvideo[height<=1080][vcodec^=avc1]+bestaudio/best[height<=1080]
```

Every download also gets these, which is how progress and the final path are read:

```
--newline --no-quiet --progress
--progress-template "download:PROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.title)s"
--print "after_move:FINAL|%(filepath)s" --no-simulate --no-colors
```

`--print` implies `--quiet`, which would hide the progress lines, so `--no-quiet` is needed. `--no-simulate` makes sure the download still happens when `--print` is used.

The preview shows a warning when the container cannot hold the chosen streams. WebM only takes VP9 or AV1 video with Opus audio, and MP4 does not take VP9 or Opus. In those cases yt-dlp merges into MKV instead.

## Config file

Settings are saved as JSON every time you change one, and loaded at launch.

| System | Location |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/ytget-rs/config.json`, usually `~/.config/ytget-rs/config.json` |
| macOS | `~/Library/Application Support/ytget-rs/config.json` |
| Windows | `%APPDATA%\ytget-rs\config.json` |

The Help tab and the bottom of the Settings tab show the exact path on your machine. Every key is optional; missing ones take their defaults.

```json
{
  "output_dir": "/home/you/Downloads",
  "mode": "VideoAudio",
  "video_quality": "P1080",
  "video_codec": "H264",
  "audio_quality": "Aac192",
  "container": "Mkv",
  "max_concurrent": 2,
  "embed_thumbnail": false,
  "embed_metadata": true,
  "write_subs": false,
  "playlist": "Single",
  "cookies_from_browser": "",
  "extra_args": "",
  "ytdlp_path": "yt-dlp",
  "ffmpeg_path": "ffmpeg"
}
```

Allowed values:

| Key | Values |
| --- | --- |
| mode | `VideoAudio`, `AudioOnly` |
| video_quality | `Best`, `P2160`, `P1440`, `P1080`, `P720`, `P480`, `P360` |
| video_codec | `Any`, `H264`, `H265`, `Vp9`, `Av1` |
| audio_quality | `Best`, `Aac128`, `Aac192`, `Aac256`, `Aac320`, `Opus160`, `Mp3192`, `Mp3320` |
| container | `Mkv`, `Mp4`, `Webm` |
| playlist | `Single`, `Whole` |
| max_concurrent | 1 to 8 |

If the file cannot be parsed, it is renamed to `config.json.bad` and defaults are used.

## How it works

- `main.rs` sets up the terminal and runs the loop. It redraws and polls input every 100 ms, and it drains bursts of mouse events before drawing.
- `app.rs` holds state and turns key and mouse events into changes. `ui.rs` draws, and fills `App::hits` with `(Rect, Action)` pairs on every frame.
- `ytdlp.rs` builds arguments and parses output. `download.rs` runs yt-dlp on one worker thread per download and sends messages back over a channel that the loop drains with `try_recv`, so the UI never waits on yt-dlp.
- Cancelling kills the process. On Unix each download runs in its own process group and the group is stopped, so the ffmpeg that yt-dlp started goes with it. On Windows it uses `taskkill /T /F`.
- Up to `max_concurrent` downloads run at once. The rest wait as Queued.
- The terminal is restored by a drop guard and by a panic hook. A SIGTERM, SIGHUP or SIGKILL cannot be caught without a signal-handling crate, so after one of those, run `reset`.

## Troubleshooting

- "yt-dlp was not found": install it, or give the full path in Settings. The Help tab shows what was resolved.
- Mouse does nothing: check that your terminal reports mouse events, and turn on tmux mouse mode if you use tmux.
- The trash icon looks too wide or misaligned: some terminals and fonts draw that emoji at double width. Start with `YTGET_ASCII=1` to use plain letters instead.
- A download fails with "Requested format is not available": the chosen quality or codec does not exist for that video. Try Best, or Any codec.
- Age-restricted or members-only videos: set Cookies from browser to the browser you are logged in with, for example `firefox`.
