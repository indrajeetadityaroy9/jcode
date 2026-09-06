//! KV cache telemetry types and the `KvCache` info widget that renders them.
//!
//! Split out of `info_widget.rs` to keep that file within its size budget. The
//! unit is self-contained: the session-level cache-hit accounting types
//! (`CacheHitInfo`, `CacheMissAttribution`) share the `effective_prompt_tokens`
//! denominator heuristic with the ratios computed on them, and the renderer
//! plus its percentage/color/token formatting helpers are used by nothing else.

use super::InfoWidgetData;
use crate::tui::color_support::rgb;
use ratatui::prelude::*;

/// Session-level KV cache telemetry for providers that report cache usage.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheHitInfo {
    /// Input tokens from completed API requests that included explicit cache telemetry.
    pub reported_input_tokens: u64,
    /// Tokens read from provider KV/prefix cache across this session.
    pub read_tokens: u64,
    /// Tokens written/created in provider cache across this session, when reported.
    pub creation_tokens: u64,
    /// Approximate reusable prefix tokens expected to be cache-readable.
    pub optimal_input_tokens: u64,
    /// Input tokens from the latest completed request with cache telemetry.
    pub last_reported_input_tokens: Option<u64>,
    /// Cached input tokens read on the latest completed request with cache telemetry.
    pub last_read_tokens: Option<u64>,
    /// Tokens written/created in provider cache on the latest completed request.
    pub last_creation_tokens: Option<u64>,
    /// Approximate reusable prefix tokens expected on the latest completed request.
    pub last_optimal_input_tokens: Option<u64>,
    /// Recent attributed misses with estimated cacheable tokens not read.
    pub miss_attributions: Vec<CacheMissAttribution>,
}

/// Effective prompt size to use as the denominator for cache-hit ratios.
///
/// Providers report `input_tokens` differently:
/// - Anthropic/Claude (split accounting): `input` is the *uncached remainder*,
///   while cache-read and cache-creation tokens are reported separately, so the
///   true prompt size is `input + read + creation`.
/// - OpenAI-style (subset accounting): cached tokens are already counted inside
///   `input`, so the prompt size is just `input`.
///
/// We don't always know the provider at the point a ratio is computed, so we use
/// the same heuristic the compaction path uses: treat accounting as split when a
/// cache-creation count exists or when reported reads exceed the bare input.
pub fn effective_prompt_tokens(input: u64, read: u64, creation: u64) -> u64 {
    if creation > 0 || read > input {
        input.saturating_add(read).saturating_add(creation)
    } else {
        input
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMissAttribution {
    pub turn_number: usize,
    pub call_index: u16,
    pub missed_tokens: u64,
    pub reason: String,
}

impl CacheHitInfo {
    /// Effective total prompt tokens across the session (read denominator).
    fn effective_reported_tokens(&self) -> u64 {
        effective_prompt_tokens(
            self.reported_input_tokens,
            self.read_tokens,
            self.creation_tokens,
        )
    }

    /// Fraction of the session's prompt tokens that were served from cache.
    pub fn hit_ratio(&self) -> Option<f32> {
        let denominator = self.effective_reported_tokens();
        if denominator == 0 {
            None
        } else {
            Some((self.read_tokens as f32 / denominator as f32).clamp(0.0, 1.0))
        }
    }

    /// Fraction of the previously-cacheable prompt that was actually reused
    /// (read_tokens vs. the prior request's full prompt).
    pub fn optimal_ratio(&self) -> Option<f32> {
        if self.optimal_input_tokens == 0 {
            None
        } else {
            Some((self.read_tokens as f32 / self.optimal_input_tokens as f32).clamp(0.0, 1.0))
        }
    }

    pub fn last_ratio(&self) -> Option<f32> {
        let input = self.last_reported_input_tokens?;
        let denominator = effective_prompt_tokens(
            input,
            self.last_read_tokens.unwrap_or(0),
            self.last_creation_tokens.unwrap_or(0),
        );
        if denominator == 0 {
            None
        } else {
            Some((self.last_read_tokens.unwrap_or(0) as f32 / denominator as f32).clamp(0.0, 1.0))
        }
    }

    pub fn last_optimal_ratio(&self) -> Option<f32> {
        let optimal = self.last_optimal_input_tokens?;
        if optimal == 0 {
            None
        } else {
            Some((self.last_read_tokens.unwrap_or(0) as f32 / optimal as f32).clamp(0.0, 1.0))
        }
    }
}

pub(super) fn render_kv_cache_widget(data: &InfoWidgetData, _inner: Rect) -> Vec<Line<'static>> {
    let Some(cache) = data.cache_hit_info.as_ref() else {
        return Vec::new();
    };
    let mut lines = vec![render_kv_cache_summary_line(cache)];

    lines.push(Line::from(vec![Span::styled(
        "miss attribution",
        Style::default().fg(rgb(140, 140, 150)).bold(),
    )]));

    if cache.miss_attributions.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "none",
            Style::default().fg(rgb(110, 210, 140)),
        )]));
        return lines;
    }

    let total_missed: u64 = cache
        .miss_attributions
        .iter()
        .map(|sample| sample.missed_tokens)
        .sum();
    lines.push(Line::from(vec![Span::styled(
        format!("{} missed total", compact_token_count(total_missed)),
        Style::default().fg(rgb(180, 180, 190)),
    )]));

    for sample in cache.miss_attributions.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled(
                format_cache_turn_label(sample.turn_number, sample.call_index),
                Style::default().fg(rgb(140, 180, 255)).bold(),
            ),
            Span::styled(
                format!(" {} miss ", compact_token_count(sample.missed_tokens)),
                Style::default().fg(rgb(255, 200, 100)),
            ),
            Span::styled(
                format!("({})", sample.reason),
                Style::default().fg(rgb(140, 140, 150)),
            ),
        ]));
    }

    if cache.miss_attributions.len() > 5 {
        lines.push(Line::from(vec![Span::styled(
            format!("… {} more", cache.miss_attributions.len() - 5),
            Style::default().fg(rgb(100, 100, 110)),
        )]));
    }

    lines
}

