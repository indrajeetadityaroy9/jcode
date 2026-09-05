use crate::{message::ToolCall, tui::ui::tools_ui};
use ratatui::prelude::*;

pub(super) fn diff_add_color() -> Color {
    Color::Rgb(100, 200, 100)
}

pub(super) fn diff_del_color() -> Color {
    Color::Rgb(200, 100, 100)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiffLineKind {
    Add,
    Del,
}

#[derive(Clone, Debug)]
pub(super) struct ParsedDiffLine {
    pub kind: DiffLineKind,
    pub prefix: String,
    pub content: String,
    /// Byte ranges into `content` that actually changed, for word-level
    /// emphasis. Empty means "emphasise the whole line", which is the honest
    /// answer whenever the ranges are unknown: lines parsed out of unified
    /// patch text arrive already split into `-`/`+`, with no pairing to diff.
    pub emphasis: Vec<(usize, usize)>,
}

pub(super) fn diff_change_counts(content: &str) -> (usize, usize) {
    let lines = collect_diff_lines(content);
    let additions = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Add)
        .count();
    let deletions = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Del)
        .count();
    (additions, deletions)
}

pub(super) fn diff_change_counts_for_tool(tool: &ToolCall, content: &str) -> (usize, usize) {
    let (additions, deletions) = diff_change_counts(content);
    if additions > 0 || deletions > 0 {
        return (additions, deletions);
    }

    match tools_ui::canonical_tool_name(&tool.name) {
        "edit" => {
            diff_counts_from_input_pair(&tool.input, "old_string", "new_string").unwrap_or((0, 0))
        }
        "write" => {
            let content = tool
                .input
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            diff_counts_from_strings("", content)
        }
        "multiedit" => diff_counts_from_multiedit(&tool.input).unwrap_or((0, 0)),
        "patch" => diff_counts_from_unified_patch_input(&tool.input).unwrap_or((0, 0)),
        "apply_patch" => diff_counts_from_apply_patch_input(&tool.input).unwrap_or((0, 0)),
        _ => (additions, deletions),
    }
}

fn diff_counts_from_input_pair(
    input: &serde_json::Value,
    old_key: &str,
    new_key: &str,
) -> Option<(usize, usize)> {
    let old = input.get(old_key)?.as_str()?;
    let new = input.get(new_key)?.as_str()?;
    Some(diff_counts_from_strings(old, new))
}

fn diff_counts_from_multiedit(input: &serde_json::Value) -> Option<(usize, usize)> {
    let edits = input.get("edits")?.as_array()?;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for edit in edits {
        let old = edit
            .get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new = edit
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if old.is_empty() && new.is_empty() {
            continue;
        }
        let (add, del) = diff_counts_from_strings(old, new);
        additions += add;
        deletions += del;
    }

    Some((additions, deletions))
}

