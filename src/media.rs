//! Extract images and audio from card HTML, build image protocols, play audio.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::anki::AnkiConnect;

/// Everything renderable/playable for one side of a card.
pub struct SideMedia {
    /// Plain text with HTML tags and media tokens stripped out.
    pub text: String,
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
        let text = strip_html(html);

        let mut images = Vec::new();
        for name in image_files {
            if let Ok(Some(bytes)) = anki.retrieve_media_file(&name)
                && let Ok(img) = image::load_from_memory(&bytes) {
                    images.push(picker.new_resize_protocol(img));
                }
        }

        let mut audio = Vec::new();
        for name in audio_files {
            if let Ok(Some(bytes)) = anki.retrieve_media_file(&name)
                && let Ok(path) = write_temp(&name, &bytes) {
                    audio.push(path);
                }
        }

        SideMedia {
            text,
            images,
            audio,
        }
    }

    /// Play every audio clip on this side (used on reveal and for the `r` key).
    pub fn play_audio(&self) {
        for path in &self.audio {
            play_file(path);
        }
    }
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

/// Strip HTML tags and media tokens, normalize whitespace, decode basic entities.
fn strip_html(html: &str) -> String {
    // Drop sound tokens first.
    let mut s = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("[sound:") {
        s.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find(']') {
            rest = &rest[start + end + 1..];
        } else {
            rest = "";
            break;
        }
    }
    s.push_str(rest);

    // Convert block-ish tags to newlines, then remove all tags.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                in_tag = true;
                // Peek tag name to insert a newline for common block elements.
                let mut name = String::new();
                while let Some(&p) = chars.peek() {
                    if p.is_ascii_alphabetic() || p == '/' {
                        name.push(p.to_ascii_lowercase());
                        chars.next();
                    } else {
                        break;
                    }
                }
                if matches!(
                    name.as_str(),
                    "br" | "/br" | "p" | "/p" | "div" | "/div" | "hr" | "tr" | "/tr" | "li"
                )
                    && !out.ends_with('\n') {
                        out.push('\n');
                    }
            }
            '>' => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }

    let decoded = decode_entities(&out);
    // Collapse runs of blank lines and trim each line's trailing spaces.
    let mut lines: Vec<String> = decoded
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Decode the handful of HTML entities common in Anki cards.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        if let Some(semi) = after.find(';').filter(|&i| i <= 8) {
            let entity = &after[1..semi];
            let replacement = match entity {
                "nbsp" => Some(" ".to_string()),
                "amp" => Some("&".to_string()),
                "lt" => Some("<".to_string()),
                "gt" => Some(">".to_string()),
                "quot" => Some("\"".to_string()),
                "apos" | "#39" => Some("'".to_string()),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string())
                }
                _ if entity.starts_with('#') => entity[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string()),
                _ => None,
            };
            if let Some(r) = replacement {
                out.push_str(&r);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
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
    fn strips_tags_and_sound_tokens() {
        let html = "<p>Hello&nbsp;world</p><br>line2 [sound:x.mp3]<img src=\"a.jpg\">";
        let text = strip_html(html);
        assert!(text.contains("Hello world"));
        assert!(text.contains("line2"));
        assert!(!text.contains("[sound:"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &#65;"), "a & b <c> A");
        // Unknown entities are left intact.
        assert_eq!(decode_entities("100% &bogus; ok"), "100% &bogus; ok");
    }
}