pub(super) fn render_kv_cache_summary_line(cache: &CacheHitInfo) -> Line<'static> {
    let Some(lifetime_ratio) = cache.hit_ratio() else {
        return Line::default();
    };

    let lifetime_pct = ratio_pct(lifetime_ratio);
    let warm_pct = cache.optimal_ratio().map(ratio_pct);
    let last_pct = cache.last_ratio().map(ratio_pct);
    let last_optimal_pct = cache.last_optimal_ratio().map(ratio_pct);
    let health_pct = last_optimal_pct
        .or(last_pct)
        .or(warm_pct)
        .unwrap_or(lifetime_pct);
    let color = kv_cache_optimal_color(health_pct);

    let mut spans = vec![Span::styled(
        "KV cache: ",
        Style::default().fg(rgb(180, 180, 190)).bold(),
    )];

    if let Some(warm_pct) = warm_pct {
        spans.push(Span::styled(
            "yield ",
            Style::default().fg(rgb(140, 140, 150)),
        ));
        spans.push(Span::styled(
            format!("{}%", warm_pct),
            Style::default().fg(color).bold(),
        ));
    } else {
        spans.push(Span::styled(
            "priming",
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(last_pct) = last_pct {
        spans.push(Span::styled(" · ", Style::default().fg(rgb(80, 80, 90))));
        spans.push(Span::styled(
            "last ",
            Style::default().fg(rgb(140, 140, 150)),
        ));
        spans.push(Span::styled(
            format!("{}%", last_pct),
            Style::default().fg(color).bold(),
        ));
    }

    spans.push(Span::styled(" · ", Style::default().fg(rgb(80, 80, 90))));
    spans.push(Span::styled(
        "session ",
        Style::default().fg(rgb(140, 140, 150)),
    ));
    spans.push(Span::styled(
        format!("{}%", lifetime_pct),
        Style::default().fg(color).bold(),
    ));

    Line::from(spans)
}

fn ratio_pct(ratio: f32) -> u8 {
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

fn kv_cache_optimal_color(pct: u8) -> Color {
    match pct {
        0..=24 => rgb(255, 110, 110),
        25..=59 => rgb(255, 200, 100),
        60..=84 => rgb(140, 180, 255),
        _ => rgb(110, 210, 140),
    }
}

fn format_cache_turn_label(turn_number: usize, call_index: u16) -> String {
    if call_index <= 1 {
        format!("{}>", turn_number)
    } else {
        format!("{}.{}>", turn_number, call_index)
    }
}

fn compact_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f32 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f32 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
