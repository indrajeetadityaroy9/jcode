//! Ignored perf probe for the syntect regex backend.
//!
//! Motivation: a jemalloc heap profile of a long-lived TUI client attributed
//! ~24 MB (47% of live heap) to lazily-compiled `regex-fancy` highlight
//! grammars (`fancy_regex` + `regex_automata` meta engines). Switching syntect
//! to the `regex-onig` backend cut the probe client's idle live heap from
//! ~51 MB to ~34 MB on the same workload. This probe measures the other half
//! of that tradeoff - grammar-compile cost and steady-state highlight
//! throughput on a representative multi-language workload - so a future
//! backend change can be A/B'd with one command. Heap attribution itself
//! needs an external profiler (`leaks`, Instruments, a jemalloc dump) and is
//! not measured here.
//!
//! Run with:
//!   cargo test -p jcode-tui-markdown --test highlight_backend_probe -- --ignored --nocapture

use jcode_tui_markdown::highlight_line;

/// Representative code lines across the languages a transcript actually
/// highlights (tool output diffs, fenced blocks in assistant replies).
fn workload() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "rs",
            "pub fn handle_server_event(app: &mut App, event: ServerEvent) -> Result<()> {",
        ),
        (
            "rs",
            "    let resolved = std::sync::Arc::new(resolve_anchored_items(&app.side_pane_images()));",
        ),
        (
            "py",
            "def compute_totals(items: list[dict], *, key: str = 'bytes') -> int:",
        ),
        (
            "py",
            "    return sum(item.get(key, 0) for item in items if item['active'])",
        ),
        (
            "js",
            "const totals = items.filter(i => i.active).reduce((acc, i) => acc + i.bytes, 0);",
        ),
        (
            "ts",
            "export async function fetchState(id: string): Promise<SessionState | null> {",
        ),
        (
            "go",
            "func (s *Server) HandleConn(ctx context.Context, conn net.Conn) error {",
        ),
        (
            "json",
            "{\"session_id\": \"abc\", \"messages\": 322, \"json_bytes\": 2133255}",
        ),
        (
            "sh",
            "for pid in $(pgrep -f jcode); do awk '/VmRSS/{print $2}' /proc/$pid/status; done",
        ),
        (
            "toml",
            "syntect = { version = \"5\", default-features = false, features = [\"regex-onig\"] }",
        ),
        ("md", "## Heading with `inline code` and **bold** text"),
        (
            "c",
            "static int parse_header(const uint8_t *buf, size_t len, struct dims *out) {",
        ),
        (
            "sql",
            "SELECT session_id, SUM(json_bytes) FROM sessions GROUP BY session_id ORDER BY 2 DESC;",
        ),
        ("yaml", "jobs:\n  build:\n    runs-on: ubuntu-latest"),
        (
            "html",
            "<div class=\"transcript\"><span data-id=\"42\">hello</span></div>",
        ),
    ]
}

/// Measures grammar-compile cost (first pass) and steady-state highlight
/// throughput with warm grammars.
#[test]
#[ignore = "perf probe; run explicitly with --ignored --nocapture"]
fn highlight_backend_probe() {
    let lines = workload();

    // First pass: pays lazy grammar compilation for every language.
    let cold_start = std::time::Instant::now();
    for (ext, code) in &lines {
        let spans = highlight_line(code, Some(ext));
        assert!(
            !spans.is_empty(),
            "highlighting produced no spans for {ext}"
        );
    }
    let cold = cold_start.elapsed();

    // Steady state: repeated highlighting with warm grammars, sized to mimic
    // re-rendering a large transcript several times.
    const STEADY_ITERS: usize = 200;
    let steady_start = std::time::Instant::now();
    for _ in 0..STEADY_ITERS {
        for (ext, code) in &lines {
            let _ = highlight_line(code, Some(ext));
        }
    }
    let steady = steady_start.elapsed();

    let steady_lines = STEADY_ITERS * lines.len();
    let lines_per_sec = steady_lines as f64 / steady.as_secs_f64();

    println!("highlight backend probe:");
    println!("  cold pass ({} langs):      {:?}", lines.len(), cold);
    println!(
        "  steady state:              {steady_lines} lines in {steady:?} ({lines_per_sec:.0} lines/s)"
    );
}
