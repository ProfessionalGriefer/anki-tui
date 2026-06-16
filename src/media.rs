//! Extract images and audio from card HTML, build image protocols, play audio.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use html2text::render::{RichAnnotation, TaggedLineElement};
use image::GenericImageView;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::anki::AnkiConnect;

/// One renderable piece of a card side, kept in the order it appears in the
/// card HTML so images sit inline with the text rather than all at the bottom.
pub enum Block {
    /// HTML fragment (media tokens removed), rendered to styled text on demand.
    Text(String),
    /// Index into [`SideMedia::images`].
    Image(usize),
}

/// A decoded image plus its natural size in terminal cells, so it can be drawn
/// near its original dimensions instead of being stretched to fill a pane.
pub struct CardImage {
    /// Terminal-ready protocol (re-encoded on resize).
    pub protocol: StatefulProtocol,
    pub cols: u16,
    pub rows: u16,
}

/// Everything renderable/playable for one side of a card.
pub struct SideMedia {
    /// Text and image blocks in the order they appear in the card HTML.
    pub blocks: Vec<Block>,
    /// Decoded images referenced by [`Block::Image`].
    pub images: Vec<CardImage>,
    /// Paths to audio files written to a temp dir, ready for the player.
    pub audio: Vec<PathBuf>,
}

impl SideMedia {
    /// Build the media for one HTML fragment, fetching referenced files from Anki.
    /// The HTML is split at `<img>` tags so each image keeps its position in the
    /// text flow; runs of text between images become [`Block::Text`] entries.
    pub fn build(html: &str, anki: &AnkiConnect, picker: &Picker) -> Self {
        let font = picker.font_size();
        let mut blocks = Vec::new();
        let mut images = Vec::new();

        // Audio is collected up front and played as a group, so order doesn't matter.
        let mut audio = Vec::new();
        for name in extract_audio(html) {
            if let Ok(Some(bytes)) = anki.retrieve_media_file(&name)
                && let Ok(path) = write_temp(&name, &bytes)
            {
                audio.push(path);
            }
        }

        let lower = html.to_ascii_lowercase();
        let mut i = 0;
        let mut text_start = 0;
        while i < html.len() {
            if lower[i..].starts_with("<img")
                && let Some(end_rel) = html[i..].find('>')
            {
                let tag_end = i + end_rel + 1;
                // Flush the text accumulated before this image.
                push_text_block(&mut blocks, &html[text_start..i]);
                // Fetch and decode the image; on any failure we just skip it.
                let tag = &html[i..tag_end];
                if let Some(src) = attr_value(tag, "src")
                    && let Ok(Some(bytes)) = anki.retrieve_media_file(&src)
                    && let Ok(img) = image::load_from_memory(&bytes)
                {
                    let (cols, rows) = natural_cells(&img, font);
                    blocks.push(Block::Image(images.len()));
                    images.push(CardImage {
                        protocol: picker.new_resize_protocol(img),
                        cols,
                        rows,
                    });
                }
                i = tag_end;
                text_start = i;
                continue;
            }
            let ch = html[i..].chars().next().unwrap();
            i += ch.len_utf8();
        }
        push_text_block(&mut blocks, &html[text_start..]);

        SideMedia {
            blocks,
            images,
            audio,
        }
    }

    /// Play every audio clip on this side (used on reveal and for the `r` key).
    pub fn play_audio(&self) {
        play_clips(&self.audio);
    }
}

/// Fetch playable audio clips for every `[sound:...]` token in `html`, returning
/// temp-file paths. Use this on raw note field values: the rendered
/// question/answer HTML from AnkiConnect has these tokens replaced by replay
/// buttons, so the fields are the only place the filenames survive.
pub fn audio_from_html(html: &str, anki: &AnkiConnect) -> Vec<PathBuf> {
    extract_audio(html)
        .into_iter()
        .filter_map(|name| match anki.retrieve_media_file(&name) {
            Ok(Some(bytes)) => write_temp(&name, &bytes).ok(),
            _ => None,
        })
        .collect()
}

