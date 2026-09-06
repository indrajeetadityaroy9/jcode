//! Wrapped-line preparation for the prepared chat frame.
//!
//! Owns the two terminal-width wrapping passes that turn a built body's
//! unwrapped `Line`s into a `PreparedMessages`: `wrap_lines` for bodies with no
//! raw-line bookkeeping (header, streaming, batch progress) and
//! `wrap_lines_with_map` for transcript bodies, which additionally translates
//! unwrapped-line indices into wrapped-line indices for the raw/logical line
//! map, edit-tool ranges, copy targets and per-message boundaries.
//!
//! Split out of `ui_prepare.rs` (which builds the lines) so the width-mapping
//! layer stands apart from message rendering and the parent stays inside its
//! code-size budget.

use super::compute_image_regions;
use crate::tui::markdown;
use crate::tui::ui::{
    self, CopyTarget, EditToolRange, MessageBoundary, PreparedMessages, RawCopyTarget,
    WrappedLineMap,
};
use ratatui::text::Line;
use std::sync::Arc;

pub(super) fn wrap_lines(
    lines: Vec<Line<'static>>,
    line_copy_offsets: &[usize],
    user_line_indices: &[usize],
    user_prompt_texts: &[String],
    width: u16,
) -> PreparedMessages {
    let full_width = width.saturating_sub(1) as usize;
    let user_width = width.saturating_sub(2) as usize;
    let mut wrapped_user_indices: Vec<usize> = Vec::new();
    let mut wrapped_user_prompt_starts: Vec<usize> = Vec::new();
    let mut wrapped_user_prompt_ends: Vec<usize> = Vec::new();
    let mut raw_plain_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut wrapped_line_map: Vec<WrappedLineMap> = Vec::new();
    let mut wrapped_copy_offsets: Vec<usize> = Vec::new();
    let mut user_line_mask = vec![false; lines.len()];
    for &idx in user_line_indices {
        if idx < user_line_mask.len() {
            user_line_mask[idx] = true;
        }
    }
    let mut wrapped_idx = 0usize;

    let mut wrapped_lines: Vec<Line> = Vec::new();
    for (orig_idx, line) in lines.into_iter().enumerate() {
        let raw_text = ui::line_plain_text(&line);
        let raw_width = unicode_width::UnicodeWidthStr::width(raw_text.as_str());
        raw_plain_lines.push(raw_text);
        let is_user_line = user_line_mask.get(orig_idx).copied().unwrap_or(false);
        let wrap_width = if is_user_line { user_width } else { full_width };
        let new_lines = markdown::wrap_line(line, wrap_width);
        let count = new_lines.len();
        let mut remaining_copy_offset = line_copy_offsets.get(orig_idx).copied().unwrap_or(0);
        let mut start_col = 0usize;

        for wrapped_line in &new_lines {
            let width = wrapped_line.width();
            let end_col = (start_col + width).min(raw_width);
            wrapped_line_map.push(WrappedLineMap {
                raw_line: orig_idx,
                start_col,
                end_col,
            });
            wrapped_copy_offsets.push(remaining_copy_offset.min(width));
            remaining_copy_offset = remaining_copy_offset.saturating_sub(width);
            start_col = end_col;
        }

        if is_user_line {
            wrapped_user_prompt_starts.push(wrapped_idx);
            wrapped_user_prompt_ends.push(wrapped_idx + count);
            for i in 0..count {
                wrapped_user_indices.push(wrapped_idx + i);
            }
        }

        wrapped_lines.extend(new_lines);
        wrapped_idx += count;
    }

    let image_regions = compute_image_regions(&wrapped_lines);

    let wrapped_plain_lines = Arc::new(wrapped_lines.iter().map(ui::line_plain_text).collect());

    PreparedMessages {
        wrapped_lines,
        wrapped_plain_lines,
        wrapped_copy_offsets: Arc::new(wrapped_copy_offsets),
        raw_plain_lines: Arc::new(raw_plain_lines),
        wrapped_line_map: Arc::new(wrapped_line_map),
        wrapped_user_indices,
        wrapped_user_prompt_starts,
        wrapped_user_prompt_ends,
        user_prompt_texts: user_prompt_texts.to_vec(),
        image_regions,
        edit_tool_ranges: Vec::new(),
        copy_targets: Vec::new(),
        message_boundaries: Vec::new(),
        mermaid_pending_epoch: None,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Wrapped-line preparation carries explicit render state to avoid hidden coupling"
)]
pub(super) fn wrap_lines_with_map(
    lines: Vec<Line<'static>>,
    seeded_raw_plain_lines: &[String],
    line_raw_overrides: &[Option<WrappedLineMap>],
    line_copy_offsets: &[usize],
    user_line_indices: &[usize],
    user_prompt_texts: &[String],
    width: u16,
    edit_ranges: &[(usize, String, usize, usize, bool)],
    copy_ranges: &[RawCopyTarget],
    segments: &[(u64, usize, usize, usize)],
) -> PreparedMessages {
    let full_width = width.saturating_sub(1) as usize;
    let user_width = width.saturating_sub(2) as usize;
    let mut wrapped_user_indices: Vec<usize> = Vec::new();
    let mut wrapped_user_prompt_starts: Vec<usize> = Vec::new();
    let mut wrapped_user_prompt_ends: Vec<usize> = Vec::new();
    let mut raw_plain_lines: Vec<String> = seeded_raw_plain_lines.to_vec();
    let mut wrapped_line_map: Vec<WrappedLineMap> = Vec::new();
    let mut wrapped_copy_offsets: Vec<usize> = Vec::new();
    let mut user_line_mask = vec![false; lines.len()];
    for &idx in user_line_indices {
        if idx < user_line_mask.len() {
            user_line_mask[idx] = true;
        }
    }
    let mut wrapped_idx = 0usize;

    let mut raw_to_wrapped: Vec<usize> = Vec::with_capacity(lines.len() + 1);

    let mut wrapped_lines: Vec<Line> = Vec::new();
    for (orig_idx, line) in lines.into_iter().enumerate() {
        let (raw_line, start_col, end_col) =
            if let Some(Some(map)) = line_raw_overrides.get(orig_idx) {
                (map.raw_line, map.start_col, map.end_col)
            } else {
                let raw_text = ui::line_plain_text(&line);
                let raw_width = unicode_width::UnicodeWidthStr::width(raw_text.as_str());
                let raw_line = raw_plain_lines.len();
                raw_plain_lines.push(raw_text);
                (raw_line, 0usize, raw_width)
            };
        raw_to_wrapped.push(wrapped_idx);
        let is_user_line = user_line_mask.get(orig_idx).copied().unwrap_or(false);
        let wrap_width = if is_user_line { user_width } else { full_width };
        let new_lines = markdown::wrap_line(line, wrap_width);
        let count = new_lines.len();
        let mut remaining_copy_offset = line_copy_offsets.get(orig_idx).copied().unwrap_or(0);
        let mut segment_start = start_col;

        for wrapped_line in &new_lines {
            let width = wrapped_line.width();
            let segment_end = (segment_start + width).min(end_col);
            wrapped_line_map.push(WrappedLineMap {
                raw_line,
                start_col: segment_start,
                end_col: segment_end,
            });
            wrapped_copy_offsets.push(remaining_copy_offset.min(width));
            remaining_copy_offset = remaining_copy_offset.saturating_sub(width);
            segment_start = segment_end;
        }

        if is_user_line {
            wrapped_user_prompt_starts.push(wrapped_idx);
            wrapped_user_prompt_ends.push(wrapped_idx + count);
            for i in 0..count {
                wrapped_user_indices.push(wrapped_idx + i);
            }
        }

        wrapped_lines.extend(new_lines);
        wrapped_idx += count;
    }
    raw_to_wrapped.push(wrapped_idx);

    let image_regions = compute_image_regions(&wrapped_lines);

    let mut edit_tool_ranges = Vec::new();
    for (msg_idx, file_path, raw_start, raw_end, expandable) in edit_ranges {
        let start_line = raw_to_wrapped.get(*raw_start).copied().unwrap_or(0);
        let end_line = raw_to_wrapped
            .get(*raw_end)
            .copied()
            .unwrap_or(wrapped_lines.len());
        edit_tool_ranges.push(EditToolRange {
            edit_index: edit_tool_ranges.len(),
            msg_index: *msg_idx,
            file_path: file_path.clone(),
            start_line,
            end_line,
            expandable: *expandable,
        });
    }

    let mut copy_targets = Vec::new();
    for target in copy_ranges {
        let start_line = raw_to_wrapped
            .get(target.start_raw_line)
            .copied()
            .unwrap_or(0);
        let end_line = raw_to_wrapped
            .get(target.end_raw_line)
            .copied()
            .unwrap_or(wrapped_lines.len());
        let badge_line = raw_to_wrapped
            .get(target.badge_raw_line)
            .copied()
            .unwrap_or(start_line);
        copy_targets.push(CopyTarget {
            kind: target.kind.clone(),
            content: target.content.clone(),
            start_line,
            end_line,
            badge_line,
        });
    }

    let wrapped_plain_lines = Arc::new(wrapped_lines.iter().map(ui::line_plain_text).collect());

    // Translate the per-message unwrapped-line boundaries into wrapped-line
    // boundaries. `raw_to_wrapped[i]` is the first wrapped index produced by
    // unwrapped line `i` (with a final sentinel at `lines.len()`), so a segment
    // that ends just before unwrapped line `L` ends at wrapped line
    // `raw_to_wrapped[L]`. This lets a later rebuild truncate the wrapped body at
    // any message boundary and reuse the unchanged prefix.
    let message_boundaries: Vec<MessageBoundary> = segments
        .iter()
        .map(
            |&(msg_hash, lines_len_after, raw_len_after, user_prompt_len_after)| MessageBoundary {
                msg_hash,
                wrapped_len: raw_to_wrapped
                    .get(lines_len_after)
                    .copied()
                    .unwrap_or(wrapped_lines.len()),
                raw_len: raw_len_after,
                user_prompt_len: user_prompt_len_after,
            },
        )
        .collect();

    PreparedMessages {
        wrapped_lines,
        wrapped_plain_lines,
        wrapped_copy_offsets: Arc::new(wrapped_copy_offsets),
        raw_plain_lines: Arc::new(raw_plain_lines),
        wrapped_line_map: Arc::new(wrapped_line_map),
        wrapped_user_indices,
        wrapped_user_prompt_starts,
        wrapped_user_prompt_ends,
        user_prompt_texts: user_prompt_texts.to_vec(),
        image_regions,
        edit_tool_ranges,
        copy_targets,
        message_boundaries,
        mermaid_pending_epoch: None,
    }
}
