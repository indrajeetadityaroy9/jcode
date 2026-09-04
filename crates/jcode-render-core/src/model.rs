//! Backend-neutral rendering primitives shared by jcode's markdown renderers.

use serde::{Deserialize, Serialize};

/// Horizontal alignment for a rendered line.
///
/// Consumed by the TUI markdown renderers to carry per-column table alignment
/// from `pulldown_cmark::Alignment` through to cell padding, without those
/// modules depending on each other's types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}
