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

/// Like `shorten`, but keeps newlines so multi-line markdown survives.
pub(crate) fn shorten_block(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        if ch == '\n' {
            out.push(ch);
        } else if ch == '\r' {
            // Dropped; `\r\n` becomes `\n`.
        } else if ch.is_control() {
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

/// Block kinds produced by `markdown_blocks` for styled rendering in Slint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownBlockKind {
    Paragraph,
    /// Level clamped to 1..=3.
    Heading(u8),
    Bullet,
    Code,
    Quote,
    /// Unified-diff body (``` fenced with the `diff` language). Each line is
    /// tinted by its leading `+`/`-`/space marker when rendered.
    Diff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownBlock {
    pub(crate) kind: MarkdownBlockKind,
    pub(crate) text: String,
    /// Nesting depth for list items (2 spaces per level, capped).
    pub(crate) indent: u8,
}

/// Parse markdown into display blocks: headings, bullets, fenced code,
/// quotes, and plain paragraphs. Inline markup is stripped; code blocks
/// keep their text verbatim with one block per fence.
pub(crate) fn markdown_blocks(input: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    // Tracks an open fenced block: (collected lines, is the language `diff`).
    let mut code: Option<(Vec<String>, bool)> = None;
    for raw_line in input.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let trimmed = raw_line.trim();
        // Inside a fence: only an all-backtick line closes it, so `+`/`-`
        // prefixed diff content (and code containing fences) survives intact.
        if let Some((lines, is_diff)) = code.as_mut() {
            if is_fence_close(trimmed) {
                blocks.push(finish_code_block(std::mem::take(lines), *is_diff));
                code = None;
            } else {
                lines.push(raw_line.trim_end().to_string());
            }
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            code = Some((Vec::new(), info.trim().eq_ignore_ascii_case("diff")));
            continue;
        }
        if trimmed.is_empty() || is_horizontal_rule(trimmed) {
            continue;
        }

        let indent = line_indent(raw_line);
        if let Some((level, rest)) = heading_text(trimmed) {
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Heading(level),
                text: strip_inline_markdown(rest),
                indent: 0,
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Quote,
                text: strip_inline_markdown(rest.trim_start()),
                indent,
            });
            continue;
        }
        if let Some((prefix, rest)) = block_bullet_prefix(trimmed) {
            blocks.push(MarkdownBlock {
                kind: MarkdownBlockKind::Bullet,
                text: format!("{prefix}{}", strip_inline_markdown(rest)),
                indent,
            });
            continue;
        }
        blocks.push(MarkdownBlock {
            kind: MarkdownBlockKind::Paragraph,
            text: strip_inline_markdown(trimmed),
            indent,
        });
    }
    if let Some((lines, is_diff)) = code.take() {
        blocks.push(finish_code_block(lines, is_diff));
    }
    blocks
}

/// A fence is closed only by a line made entirely of backticks (```` ``` ````),
/// so opening info strings and prefixed diff content never close it by accident.
fn is_fence_close(trimmed: &str) -> bool {
    trimmed.len() >= 3 && trimmed.bytes().all(|b| b == b'`')
}

fn finish_code_block(lines: Vec<String>, is_diff: bool) -> MarkdownBlock {
    MarkdownBlock {
        kind: if is_diff {
            MarkdownBlockKind::Diff
        } else {
            MarkdownBlockKind::Code
        },
        text: lines.join("\n"),
        indent: 0,
    }
}

fn heading_text(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes.min(3) as u8, rest.trim()))
}

fn block_bullet_prefix(line: &str) -> Option<(String, &str)> {
    if let Some((prefix, rest)) = markdown_bullet_prefix(line) {
        let marker = match prefix {
            "[ ] " => "\u{2610} ",
            "[x] " => "\u{2611} ",
            _ => "\u{2022} ",
        };
        return Some((marker.to_string(), rest));
    }
    // Numbered list: "1. text" or "1) text".
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    let rest = rest.strip_prefix(' ')?;
    Some((format!("{}. ", &line[..digits]), rest))
}

fn is_horizontal_rule(line: &str) -> bool {
    line.len() >= 3
        && (line.bytes().all(|b| b == b'-')
            || line.bytes().all(|b| b == b'*')
            || line.bytes().all(|b| b == b'_'))
}

fn line_indent(raw_line: &str) -> u8 {
    let spaces: usize = raw_line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum();
    (spaces / 2).min(4) as u8
}

/// Estimate how many lines `text` occupies when word-wrapped into
/// `avail_px` at `font_px`. Slint's `Text` does not grow with wrapping,
/// so the UI sizes rows from this estimate; it leans slightly generous.
pub(crate) fn estimate_wrapped_lines(text: &str, font_px: f32, avail_px: f32, mono: bool) -> u32 {
    let avail_px = avail_px.max(font_px);
    text.split('\n')
        .map(|line| {
            let units: f32 = line.chars().map(|c| char_width_units(c, mono)).sum();
            let px = units * font_px * 1.06;
            (px / avail_px).ceil().max(1.0) as u32
        })
        .sum()
}

