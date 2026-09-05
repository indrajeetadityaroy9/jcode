//! Nearest-match hints for edit tools that failed to locate their target.
//!
//! When `edit`, `multiedit` or `apply_patch` cannot find the lines it was told
//! to replace, the file has almost always drifted slightly — a renamed symbol,
//! a reformatted argument list, a line that moved. Telling the caller only
//! "not found" costs a full round trip: read the file, compare by eye, retry.
//! Measured on real session transcripts, every locate failure was followed by
//! at least one extra `bash`/`read` call before the retry.
//!
//! This module answers "not found, but here is the closest thing and where it
//! is" using `similar`, which already backs every diff in the write path.
//!
//! Cost is bounded by construction, because this runs on a failure path over
//! a file that may be very large:
//!
//! 1. One `get_close_matches` pass over the file's trimmed lines, anchored on
//!    the most distinctive line of the needle. `similar` pre-filters each
//!    candidate with a length bound and a quick character-multiset ratio
//!    before it ever builds a diff.
//! 2. A full character-level ratio for at most [`MAX_CANDIDATE_WINDOWS`]
//!    windows around those anchors.
//!
//! Line-level ratios are deliberately *not* used for scoring: `TextDiff`
//! matches whole lines, so a one-character difference scores 0 and every
//! near-miss looks equally wrong. Character-level ratios are what make
//! "82% similar" meaningful.

use similar::{TextDiff, get_close_matches};

/// Anchor lines shorter than this carry no signal — `}`, `);`, `else {` match
/// everywhere — so a needle made only of such lines yields no hint at all
/// rather than a confidently wrong one.
const MIN_ANCHOR_CHARS: usize = 4;

/// How many anchor lines `similar` is asked to return.
const MAX_ANCHORS: usize = 5;

/// Upper bound on full-window comparisons, so a needle whose anchor repeats
/// hundreds of times cannot turn a failed edit into a slow failed edit.
const MAX_CANDIDATE_WINDOWS: usize = 24;

/// Minimum similarity worth reporting. Below this the "closest match" is noise
/// and pointing at it would send the caller to the wrong place.
const MIN_RATIO: f32 = 0.5;

/// Lines of the candidate to quote back.
const QUOTED_LINES: usize = 3;

/// Longest quoted line; keeps a minified or data line from filling the reply.
const MAX_QUOTED_CHARS: usize = 160;

/// The closest region of a file to a needle that was not found in it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocateHint {
    /// 1-based line number where the closest region starts.
    pub line: usize,
    /// Character-level similarity, 0.0 to 1.0.
    pub ratio: f32,
    /// The region as it actually appears in the file, clipped for display.
    pub excerpt: String,
}

impl LocateHint {
    /// Render as a suffix for a "not found" error.
    ///
    /// The percentage is what tells the caller which failure this is: a high
    /// score means whitespace or a one-token drift and the retry should copy
    /// the quoted text; a low score means the target is elsewhere entirely.
    pub(crate) fn describe(&self) -> String {
        format!(
            "Closest match in the file is {}% similar, at line {}:\n{}",
            (self.ratio * 100.0).round() as u32,
            self.line,
            self.excerpt
        )
    }
}

/// Find the region of `content` most similar to `needle`.
///
/// Returns `None` when the file is empty, the needle has no distinctive line,
/// or nothing reaches [`MIN_RATIO`] — an absent hint is better than one that
/// points at an unrelated line.
pub(crate) fn closest_match(content: &str, needle: &str) -> Option<LocateHint> {
    let needle_lines: Vec<&str> = needle.lines().collect();
    let content_lines: Vec<&str> = content.lines().collect();
    if needle_lines.is_empty() || content_lines.is_empty() {
        return None;
    }

    // Anchor on the needle's longest line: the most distinctive one, and the
    // one whose near-misses are most likely to be the real drift.
    let anchor = needle_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| line.chars().count() >= MIN_ANCHOR_CHARS)
        .max_by_key(|line| line.chars().count())?;

    let trimmed: Vec<&str> = content_lines.iter().map(|line| line.trim()).collect();
    let anchors = get_close_matches(anchor, &trimmed, MAX_ANCHORS, MIN_RATIO);
    if anchors.is_empty() {
        return None;
    }

    let window = needle_lines.len();
    let needle_text = needle_lines.join("\n");
    let mut best: Option<LocateHint> = None;
    let mut compared = 0usize;

    for candidate in anchors {
        for (index, line) in trimmed.iter().enumerate() {
            if line != &candidate {
                continue;
            }
            if compared >= MAX_CANDIDATE_WINDOWS {
                break;
            }
            compared += 1;

            // The anchor may sit anywhere inside the needle, so align the
            // window on the anchor's offset instead of assuming it is first.
            let anchor_offset = needle_lines
                .iter()
                .position(|needle_line| needle_line.trim() == candidate)
                .unwrap_or(0);
            let start = index.saturating_sub(anchor_offset);
            let end = (start + window).min(content_lines.len());
            let region = content_lines[start..end].join("\n");

            let ratio = TextDiff::from_chars(region.as_str(), needle_text.as_str()).ratio();
            if ratio < MIN_RATIO {
                continue;
            }
            if best.as_ref().is_none_or(|current| ratio > current.ratio) {
                best = Some(LocateHint {
                    line: start + 1,
                    ratio,
                    excerpt: excerpt(&content_lines[start..end], start + 1),
                });
            }
        }
    }

    best
}

/// Quote the found region the way `read` does, so the caller can copy it
/// verbatim into the retry.
fn excerpt(lines: &[&str], first_line: usize) -> String {
    lines
        .iter()
        .take(QUOTED_LINES)
        .enumerate()
        .map(|(offset, line)| {
            let mut text: String = line.chars().take(MAX_QUOTED_CHARS).collect();
            if line.chars().count() > MAX_QUOTED_CHARS {
                text.push_str(" ...");
            }
            format!("{:>4}│ {}", first_line + offset, text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "locate_hint_tests.rs"]
mod locate_hint_tests;
