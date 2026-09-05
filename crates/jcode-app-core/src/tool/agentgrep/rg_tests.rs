//! Tests for the ripgrep-engine grep mode.
//!
//! Where a real `rg` binary is available it is used as the correctness oracle:
//! our match set must equal ripgrep's for the same arguments. Those cases are
//! skipped (not failed) when `rg` is absent, because the whole point of this
//! module is that jcode no longer requires it.

use super::*;
use std::process::Command;

fn args(query: &str) -> GrepArgs {
    GrepArgs {
        query: query.to_string(),
        regex: false,
        file_type: None,
        json: false,
        paths_only: false,
        hidden: false,
        no_ignore: false,
        path: None,
        glob: None,
    }
}

/// A git repository, so `.gitignore` is authoritative exactly as it is for
/// ripgrep (which only honors it inside a repo).
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "// top level marker note\n\
         pub fn alpha_marker() -> u32 {\n\
         \x20   let marker = 1;\n\
         \x20   marker + 1\n\
         }\n\
         \n\
         pub struct Holder {\n\
         \x20   pub marker: u32,\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/other.py"),
        "def gamma():\n    marker = 2\n    return marker\n",
    )
    .unwrap();
    std::fs::write(root.join("notes.md"), "marker in markdown\n").unwrap();
    std::fs::write(root.join(".hidden/secret.rs"), "fn hidden_marker() {}\n").unwrap();
    std::fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
    std::fs::write(root.join("ignored.rs"), "fn ignored_marker() {}\n").unwrap();

    let git = Command::new("git").arg("init").current_dir(root).output();
    assert!(
        git.map(|out| out.status.success()).unwrap_or(false),
        "fixture needs a git repo so .gitignore is authoritative"
    );
    dir
}

fn matched_paths(result: &GrepResult) -> Vec<String> {
    let mut paths: Vec<String> = result.files.iter().map(|f| f.path.clone()).collect();
    paths.sort();
    paths
}

/// Ask the real ripgrep for the same file set, or `None` when `rg` is absent.
fn rg_oracle_paths(root: &std::path::Path, extra: &[&str], query: &str) -> Option<Vec<String>> {
    let output = Command::new("rg")
        .current_dir(root)
        .args(["--files-with-matches", "--color", "never", "--fixed-strings"])
        .args(extra)
        .args(["-e", query, "."])
        .output()
        .ok()?;
    // 0 = matches, 1 = no matches; anything else means rg rejected the query.
    if !matches!(output.status.code(), Some(0) | Some(1)) {
        return None;
    }
    let mut paths: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim_start_matches("./").to_string())
        .filter(|line| !line.is_empty())
        .collect();
    paths.sort();
    Some(paths)
}

#[test]
fn literal_search_matches_ripgrep_file_set() {
    let dir = fixture();
    let result = run_grep(dir.path(), &args("marker")).expect("grep");

    assert_eq!(
        matched_paths(&result),
        vec![
            "notes.md".to_string(),
            "src/lib.rs".to_string(),
            "src/other.py".to_string()
        ],
        "gitignored and hidden files must stay out of a default search"
    );

    if let Some(oracle) = rg_oracle_paths(dir.path(), &[], "marker") {
        assert_eq!(
            matched_paths(&result), oracle,
            "our engine disagreed with the installed ripgrep"
        );
    }
}

#[test]
fn hidden_and_no_ignore_flags_widen_the_search_like_ripgrep() {
    let dir = fixture();

    let mut hidden = args("marker");
    hidden.hidden = true;
    let hidden_result = run_grep(dir.path(), &hidden).expect("grep");
    assert!(
        matched_paths(&hidden_result).contains(&".hidden/secret.rs".to_string()),
        "hidden=true must reach dotfiles: {:?}",
        matched_paths(&hidden_result)
    );
    if let Some(oracle) = rg_oracle_paths(dir.path(), &["--hidden"], "marker") {
        assert_eq!(matched_paths(&hidden_result), oracle);
    }

    let mut no_ignore = args("marker");
    no_ignore.no_ignore = true;
    let no_ignore_result = run_grep(dir.path(), &no_ignore).expect("grep");
    assert!(
        matched_paths(&no_ignore_result).contains(&"ignored.rs".to_string()),
        "no_ignore=true must reach gitignored files: {:?}",
        matched_paths(&no_ignore_result)
    );
    if let Some(oracle) = rg_oracle_paths(dir.path(), &["--no-ignore"], "marker") {
        assert_eq!(matched_paths(&no_ignore_result), oracle);
    }
}

#[test]
fn file_type_and_glob_narrow_the_search() {
    let dir = fixture();

    let mut typed = args("marker");
    typed.file_type = Some("rust".to_string());
    let typed_result = run_grep(dir.path(), &typed).expect("grep");
    assert_eq!(
        matched_paths(&typed_result),
        vec!["src/lib.rs".to_string()],
        "type=rust must normalize to the rs extension and exclude .md/.py"
    );

    let mut globbed = args("marker");
    globbed.glob = Some("*.py".to_string());
    let globbed_result = run_grep(dir.path(), &globbed).expect("grep");
    assert_eq!(
        matched_paths(&globbed_result),
        vec!["src/other.py".to_string()],
        "a bare *.py glob must match on file name, not just the full display path"
    );
}