fn char_width_units(c: char, mono: bool) -> f32 {
    let wide = matches!(c,
        '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{20000}'..='\u{2FFFD}');
    if wide {
        return 1.0;
    }
    if mono {
        return 0.62;
    }
    match c {
        'i' | 'l' | 'j' | 'I' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' | '(' | ')' | '['
        | ']' | 'f' | 't' | 'r' | ' ' => 0.34,
        'm' | 'w' | 'M' | 'W' | '@' => 0.92,
        'A'..='Z' | '0'..='9' => 0.66,
        _ => 0.52,
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

/// Render a unified-diff body comparing `old` to `new`, line by line.
///
/// Unchanged leading/trailing lines are collapsed to a few lines of context
/// (with a `…` hunk marker when more were trimmed); changed runs are emitted as
/// `-` removed then `+` added lines, each capped so a huge edit stays readable.
/// Context lines keep a single-space gutter so a literal `+`/`-` in the source
/// is never mistaken for a change marker.
pub(crate) fn diff_lines_text(old: &str, new: &str) -> String {
    const CONTEXT: usize = 2;
    const MAX_CHANGED: usize = 60;

    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();

    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let removed = &old_lines[prefix..old_lines.len() - suffix];
    let added = &new_lines[prefix..new_lines.len() - suffix];

    let mut out: Vec<String> = Vec::new();
    let lead_start = prefix.saturating_sub(CONTEXT);
    if lead_start > 0 {
        out.push(" …".to_string());
    }
    for line in &old_lines[lead_start..prefix] {
        out.push(format!(" {line}"));
    }
    push_changed_lines(&mut out, removed, '-', MAX_CHANGED);
    push_changed_lines(&mut out, added, '+', MAX_CHANGED);
    let trail_start = old_lines.len() - suffix;
    let trail_end = (trail_start + CONTEXT).min(old_lines.len());
    for line in &old_lines[trail_start..trail_end] {
        out.push(format!(" {line}"));
    }
    if trail_end < old_lines.len() {
        out.push(" …".to_string());
    }
    out.join("\n")
}

fn push_changed_lines(out: &mut Vec<String>, lines: &[&str], sign: char, max: usize) {
    if lines.len() > max {
        for line in &lines[..max] {
            out.push(format!("{sign}{line}"));
        }
        out.push(format!("{sign}… {} more line(s)", lines.len() - max));
    } else {
        for line in lines {
            out.push(format!("{sign}{line}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_blocks_classifies_common_structures() {
        let text = "# Title\n\n## Step 1\nDo **this** first.\n- item one\n  - nested\n1. numbered\n- [x] done\n> note\n---\n```rs\nlet x = 1;\nlet y = 2;\n```";
        let blocks = markdown_blocks(text);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Heading(1));
        assert_eq!(blocks[0].text, "Title");
        assert_eq!(blocks[1].kind, MarkdownBlockKind::Heading(2));
        assert_eq!(blocks[2].kind, MarkdownBlockKind::Paragraph);
        assert_eq!(blocks[2].text, "Do this first.");
        assert_eq!(blocks[3].kind, MarkdownBlockKind::Bullet);
        assert_eq!(blocks[3].text, "\u{2022} item one");
        assert_eq!(blocks[3].indent, 0);
        assert_eq!(blocks[4].kind, MarkdownBlockKind::Bullet);
        assert_eq!(blocks[4].indent, 1);
        assert_eq!(blocks[5].text, "1. numbered");
        assert_eq!(blocks[6].text, "\u{2611} done");
        assert_eq!(blocks[7].kind, MarkdownBlockKind::Quote);
        assert_eq!(blocks[7].text, "note");
        // The horizontal rule is dropped; the fence becomes one code block.
        assert_eq!(blocks[8].kind, MarkdownBlockKind::Code);
        assert_eq!(blocks[8].text, "let x = 1;\nlet y = 2;");
        assert_eq!(blocks.len(), 9);
    }

    #[test]
    fn markdown_blocks_keeps_unclosed_fence() {
        let blocks = markdown_blocks("```\ncargo test");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Code);
        assert_eq!(blocks[0].text, "cargo test");
    }

    #[test]
    fn markdown_blocks_reads_diff_language_and_protects_content() {
        // A `-` prefixed line that contains a fence must not close the block.
        let blocks = markdown_blocks("path.rs\n```diff\n-let a = `1`;\n+let a = 2;\n```");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, MarkdownBlockKind::Paragraph);
        assert_eq!(blocks[1].kind, MarkdownBlockKind::Diff);
        assert_eq!(blocks[1].text, "-let a = `1`;\n+let a = 2;");
    }

    #[test]
    fn diff_lines_text_trims_common_context_and_marks_changes() {
        let old = "a\nb\nc\nd\ne";
        let new = "a\nb\nX\nd\ne";
        let diff = diff_lines_text(old, new);
        assert_eq!(diff, " a\n b\n-c\n+X\n d\n e");
    }

    #[test]
    fn diff_lines_text_caps_huge_changes() {
        let old = String::new();
        let new = (0..200)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let diff = diff_lines_text(&old, &new);
        assert!(diff.lines().any(|line| line.contains("more line(s)")));
        // 60 added lines + the truncation marker.
        assert_eq!(diff.lines().filter(|l| l.starts_with('+')).count(), 61);
    }

    #[test]
    fn estimate_wrapped_lines_grows_with_text() {
        assert_eq!(estimate_wrapped_lines("short", 13.0, 500.0, false), 1);
        let long = "word ".repeat(60);
        assert!(estimate_wrapped_lines(&long, 13.0, 500.0, false) >= 3);
        // CJK counts as full-width.
        let cjk = "\u{4e2d}".repeat(80);
        assert!(estimate_wrapped_lines(&cjk, 13.0, 500.0, false) >= 2);
        // Explicit newlines always count.
        assert_eq!(estimate_wrapped_lines("a\nb\nc", 13.0, 500.0, true), 3);
    }

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