fn diff_counts_from_unified_patch_input(input: &serde_json::Value) -> Option<(usize, usize)> {
    let patch_text = input.get("patch_text")?.as_str()?;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for line in patch_text.lines() {
        if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("@@")
            || line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("\\ No newline")
        {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }

    Some((additions, deletions))
}

fn diff_counts_from_apply_patch_input(input: &serde_json::Value) -> Option<(usize, usize)> {
    let patch_text = input.get("patch_text")?.as_str()?;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for line in patch_text.lines() {
        if line.starts_with("***") || line.starts_with("@@") {
            continue;
        }

        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }

    Some((additions, deletions))
}

fn diff_counts_from_strings(old: &str, new: &str) -> (usize, usize) {
    use similar::ChangeTag;

    let diff = similar::TextDiff::from_lines(old, new);
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    (additions, deletions)
}

pub(super) fn generate_diff_lines_from_tool_input(tool: &ToolCall) -> Vec<ParsedDiffLine> {
    match tools_ui::canonical_tool_name(&tool.name) {
        "edit" => {
            let old = tool
                .input
                .get("old_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new = tool
                .input
                .get("new_string")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            generate_diff_lines_from_strings(old, new)
        }
        "multiedit" => {
            let Some(edits) = tool.input.get("edits").and_then(|v| v.as_array()) else {
                return Vec::new();
            };
            let mut all_lines = Vec::new();
            for edit in edits {
                let old = edit
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new = edit
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                all_lines.extend(generate_diff_lines_from_strings(old, new));
            }
            all_lines
        }
        "write" => {
            let content = tool
                .input
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            generate_diff_lines_from_strings("", content)
        }
        "patch" => {
            let patch_text = tool
                .input
                .get("patch_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            collect_diff_lines(patch_text)
        }
        "apply_patch" => {
            let patch_text = tool
                .input
                .get("patch_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            collect_diff_lines(patch_text)
        }
        _ => Vec::new(),
    }
}

/// Build renderable diff lines, with word-level emphasis inside each changed
/// line.
///
/// `iter_inline_changes` runs a second, character-level diff within each
/// changed block, so `let x = 1;` -> `let x = 2;` marks only `2` instead of
/// presenting two whole lines as unrelated. It needs the op-by-op form of the
/// diff, hence the `diff.ops()` loop rather than `iter_all_changes`.
fn generate_diff_lines_from_strings(old: &str, new: &str) -> Vec<ParsedDiffLine> {
    use similar::ChangeTag;

    let diff = similar::TextDiff::from_lines(old, new);
    let mut lines = Vec::new();

    for op in diff.ops() {
        for change in diff.iter_inline_changes(op) {
            let (kind, prefix) = match change.tag() {
                ChangeTag::Delete => (
                    DiffLineKind::Del,
                    format!("{}- ", change.old_index().unwrap_or(0) + 1),
                ),
                ChangeTag::Insert => (
                    DiffLineKind::Add,
                    format!("{}+ ", change.new_index().unwrap_or(0) + 1),
                ),
                ChangeTag::Equal => continue,
            };

            let (raw, raw_emphasis) = flatten_inline_change(&change);
            let content = raw.trim();
            if content.is_empty() {
                continue;
            }
            let emphasis = shift_ranges(
                &raw_emphasis,
                raw.len() - raw.trim_start().len(),
                content.len(),
            );

            lines.push(ParsedDiffLine {
                kind,
                prefix,
                content: content.to_string(),
                emphasis,
            });
        }
    }

    lines
}

/// Concatenate an inline change's segments, recording where the emphasised
/// ones land in the rebuilt string.
fn flatten_inline_change<'a>(
    change: &similar::InlineChange<'a, str>,
) -> (String, Vec<(usize, usize)>) {
    let mut raw = String::new();
    let mut ranges = Vec::new();
    for (emphasized, value) in change.iter_strings_lossy() {
        let start = raw.len();
        raw.push_str(value.as_ref());
        if emphasized && raw.len() > start {
            ranges.push((start, raw.len()));
        }
    }
    (raw, ranges)
}

/// Rebase ranges after leading whitespace was trimmed, dropping anything that
/// falls outside the kept text.
fn shift_ranges(ranges: &[(usize, usize)], offset: usize, len: usize) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .filter_map(|(start, end)| {
            let start = start.saturating_sub(offset).min(len);
            let end = end.saturating_sub(offset).min(len);
            (start < end).then_some((start, end))
        })
        .collect()
}

/// Word-level change ranges for one replaced line, as `(deleted, inserted)`.
///
/// The file-diff pane pairs a removed line with its replacement itself, so it
/// has the two sides in hand and only needs the intra-line ranges. Text is
/// used verbatim — these rows are not trimmed — so the ranges index straight
/// into each row's `text`.
pub(super) fn word_emphasis(old_line: &str, new_line: &str) -> WordEmphasis {
    use similar::ChangeTag;

    let diff = similar::TextDiff::from_lines(old_line, new_line);
    let mut out = WordEmphasis::default();
    for op in diff.ops() {
        for change in diff.iter_inline_changes(op) {
            let (raw, ranges) = flatten_inline_change(&change);
            let trimmed_end = raw.trim_end_matches(['\n', '\r']).len();
            let ranges = shift_ranges(&ranges, 0, trimmed_end);
            match change.tag() {
                ChangeTag::Delete => out.deleted.extend(ranges),
                ChangeTag::Insert => out.inserted.extend(ranges),
                ChangeTag::Equal => {}
            }
        }
    }
    out
}

/// Intra-line change ranges for a replaced line pair.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct WordEmphasis {
    pub deleted: Vec<(usize, usize)>,
    pub inserted: Vec<(usize, usize)>,
}

pub(super) fn collect_diff_lines(content: &str) -> Vec<ParsedDiffLine> {
    content.lines().filter_map(parse_diff_line).collect()
}

fn parse_diff_line(raw_line: &str) -> Option<ParsedDiffLine> {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() || trimmed == "..." {
        return None;
    }
    if trimmed.starts_with("diff --git ")
        || trimmed.starts_with("index ")
        || trimmed.starts_with("--- ")
        || trimmed.starts_with("+++ ")
        || trimmed.starts_with("@@ ")
        || trimmed.starts_with("\\ No newline")
    {
        return None;
    }

    if let Some(pos) = trimmed.find("- ") {
        let (prefix, content) = trimmed.split_at(pos + 2);
        if !prefix.is_empty() && prefix[..pos].chars().all(|c| c.is_ascii_digit()) {
            return Some(ParsedDiffLine {
                kind: DiffLineKind::Del,
                prefix: prefix.to_string(),
                content: trim_diff_content(content),
                emphasis: Vec::new(),
            });
        }
    }
    if let Some(pos) = trimmed.find("+ ") {
        let (prefix, content) = trimmed.split_at(pos + 2);
        if !prefix.is_empty() && prefix[..pos].chars().all(|c| c.is_ascii_digit()) {
            return Some(ParsedDiffLine {
                kind: DiffLineKind::Add,
                prefix: prefix.to_string(),
                content: trim_diff_content(content),
                emphasis: Vec::new(),
            });
        }
    }

    if let Some(rest) = raw_line.strip_prefix('+') {
        return Some(ParsedDiffLine {
            kind: DiffLineKind::Add,
            prefix: "+".to_string(),
            content: trim_diff_content(rest),
            emphasis: Vec::new(),
        });
    }
    if let Some(rest) = raw_line.strip_prefix('-') {
        return Some(ParsedDiffLine {
            kind: DiffLineKind::Del,
            prefix: "-".to_string(),
            content: trim_diff_content(rest),
            emphasis: Vec::new(),
        });
    }

    None
}

fn trim_diff_content(content: &str) -> String {
    content.trim_start_matches([' ', '\t']).to_string()
}

pub(super) fn tint_span_with_diff_color(span: Span<'static>, diff_color: Color) -> Span<'static> {
    let (dr, dg, db) = match diff_color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(n) => super::color_support::indexed_to_rgb(n),
        _ => return span,
    };

    let fg = span.style.fg.unwrap_or(Color::White);
    let (sr, sg, sb) = match fg {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(n) => super::color_support::indexed_to_rgb(n),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        _ => return span,
    };

    let blend = |s: u8, d: u8| -> u8 { ((s as u16 * 70 + d as u16 * 30) / 100) as u8 };

    let tinted = Color::Rgb(blend(sr, dr), blend(sg, dg), blend(sb, db));
    Span::styled(span.content, span.style.fg(tinted))
}

/// Apply word-level emphasis to already syntax-highlighted, diff-tinted spans.
///
/// `emphasis` holds byte ranges into the line's full content; `rendered_bytes`
/// is how much of that content these spans actually cover, which is less than
/// the whole line when the renderer truncated it to the pane width.
///
/// An empty `emphasis` returns the spans untouched: that is the case for a
/// wholly new or wholly removed line, where the line's own colour already says
/// everything and underlining all of it would be noise.
pub(super) fn emphasize_diff_spans(
    spans: Vec<Span<'static>>,
    emphasis: &[(usize, usize)],
    rendered_bytes: usize,
) -> Vec<Span<'static>> {
    if emphasis.is_empty() {
        return spans;
    }

    let accent = Modifier::BOLD | Modifier::UNDERLINED;
    let mut out = Vec::with_capacity(spans.len());
    let mut cursor = 0usize;

    for span in spans {
        let text = span.content.into_owned();
        let style = span.style;
        let span_end = cursor + text.len();
        if cursor >= rendered_bytes {
            out.push(Span::styled(text, style));
            cursor = span_end;
            continue;
        }

        // Cut this span wherever an emphasised range starts or ends inside it,
        // so highlighting survives syntax colouring instead of replacing it.
        let mut cuts: Vec<usize> = vec![0, text.len()];
        for (start, end) in emphasis {
            for edge in [*start, *end] {
                if edge > cursor && edge < span_end && text.is_char_boundary(edge - cursor) {
                    cuts.push(edge - cursor);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();

        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if from == to {
                continue;
            }
            let absolute = cursor + from;
            let emphasised = emphasis
                .iter()
                .any(|(start, end)| absolute >= *start && absolute < *end);
            let piece = text[from..to].to_string();
            out.push(if emphasised {
                Span::styled(piece, style.add_modifier(accent))
            } else {
                Span::styled(piece, style)
            });
        }

        cursor = span_end;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{
        DiffLineKind, collect_diff_lines, diff_change_counts_for_tool,
        diff_counts_from_apply_patch_input, emphasize_diff_spans, generate_diff_lines_from_strings,
        word_emphasis,
    };
    use crate::message::ToolCall;
    use ratatui::prelude::*;
    use serde_json::json;

    #[test]
    fn apply_patch_counts_ignore_context_lines_with_plus_or_minus_prefixes() {
        let input = json!({
            "patch_text": "*** Begin Patch\n*** Update File: demo.txt\n@@\n  +context line\n  -context line\n+added line\n-deleted line\n*** End Patch\n"
        });

        assert_eq!(diff_counts_from_apply_patch_input(&input), Some((1, 1)));
    }

    #[test]
    fn write_tool_falls_back_to_content_diff_counts() {
        let tool = ToolCall {
            id: "tool_1".to_string(),
            name: "write".to_string(),
            input: json!({
                "file_path": "demo.txt",
                "content": "first line\nsecond line\n"
            }),
            intent: None,
            thought_signature: None,
        };

        assert_eq!(diff_change_counts_for_tool(&tool, ""), (2, 0));
    }

    #[test]
    fn multiedit_pascal_case_falls_back_to_input_diff_counts() {
        let tool = ToolCall {
            id: "tool_2".to_string(),
            name: "MultiEdit".to_string(),
            input: json!({
                "file_path": "demo.txt",
                "edits": [
                    {"old_string": "two\n", "new_string": "TWO\n"},
                    {"old_string": "three\n", "new_string": "THREE\n"}
                ]
            }),
            intent: None,
            thought_signature: None,
        };

        assert_eq!(diff_change_counts_for_tool(&tool, ""), (2, 2));
    }

    #[test]
    fn generated_diff_lines_use_old_and_new_line_numbers() {
        let lines =
            generate_diff_lines_from_strings("one\ntwo\nthree\n", "one\nthree\nfour\nfive\n");

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, DiffLineKind::Del);
        assert_eq!(lines[0].prefix, "2- ");
        assert_eq!(lines[1].kind, DiffLineKind::Add);
        assert_eq!(lines[1].prefix, "3+ ");
        assert_eq!(lines[2].kind, DiffLineKind::Add);
        assert_eq!(lines[2].prefix, "4+ ");
    }

    /// The substrings a line's emphasis ranges actually cover.
    fn emphasised(content: &str, ranges: &[(usize, usize)]) -> Vec<String> {
        ranges
            .iter()
            .map(|(start, end)| content[*start..*end].to_string())
            .collect()
    }

    #[test]
    fn a_one_token_change_emphasises_only_that_token() {
        let lines = generate_diff_lines_from_strings("let x = 1;\n", "let x = 2;\n");

        assert_eq!(
            lines.len(),
            2,
            "one replaced line renders as a del and an add"
        );
        let del = &lines[0];
        let add = &lines[1];
        assert_eq!(emphasised(&del.content, &del.emphasis), vec!["1;"]);
        assert_eq!(emphasised(&add.content, &add.emphasis), vec!["2;"]);
    }

    #[test]
    fn a_wholly_new_line_carries_no_emphasis() {
        // Nothing to contrast against, so the line's colour is the whole story
        // and underlining every character would be noise.
        let lines = generate_diff_lines_from_strings("keep();\n", "keep();\nadded();\n");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, DiffLineKind::Add);
        assert!(
            lines[0].emphasis.is_empty(),
            "unpaired insert must not be word-emphasised: {:?}",
            lines[0].emphasis
        );
    }

    #[test]
    fn emphasis_ranges_survive_the_leading_whitespace_trim() {
        // `content` is stored trimmed, so ranges computed on the raw line must
        // be rebased or they point at the wrong characters.
        let lines =
            generate_diff_lines_from_strings("        value = old;\n", "        value = new;\n");

        for line in &lines {
            assert!(
                !line.content.starts_with(' '),
                "content is stored trimmed: {:?}",
                line.content
            );
        }
        assert_eq!(
            emphasised(&lines[0].content, &lines[0].emphasis),
            vec!["old;"]
        );
        assert_eq!(
            emphasised(&lines[1].content, &lines[1].emphasis),
            vec!["new;"]
        );
    }

    #[test]
    fn patch_text_lines_have_no_emphasis_because_sides_are_unpaired() {
        let lines = collect_diff_lines("-old line\n+new line\n");

        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.emphasis.is_empty()));
    }

    #[test]
    fn word_emphasis_reports_both_sides_of_a_replacement() {
        let pair = word_emphasis("    let total = a + b;", "    let total = a - b;");

        assert_eq!(
            emphasised("    let total = a + b;", &pair.deleted),
            vec!["+"]
        );
        assert_eq!(
            emphasised("    let total = a - b;", &pair.inserted),
            vec!["-"]
        );
    }

    #[test]
    fn emphasised_spans_are_split_without_losing_text_or_style() {
        let style = Style::default().fg(Color::Rgb(1, 2, 3));
        let spans = vec![Span::styled("let x = 2;".to_string(), style)];
        // Emphasise `2` only: byte range 8..9.
        let out = emphasize_diff_spans(spans, &[(8, 9)], 10);

        let rebuilt: String = out.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(
            rebuilt, "let x = 2;",
            "no text may be dropped when splitting"
        );
        assert!(
            out.iter()
                .all(|span| span.style.fg == Some(Color::Rgb(1, 2, 3))),
            "syntax colour must survive emphasis"
        );
        let accented: Vec<&str> = out
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::BOLD))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(accented, vec!["2"]);
    }

    #[test]
    fn spans_are_returned_untouched_when_there_is_no_emphasis() {
        let spans = vec![Span::raw("unchanged".to_string())];
        let out = emphasize_diff_spans(spans.clone(), &[], 9);
        assert_eq!(out.len(), spans.len());
        assert!(!out[0].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn emphasis_past_the_truncation_point_is_ignored() {
        // The renderer clips long lines; ranges beyond what it drew must not
        // accent the trailing ellipsis or panic.
        let style = Style::default();
        let spans = vec![Span::styled("abcdef".to_string(), style)];
        let out = emphasize_diff_spans(spans, &[(20, 24)], 6);

        let rebuilt: String = out.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(rebuilt, "abcdef");
        assert!(
            out.iter()
                .all(|span| !span.style.add_modifier.contains(Modifier::BOLD))
        );
    }
}