/// Play a set of audio clips in the background.
pub fn play_clips(clips: &[PathBuf]) {
    for path in clips {
        play_file(path);
    }
}

/// Strip media tokens from an HTML fragment and, if anything renderable is left,
/// append it as a [`Block::Text`].
fn push_text_block(blocks: &mut Vec<Block>, html: &str) {
    let cleaned = strip_media_tokens(html);
    if !cleaned.trim().is_empty() {
        blocks.push(Block::Text(cleaned));
    }
}

/// An image's natural size in terminal cells, given the font's cell pixel size.
fn natural_cells(img: &image::DynamicImage, font: FontSize) -> (u16, u16) {
    let (w, h) = img.dimensions();
    let fw = font.width.max(1) as u32;
    let fh = font.height.max(1) as u32;
    let cols = w.div_ceil(fw).clamp(1, u16::MAX as u32) as u16;
    let rows = h.div_ceil(fh).clamp(1, u16::MAX as u32) as u16;
    (cols, rows)
}

/// Render an HTML fragment to styled terminal text, wrapped to `width` columns.
/// Uses html2text's rich renderer so inline markup (`<b>`, `<em>`, `<code>`, …)
/// becomes ratatui styling rather than literal `**markers**`; it also handles
/// entities, lists, tables, and drops `<style>`/`<script>` blocks.
pub fn render_html(html: &str, width: u16) -> Text<'static> {
    let width = width.max(10) as usize;
    // html2text drops `<hr>` entirely, so split on it ourselves and draw a
    // rule between the segments (this is Anki's question/answer separator).
    let mut out: Vec<Line> = Vec::new();
    for segment in split_on_hr(html) {
        let seg_lines = render_segment(&segment, width);
        if seg_lines.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Line::from(Span::styled(
                "─".repeat(width),
                Style::default().fg(Color::DarkGray),
            )));
        }
        out.extend(seg_lines);
    }
    Text::from(out)
}

