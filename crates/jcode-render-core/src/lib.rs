//! # jcode-render-core
//!
//! Backend-neutral text-rendering primitives shared by jcode's TUI crates,
//! kept free of any dependency on ratatui so that `jcode-base` (the
//! foundation layer) can use them too.
//!
//! What lives here:
//!
//! - [`math`] — LaTeX math laid out to unicode text (inline and display).
//! - [`preprocess`] — math/currency normalization applied before parsing.
//! - [`reasoning`] — the reasoning-block sentinel and its line markup, used by
//!   session rendering in `jcode-base` as well as by the TUI.
//! - [`model`] — the shared [`model::Alignment`] used for table columns.
//!
//! The authoritative markdown renderer is `jcode-tui-markdown`; this crate
//! deliberately holds only the pieces that more than one layer needs.

pub mod math;
pub mod model;
pub mod preprocess;
pub mod reasoning;

pub use math::{render_display_latex, render_inline_latex};
pub use model::Alignment;
pub use preprocess::{escape_currency_dollars, normalize_latex_math};
pub use reasoning::{
    REASONING_SENTINEL, reasoning_line_markup, reasoning_partial_markup,
    reasoning_summary_line_markup,
};