#[test]
fn literal_query_is_not_treated_as_a_pattern() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), "a+b\naxb\n").unwrap();

    let result = run_grep(dir.path(), &args("a+b")).expect("grep");
    let lines: Vec<usize> = result.files[0]
        .matches
        .iter()
        .map(|m| m.line_number)
        .collect();
    assert_eq!(
        lines,
        vec![1],
        "a literal query must not compile `+` as a quantifier"
    );

    let mut regex = args("a.b");
    regex.regex = true;
    let regex_result = run_grep(dir.path(), &regex).expect("grep");
    assert_eq!(
        regex_result.total_matches, 2,
        "regex=true must let `.` match both lines"
    );
}

#[test]
fn invalid_regex_is_reported_rather_than_silently_returning_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();

    let mut bad = args("a(");
    bad.regex = true;
    let err = run_grep(dir.path(), &bad).expect_err("unbalanced group must fail");
    assert!(
        err.contains("invalid search pattern"),
        "error should name the cause: {err}"
    );
}

#[test]
fn matches_are_grouped_under_their_enclosing_symbol() {
    let dir = fixture();
    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    let lib = result
        .files
        .iter()
        .find(|f| f.path == "src/lib.rs")
        .expect("lib.rs matched");

    assert_eq!(lib.language, "rust");
    let labels: Vec<&str> = lib.groups.iter().map(|g| g.label.as_str()).collect();
    assert!(
        labels.contains(&"<file scope>"),
        "the line-1 comment match belongs to file scope: {labels:?}"
    );
    assert!(
        labels.contains(&"alpha_marker"),
        "matches inside the fn must group under it: {labels:?}"
    );
    assert!(
        lib.matched_symbol_count >= 2,
        "both the fn and the struct matched: {}",
        lib.matched_symbol_count
    );

    let rendered = render(&result, &args("marker"), Some(200));
    assert!(rendered.contains("    - function alpha_marker @ 2-6"));
    assert!(rendered.contains("      - @ 3     let marker = 1;"));
    assert!(rendered.starts_with("query: marker\nmatches: "));
}

#[test]
fn dense_files_skip_structure_and_fall_back_to_file_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut body = String::new();
    for idx in 0..DENSE_MATCH_SKIP_STRUCTURE_THRESHOLD + 5 {
        body.push_str(&format!("fn item_{idx}() {{ marker }}\n"));
    }
    std::fs::write(dir.path().join("dense.rs"), &body).unwrap();

    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    let file = &result.files[0];
    assert_eq!(file.total_symbols, 0, "structure extraction must be skipped");
    assert_eq!(file.groups.len(), 1);
    assert_eq!(file.groups[0].label, "<file scope>");
    assert_eq!(file.language, "rust", "language still comes from the path");
}

#[test]
fn non_code_files_cap_rendered_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = (0..10)
        .map(|idx| format!("marker line {idx}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("data.md"), body).unwrap();

    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    assert_eq!(result.total_matches, 10, "all matches are counted");

    let rendered = render(&result, &args("marker"), Some(200));
    let shown = rendered.matches("      - @ ").count();
    assert_eq!(
        shown, MAX_NON_CODE_MATCH_LINES_PER_FILE,
        "markdown is capped per file"
    );
    assert!(rendered.contains("more non-code matches omitted"));
}

#[test]
fn max_regions_caps_output_and_reports_the_remainder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = (0..20)
        .map(|idx| format!("fn f_{idx}() {{ marker }}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("many.rs"), body).unwrap();

    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    let rendered = render(&result, &args("marker"), Some(5));
    assert_eq!(rendered.matches("      - @ ").count(), 5);
    assert!(
        rendered.contains("... 15 more matches omitted (max_regions=5)"),
        "the omitted count must be reported: {rendered}"
    );
}

#[test]
fn paths_only_returns_bare_paths() {
    let dir = fixture();
    let mut only = args("marker");
    only.paths_only = true;
    let result = run_grep(dir.path(), &only).expect("grep");
    let rendered = render(&result, &only, Some(200));
    assert_eq!(rendered, "notes.md\nsrc/lib.rs\nsrc/other.py");
}

#[test]
fn binary_files_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("blob.bin"), b"marker\x00marker\n").unwrap();
    std::fs::write(dir.path().join("plain.txt"), "marker\n").unwrap();

    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    assert_eq!(
        matched_paths(&result),
        vec!["plain.txt".to_string()],
        "a NUL byte must take the file out of the result set"
    );
}

#[test]
fn long_match_lines_are_truncated_before_storage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let long = format!("{}marker{}", "a".repeat(400), "b".repeat(400));
    std::fs::write(dir.path().join("min.js"), format!("{long}\n")).unwrap();

    let result = run_grep(dir.path(), &args("marker")).expect("grep");
    let stored = &result.files[0].matches[0].line_text;
    assert!(
        stored.chars().count() < long.chars().count(),
        "an 800-char line must not be stored whole"
    );
    assert!(stored.contains("truncated"));
}

#[test]
fn search_does_not_depend_on_an_rg_binary_being_on_path() {
    let dir = fixture();
    let saved = std::env::var_os("PATH");
    // SAFETY: this test is the only writer of PATH here and restores it before
    // returning; it exists specifically to prove the engine is in-process.
    unsafe { std::env::set_var("PATH", "/nonexistent-jcode-probe") };
    let result = run_grep(dir.path(), &args("marker"));
    match saved {
        Some(path) => unsafe { std::env::set_var("PATH", path) },
        None => unsafe { std::env::remove_var("PATH") },
    }

    let result = result.expect("grep must work with an empty PATH");
    assert_eq!(result.total_matches, 8);
    assert_eq!(matched_paths(&result).len(), 3);
}