/// Render one HTML fragment (no `<hr>`) to styled lines via html2text's rich
/// renderer. On failure, fall back to the raw text as a single line.
fn render_segment(html: &str, width: usize) -> Vec<Line<'static>> {
    let Ok(lines) = html2text::config::rich().lines_from_read(html.as_bytes(), width) else {
        return vec![Line::from(html.to_string())];
    };
    lines
        .into_iter()
        .map(|tline| {
            let spans: Vec<Span> = tline
                .into_iter()
                .filter_map(|el| match el {
                    TaggedLineElement::Str(ts) => {
                        Some(Span::styled(ts.s, annotations_to_style(&ts.tag)))
                    }
                    // Fragment anchors carry no visible text.
                    _ => None,
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

/// Split HTML on `<hr …>` tags, which html2text discards. Returns the text
/// between (and around) the rules so the caller can draw a separator.
fn split_on_hr(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < html.len() {
        // Match `<hr` only at a tag boundary (next char ends the name).
        if lower[i..].starts_with("<hr") {
            let boundary = lower[i + 3..]
                .chars()
                .next()
                .is_none_or(|c| c == '>' || c == '/' || c.is_whitespace());
            if boundary && let Some(end) = html[i..].find('>') {
                out.push(html[start..i].to_string());
                i += end + 1;
                start = i;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
    out.push(html[start..].to_string());
    out
}

/// Map html2text's rich annotations to a ratatui style. Annotations nest
/// (e.g. bold inside a link), so accumulate every modifier in the span's tags.
/// CSS colours from Anki's card styling are intentionally ignored so text keeps
/// the terminal's own foreground instead of fighting the theme.
fn annotations_to_style(tags: &[RichAnnotation]) -> Style {
    let mut style = Style::default();
    for tag in tags {
        style = match tag {
            RichAnnotation::Strong => style.add_modifier(Modifier::BOLD),
            RichAnnotation::Emphasis => style.add_modifier(Modifier::ITALIC),
            RichAnnotation::Strikeout => style.add_modifier(Modifier::CROSSED_OUT),
            RichAnnotation::Code => style.add_modifier(Modifier::DIM),
            RichAnnotation::Link(_) => style.add_modifier(Modifier::UNDERLINED),
            _ => style,
        };
    }
    style
}

/// Pull filenames out of `[sound:filename]` tokens.
fn extract_audio(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("[sound:") {
        let after = &rest[start + "[sound:".len()..];
        if let Some(end) = after.find(']') {
            out.push(after[..end].to_string());
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    out
}

/// Read a quoted attribute value (e.g. `src="foo.jpg"`) from a tag substring.
fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{attr}=");
    let idx = lower.find(&key)? + key.len();
    let rest = &tag[idx..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let rest = &rest[1..];
        let end = rest.find(quote)?;
        Some(rest[..end].to_string())
    } else {
        // Unquoted: read up to whitespace or tag end.
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Remove media tokens that html2text would otherwise render as stray text:
/// `[sound:…]` references and `<img>` tags (we render images separately).
fn strip_media_tokens(html: &str) -> String {
    // Remove [sound:…] tokens.
    let mut s = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("[sound:") {
        s.push_str(&rest[..start]);
        match rest[start..].find(']') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    s.push_str(rest);

    // Remove <img …> tags.
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with("<img") {
            match s[i..].find('>') {
                Some(end) => {
                    i += end + 1;
                    continue;
                }
                None => break,
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Temp directory used to stage audio files.
fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("anki-tui");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write media bytes to a temp file, returning its path.
fn write_temp(name: &str, bytes: &[u8]) -> Result<PathBuf> {
    // Use just the file name component to avoid path traversal from media names.
    let safe = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let path = temp_dir().join(safe);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Play an audio file in the background using the configured player.
fn play_file(path: &PathBuf) {
    let cmd = std::env::var("ANKI_TUI_AUDIO_CMD").unwrap_or_else(|_| "afplay".to_string());
    // Fire and forget; ignore failures so playback never blocks reviewing.
    let _ = Command::new(cmd)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sound_filenames() {
        let html = "Word [sound:hello.mp3] more [sound:bye.ogg]";
        assert_eq!(extract_audio(html), vec!["hello.mp3", "bye.ogg"]);
    }

    #[test]
    fn strips_media_tokens() {
        let html = "Hello [sound:x.mp3] <img src=\"a.jpg\"> world";
        let cleaned = strip_media_tokens(html);
        assert!(!cleaned.contains("[sound:"));
        assert!(!cleaned.contains("<img"));
        assert!(cleaned.contains("Hello"));
        assert!(cleaned.contains("world"));
    }

    #[test]
    fn to_text_renders_html_and_drops_style() {
        let html =
            strip_media_tokens("<style>.card { color: red; }</style><p>Hello&nbsp;world</p>");
        // Flatten the styled lines back to a string to assert on content.
        let text: String = render_html(&html, 40)
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        // &nbsp; decodes to U+00A0, so check the words individually.
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("color"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn dbg_hr() {
        let html = "Acronym: <b>OSCI</b><hr id=answer/>Online Services Computer Interface";
        for line in html2text::config::rich()
            .lines_from_read(html.as_bytes(), 60)
            .unwrap()
        {
            let mut s = String::new();
            for el in line {
                if let html2text::render::TaggedLineElement::Str(ts) = el {
                    s.push_str(&ts.s);
                }
            }
            eprintln!("LINE {s:?}");
        }
    }

    #[test]
    fn to_text_renders_bold_as_modifier() {
        let text = render_html("Acronym: <b>VVT</b>", 40);
        // The "VVT" span must carry the BOLD modifier, and no literal `**`
        // markers should leak into the rendered text.
        let vvt = text
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content.contains("VVT"))
            .expect("VVT span present");
        assert!(vvt.style.add_modifier.contains(Modifier::BOLD));
        let joined: String = text
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(!joined.contains("**"));
    }
}
