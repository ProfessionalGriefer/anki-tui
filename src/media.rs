//! Extract images and audio from card HTML, build image protocols, play audio.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use html2text::render::RichAnnotation;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::anki::AnkiConnect;

/// Everything renderable/playable for one side of a card.
pub struct SideMedia {
    /// Card HTML with media tokens removed, rendered to text on demand.
    html: String,
    /// Decoded, terminal-ready image protocols (re-encoded on resize).
    pub images: Vec<StatefulProtocol>,
    /// Paths to audio files written to a temp dir, ready for the player.
    pub audio: Vec<PathBuf>,
}

impl SideMedia {
    /// Build the media for one HTML fragment, fetching referenced files from Anki.
    pub fn build(html: &str, anki: &AnkiConnect, picker: &Picker) -> Self {
        let image_files = extract_images(html);
        let audio_files = extract_audio(html);
        // Drop media tokens so html2text doesn't emit `[sound:…]` / image alt text.
        let cleaned = strip_media_tokens(html);

        let mut images = Vec::new();
        for name in image_files {
            if let Ok(Some(bytes)) = anki.retrieve_media_file(&name)
                && let Ok(img) = image::load_from_memory(&bytes)
            {
                images.push(picker.new_resize_protocol(img));
            }
        }

        let mut audio = Vec::new();
        for name in audio_files {
            if let Ok(Some(bytes)) = anki.retrieve_media_file(&name)
                && let Ok(path) = write_temp(&name, &bytes)
            {
                audio.push(path);
            }
        }

        SideMedia {
            html: cleaned,
            images,
            audio,
        }
    }

    /// Render the card text to styled terminal text, wrapped to `width` columns.
    /// Uses html2text's rich renderer so inline markup (`<b>`, `<em>`, `<code>`,
    /// …) becomes ratatui styling rather than literal `**markers**`; it also
    /// handles entities, lists, tables, and drops `<style>`/`<script>` blocks.
    pub fn to_text(&self, width: u16) -> Text<'static> {
        let width = width.max(10) as usize;
        let Ok(lines) = html2text::config::rich().lines_from_read(self.html.as_bytes(), width)
        else {
            return Text::raw(self.html.clone());
        };

        let rendered: Vec<Line> = lines
            .into_iter()
            .map(|tline| {
                let spans: Vec<Span> = tline
                    .into_iter()
                    .filter_map(|el| match el {
                        html2text::render::TaggedLineElement::Str(ts) => {
                            Some(Span::styled(ts.s, annotations_to_style(&ts.tag)))
                        }
                        // Fragment anchors carry no visible text.
                        _ => None,
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();
        Text::from(rendered)
    }

    /// Play every audio clip on this side (used on reveal and for the `r` key).
    pub fn play_audio(&self) {
        for path in &self.audio {
            play_file(path);
        }
    }
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

/// Pull `src="..."` filenames out of `<img>` tags.
fn extract_images(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("<img") {
        let tag_start = search_from + rel;
        // Find the end of this tag.
        let tag_end = lower[tag_start..]
            .find('>')
            .map(|e| tag_start + e)
            .unwrap_or(html.len());
        let tag = &html[tag_start..tag_end];
        if let Some(src) = attr_value(tag, "src") {
            out.push(src);
        }
        search_from = tag_end.max(tag_start + 1);
    }
    out
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
    fn extracts_image_filenames() {
        let html = r#"<div>Front <img src="cat.jpg"> and <img src='dog.png' alt="d"></div>"#;
        assert_eq!(extract_images(html), vec!["cat.jpg", "dog.png"]);
    }

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
        let side = SideMedia {
            html: strip_media_tokens(
                "<style>.card { color: red; }</style><p>Hello&nbsp;world</p>",
            ),
            images: Vec::new(),
            audio: Vec::new(),
        };
        // Flatten the styled lines back to a string to assert on content.
        let text: String = side
            .to_text(40)
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
    fn to_text_renders_bold_as_modifier() {
        let side = SideMedia {
            html: "Acronym: <b>VVT</b>".to_string(),
            images: Vec::new(),
            audio: Vec::new(),
        };
        let text = side.to_text(40);
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
