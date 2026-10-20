//! Small text helpers that respect display width (so wide characters do not break layouts).

use ratatui::text::Span;

pub fn width(s: &str) -> usize {
    Span::raw(s).width()
}

pub fn char_width(c: char) -> usize {
    Span::raw(c.to_string()).width()
}

/// Shorten `s` to at most `max` columns, ending with an ellipsis when something was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Pad (or cut) to exactly `w` columns.
pub fn fit(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let pad = w.saturating_sub(width(&t));
    format!("{t}{}", " ".repeat(pad))
}

/// Greedy word wrap. Words longer than the line are split.
pub fn wrap_text(s: &str, w: usize) -> Vec<String> {
    let w = w.max(4);
    let mut out = Vec::new();
    for paragraph in s.split('\n') {
        let mut line = String::new();
        let mut line_w = 0;
        for word in paragraph.split(' ') {
            let ww = width(word);
            if ww > w {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let mut chunk = String::new();
                let mut cw = 0;
                for c in word.chars() {
                    let x = char_width(c);
                    if cw + x > w {
                        out.push(std::mem::take(&mut chunk));
                        cw = 0;
                    }
                    chunk.push(c);
                    cw += x;
                }
                line = chunk;
                line_w = cw;
            } else if line_w == 0 {
                line.push_str(word);
                line_w = ww;
            } else if line_w + 1 + ww <= w {
                line.push(' ');
                line.push_str(word);
                line_w += 1 + ww;
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
                line_w = ww;
            }
        }
        out.push(line);
    }
    out
}

/// Which character index a click lands on, given the first visible character and the
/// number of columns from the left edge of the text area.
pub fn char_index_at(text: &str, first_visible: usize, columns: usize) -> usize {
    let mut used = 0;
    let mut idx = first_visible;
    for c in text.chars().skip(first_visible) {
        let w = char_width(c);
        if columns < used + w.div_ceil(2).max(1) {
            return idx;
        }
        used += w;
        idx += 1;
    }
    idx
}

pub fn human_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b.max(0.0);
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0} {}", UNITS[i]) } else { format!("{v:.1} {}", UNITS[i]) }
}

pub fn human_eta(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello world", 20), "hello world");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(width(&truncate("日本語のタイトル", 7)), 7);
    }

    #[test]
    fn wraps_words_and_long_tokens() {
        assert_eq!(wrap_text("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap_text("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("a\nb", 10), vec!["a", "b"]);
    }

    #[test]
    fn click_column_maps_to_a_character() {
        assert_eq!(char_index_at("hello", 0, 0), 0);
        assert_eq!(char_index_at("hello", 0, 3), 3);
        assert_eq!(char_index_at("hello", 0, 99), 5);
        assert_eq!(char_index_at("hello", 2, 1), 3);
    }

    #[test]
    fn formats_sizes_and_times() {
        assert_eq!(human_bytes(512.0), "512 B");
        assert_eq!(human_bytes(1536.0), "1.5 KiB");
        assert_eq!(human_eta(75), "01:15");
        assert_eq!(human_eta(3725), "1:02:05");
    }
}
