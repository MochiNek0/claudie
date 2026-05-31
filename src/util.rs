use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn parse_port(args: &[String]) -> Option<u16> {
    args.windows(2)
        .find(|pair| pair[0] == "--port")
        .and_then(|pair| pair[1].parse::<u16>().ok())
}

pub(crate) fn shorten(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn compact_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| shorten(path, 28))
}

#[derive(Clone)]
pub(crate) struct ConnectionLimiter {
    active: Arc<AtomicUsize>,
    max: usize,
}

pub(crate) struct ConnectionPermit {
    active: Arc<AtomicUsize>,
}

impl ConnectionLimiter {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            max,
        }
    }

    pub(crate) fn try_acquire(&self) -> Option<ConnectionPermit> {
        let mut current = self.active.load(Ordering::Relaxed);
        loop {
            if current >= self.max {
                return None;
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(ConnectionPermit {
                        active: self.active.clone(),
                    });
                }
                Err(next) => current = next,
            }
        }
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Release);
    }
}

pub(crate) fn markdown_to_display_text(input: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for raw_line in input.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }

        if in_code {
            push_markdown_line(&mut out, &format!("    {}", raw_line.trim_end()));
            continue;
        }

        let mut line = trimmed;
        line = line.trim_start_matches('#').trim_start();
        line = line.strip_prefix('>').map(str::trim_start).unwrap_or(line);

        let bullet = markdown_bullet_prefix(line);
        if let Some((prefix, rest)) = bullet {
            push_markdown_line(
                &mut out,
                &format!("{prefix}{}", strip_inline_markdown(rest)),
            );
        } else {
            push_markdown_line(&mut out, &strip_inline_markdown(line));
        }
    }
    out.trim().to_string()
}

fn push_markdown_line(out: &mut String, line: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
}

fn markdown_bullet_prefix(line: &str) -> Option<(&'static str, &str)> {
    for marker in ["- [ ] ", "* [ ] ", "+ [ ] "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(("[ ] ", rest));
        }
    }
    for marker in ["- [x] ", "- [X] ", "* [x] ", "* [X] ", "+ [x] ", "+ [X] "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(("[x] ", rest));
        }
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(("- ", rest));
        }
    }
    None
}

fn strip_inline_markdown(input: &str) -> String {
    let linked = strip_markdown_links(input);
    linked
        .chars()
        .filter(|ch| !matches!(ch, '`' | '*' | '_'))
        .collect()
}

fn strip_markdown_links(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(label_start) = rest.find('[') {
        let before = &rest[..label_start];
        let after_label_start = &rest[label_start + 1..];
        let Some(label_end) = after_label_start.find(']') else {
            break;
        };
        let after_label = &after_label_start[label_end + 1..];
        let Some(after_open) = after_label.strip_prefix('(') else {
            out.push_str(before);
            out.push('[');
            rest = after_label_start;
            continue;
        };
        let Some(url_end) = after_open.find(')') else {
            break;
        };
        out.push_str(before);
        out.push_str(&after_label_start[..label_end]);
        rest = &after_open[url_end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_to_display_text_removes_common_markup() {
        let text =
            "# Title\n- **Allow** `Read` [docs](https://example.com)\n```sh\ncargo test\n```";
        assert_eq!(
            markdown_to_display_text(text),
            "Title\n- Allow Read docs\n    cargo test"
        );
    }
}
