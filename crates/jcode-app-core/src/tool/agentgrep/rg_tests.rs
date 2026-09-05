//! Tests for the ripgrep-engine grep mode.
//!
//! Where a real `rg` binary is available it is used as the correctness oracle:
//! our match set must equal ripgrep's for the same arguments. Those cases are
//! skipped (not failed) when `rg` is absent, because the whole point of this
//! module is that jcode no longer requires it.

use super::*;
use std::process::Command;

/// A default request. `case` is pinned to `Sensitive` rather than the tool's
/// `Smart` default so these tests assert one behavior at a time; the case tests
/// set the mode they exercise. `engine` is the tool's default, the Rust engine;
/// the PCRE2 tests set it explicitly, which is the only way it is ever reached.
fn request(query: &str) -> GrepRequest {
    GrepRequest {
        base: GrepArgs {
            query: query.to_string(),
            regex: false,
            file_type: None,
            json: false,
            paths_only: false,
            hidden: false,
            no_ignore: false,
            path: None,
            glob: None,
        },
        case: CaseMode::Sensitive,
        word: false,
        multiline: false,
        context_lines: 0,
        max_matches_per_file: DEFAULT_MAX_MATCHES_PER_FILE,
        engine: EngineMode::Rust,
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
        .args([
            "--files-with-matches",
            "--color",
            "never",
            "--fixed-strings",
        ])
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
    let result = run_grep(dir.path(), &request("marker")).expect("grep");

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
            matched_paths(&result),
            oracle,
            "our engine disagreed with the installed ripgrep"
        );
    }
}

#[test]
fn hidden_and_no_ignore_flags_widen_the_search_like_ripgrep() {
    let dir = fixture();

    let mut hidden = request("marker");
    hidden.base.hidden = true;
    let hidden_result = run_grep(dir.path(), &hidden).expect("grep");
    assert!(
        matched_paths(&hidden_result).contains(&".hidden/secret.rs".to_string()),
        "hidden=true must reach dotfiles: {:?}",
        matched_paths(&hidden_result)
    );
    if let Some(oracle) = rg_oracle_paths(dir.path(), &["--hidden"], "marker") {
        assert_eq!(matched_paths(&hidden_result), oracle);
    }

    let mut no_ignore = request("marker");
    no_ignore.base.no_ignore = true;
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

    let mut typed = request("marker");
    typed.base.file_type = Some("rust".to_string());
    let typed_result = run_grep(dir.path(), &typed).expect("grep");
    assert_eq!(
        matched_paths(&typed_result),
        vec!["src/lib.rs".to_string()],
        "type=rust must normalize to the rs extension and exclude .md/.py"
    );

    let mut globbed = request("marker");
    globbed.base.glob = Some("*.py".to_string());
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

    let result = run_grep(dir.path(), &request("a+b")).expect("grep");
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

    let mut regex = request("a.b");
    regex.base.regex = true;
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

    let mut bad = request("a(");
    bad.base.regex = true;
    let err = run_grep(dir.path(), &bad).expect_err("unbalanced group must fail");
    assert!(
        err.contains("invalid search pattern"),
        "error should name the cause: {err}"
    );
}

#[test]
fn matches_are_grouped_under_their_enclosing_symbol() {
    let dir = fixture();
    let result = run_grep(dir.path(), &request("marker")).expect("grep");
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

    let rendered = render(&result, &request("marker"), Some(200));
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

    let result = run_grep(dir.path(), &request("marker")).expect("grep");
    let file = &result.files[0];
    assert_eq!(
        file.total_symbols, 0,
        "structure extraction must be skipped"
    );
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

    let result = run_grep(dir.path(), &request("marker")).expect("grep");
    assert_eq!(result.total_matches, 10, "all matches are counted");

    let rendered = render(&result, &request("marker"), Some(200));
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

    let result = run_grep(dir.path(), &request("marker")).expect("grep");
    let rendered = render(&result, &request("marker"), Some(5));
    assert_eq!(rendered.matches("      - @ ").count(), 5);
    assert!(
        rendered.contains("... 15 more matches omitted (max_regions=5)"),
        "the omitted count must be reported: {rendered}"
    );
}

#[test]
fn paths_only_returns_bare_paths() {
    let dir = fixture();
    let mut only = request("marker");
    only.base.paths_only = true;
    let result = run_grep(dir.path(), &only).expect("grep");
    let rendered = render(&result, &only, Some(200));
    assert_eq!(rendered, "notes.md\nsrc/lib.rs\nsrc/other.py");
}

#[test]
fn binary_files_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("blob.bin"), b"marker\x00marker\n").unwrap();
    std::fs::write(dir.path().join("plain.txt"), "marker\n").unwrap();

    let result = run_grep(dir.path(), &request("marker")).expect("grep");
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

    let result = run_grep(dir.path(), &request("marker")).expect("grep");
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
    let result = run_grep(dir.path(), &request("marker"));
    match saved {
        Some(path) => unsafe { std::env::set_var("PATH", path) },
        None => unsafe { std::env::remove_var("PATH") },
    }

    let result = result.expect("grep must work with an empty PATH");
    assert_eq!(result.total_matches, 8);
    assert_eq!(matched_paths(&result).len(), 3);
}

/// A file whose identifiers differ only in case, for the case-mode tests.
fn case_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("case.rs"),
        "let TODO_MARKER = 1;\nlet lowercase_only = 2;\n",
    )
    .unwrap();
    dir
}

#[test]
fn smart_case_widens_an_all_lowercase_query() {
    let dir = case_fixture();

    let mut smart = request("todo_marker");
    smart.case = CaseMode::Smart;
    let smart_result = run_grep(dir.path(), &smart).expect("grep");
    assert_eq!(
        smart_result.total_matches, 1,
        "an all-lowercase query must reach TODO_MARKER under smart case"
    );

    let sensitive = request("todo_marker");
    let sensitive_result = run_grep(dir.path(), &sensitive).expect("grep");
    assert_eq!(
        sensitive_result.total_matches, 0,
        "sensitive mode must keep the old behavior"
    );
}

#[test]
fn smart_case_stays_sensitive_when_the_query_has_uppercase() {
    let dir = case_fixture();
    let mut smart = request("TODO_marker");
    smart.case = CaseMode::Smart;
    let result = run_grep(dir.path(), &smart).expect("grep");
    assert_eq!(
        result.total_matches, 0,
        "an uppercase literal in the query must pin case sensitivity"
    );
}

#[test]
fn insensitive_case_matches_regardless_of_query_case() {
    let dir = case_fixture();
    let mut insensitive = request("Todo_Marker");
    insensitive.case = CaseMode::Insensitive;
    let result = run_grep(dir.path(), &insensitive).expect("grep");
    assert_eq!(result.total_matches, 1);
}

#[test]
fn word_boundaries_exclude_substring_hits() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("ids.rs"),
        "let marker_id = 2;\nlet id = 3;\n",
    )
    .unwrap();

    let mut worded = request("id");
    worded.word = true;
    let result = run_grep(dir.path(), &worded).expect("grep");
    let lines: Vec<usize> = result.files[0]
        .matches
        .iter()
        .map(|line_match| line_match.line_number)
        .collect();
    assert_eq!(
        lines,
        vec![2],
        "word=true must skip the `marker_id` substring"
    );

    if let Some(oracle) = rg_oracle_paths(dir.path(), &["-w"], "id") {
        assert_eq!(
            matched_paths(&result),
            oracle,
            "our word-boundary file set disagreed with the installed ripgrep"
        );
    }

    let plain = run_grep(dir.path(), &request("id")).expect("grep");
    assert_eq!(
        plain.total_matches, 2,
        "without word=true the substring still matches"
    );
}

#[test]
fn context_lines_surround_matches_without_duplication() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Matches on lines 1 and 3 share line 2 as context.
    std::fs::write(
        dir.path().join("gap.rs"),
        "let marker_a = 1;\nlet shared_middle = 2;\nlet marker_b = 3;\n",
    )
    .unwrap();

    let plain = run_grep(dir.path(), &request("marker_")).expect("grep");
    let mut with_context = request("marker_");
    with_context.context_lines = 1;
    let result = run_grep(dir.path(), &with_context).expect("grep");

    assert_eq!(
        result.total_matches, plain.total_matches,
        "context lines must not be counted as matches"
    );

    let rendered = render(&result, &with_context, Some(200));
    assert_eq!(
        rendered.matches("        ~ @ 2 ").count(),
        1,
        "the shared context line must render exactly once:\n{rendered}"
    );
    assert!(
        rendered.contains("let shared_middle = 2;"),
        "context text must be rendered:\n{rendered}"
    );
    assert_eq!(
        render(&result, &request("marker_"), Some(200))
            .matches("        ~ @ ")
            .count(),
        0,
        "context is only rendered when the request asked for it"
    );
}

#[test]
fn multiline_regex_matches_across_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("span.rs"),
        "fn alpha() {\n    let marker = 1;\n}\n",
    )
    .unwrap();

    let mut multi = request(r"alpha\(\)[\s\S]*?let marker");
    multi.base.regex = true;
    multi.multiline = true;
    let result = run_grep(dir.path(), &multi).expect("grep");
    assert_eq!(result.total_matches, 1);
    let line_match = &result.files[0].matches[0];
    assert!(
        line_match.line_text.contains(" ⏎ "),
        "a block spanning lines must be joined onto one rendered line: {:?}",
        line_match.line_text
    );
    assert_eq!(line_match.line_span, 2, "the match covers two lines");

    let mut single = multi.clone();
    single.multiline = false;
    let single_result = run_grep(dir.path(), &single).expect("grep");
    assert_eq!(
        single_result.total_matches, 0,
        "without multiline the newline is stripped from the class, so nothing matches"
    );
}

#[test]
fn literal_newline_pattern_is_rejected_without_multiline() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), "foo\nbar\n").unwrap();

    let mut single = request("foo\nbar");
    single.base.regex = true;
    let err = run_grep(dir.path(), &single).expect_err("a literal newline must be rejected");
    assert!(
        err.contains("is not allowed in a regex"),
        "the error should name the cause: {err}"
    );

    let mut multi = request("foo\nbar");
    multi.base.regex = true;
    multi.multiline = true;
    let result = run_grep(dir.path(), &multi).expect("multiline accepts a literal newline");
    assert_eq!(result.total_matches, 1);
}

#[test]
fn per_file_cap_stops_the_search_and_reports_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = (0..30)
        .map(|idx| format!("fn f_{idx}() {{ marker }}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("many.rs"), body).unwrap();

    let mut capped = request("marker");
    capped.max_matches_per_file = 10;
    let result = run_grep(dir.path(), &capped).expect("grep");
    assert_eq!(result.files[0].matches.len(), 10);
    assert!(result.files[0].matches_truncated);
    let rendered = render(&result, &capped, Some(200));
    assert!(
        rendered.contains("per-file cap of 10 matches reached"),
        "truncation must be visible in the output:\n{rendered}"
    );

    // A file with exactly `cap` matches is not truncated: the searcher is asked
    // for one past the cap precisely so these two cases stay distinguishable.
    let exact_dir = tempfile::tempdir().expect("tempdir");
    let exact_body = (0..10)
        .map(|idx| format!("fn g_{idx}() {{ marker }}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(exact_dir.path().join("ten.rs"), exact_body).unwrap();
    let exact = run_grep(exact_dir.path(), &capped).expect("grep");
    assert_eq!(exact.files[0].matches.len(), 10);
    assert!(!exact.files[0].matches_truncated);
}

/// A file with the shapes Rust's `regex` cannot express: a value only
/// identifiable by what follows it, and a repeated word.
fn lookaround_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("cfg.rs"),
        "let marker_keep = 1;\nlet marker_drop = 2;\nlet the the duplicate = 3;\n",
    )
    .unwrap();
    dir
}

#[test]
fn pcre2_engine_supports_lookaround() {
    let dir = lookaround_fixture();
    let mut lookahead = request(r"marker_\w+(?=\s*=\s*1)");
    lookahead.base.regex = true;
    lookahead.engine = EngineMode::Pcre2;

    let result = run_grep(dir.path(), &lookahead).expect("pcre2 must take a look-ahead");
    assert_eq!(result.total_matches, 1);
    assert_eq!(
        result.files[0].matches[0].line_number, 1,
        "only the `= 1` line qualifies"
    );
}

#[test]
fn pcre2_engine_supports_backreferences() {
    let dir = lookaround_fixture();
    let mut backref = request(r"\b(\w+) \1\b");
    backref.base.regex = true;
    backref.engine = EngineMode::Pcre2;

    let result = run_grep(dir.path(), &backref).expect("backreference must compile");
    assert_eq!(result.total_matches, 1);
    assert_eq!(result.files[0].matches[0].line_number, 3);

    // The same pattern under the default engine is an error, not a silent
    // engine change.
    let mut defaulted = backref.clone();
    defaulted.engine = EngineMode::Rust;
    assert!(
        run_grep(dir.path(), &defaulted)
            .expect_err("the default engine must refuse a backreference")
            .contains("backreference")
    );
}

#[test]
fn the_default_engine_refuses_lookaround_and_names_the_escape_hatch() {
    let dir = lookaround_fixture();
    let mut pinned = request(r"marker_\w+(?=\s*=\s*1)");
    pinned.base.regex = true;
    pinned.engine = EngineMode::Rust;

    let err = run_grep(dir.path(), &pinned).expect_err("the Rust engine must refuse");
    assert!(
        err.contains("look-around"),
        "the error must say what the engine refused: {err}"
    );
    assert!(
        err.contains(r#"engine="pcre2""#),
        "a refusal must name the escape hatch, or engine=rust is a dead end: {err}"
    );
}

#[test]
fn only_unsupported_feature_patterns_advertise_pcre2() {
    // The hint keys on the parser's structured error kind, not the rendered
    // message, and covers exactly the two features PCRE2 adds. A pattern that
    // is merely invalid must not be blamed on the engine: PCRE2 would often
    // accept it and then match nothing.
    for pattern in [
        r"marker(?=_tail)",
        r"marker(?!_tail)",
        r"(?<=let )marker",
        r"(?<!let )marker",
        r"(\w+) \1",
    ] {
        assert!(
            pcre2_can_take_over(pattern),
            "{pattern:?} is a feature PCRE2 has and Rust's regex does not"
        );
    }

    for pattern in [
        "marker",         // valid: nothing to point anywhere
        r"marker(",       // malformed
        r"marker[a-",     // malformed
        "marker\nkeep",   // valid syntax; rejected later by the line terminator
        r"marker(?P<>x)", // malformed capture name
    ] {
        assert!(
            !pcre2_can_take_over(pattern),
            "{pattern:?} must not be advertised as a PCRE2 case"
        );
    }
}

#[test]
fn malformed_patterns_stay_errors_without_mentioning_pcre2() {
    // PCRE2 accepts `marker_(` no more than Rust does, but it *would* accept a
    // literal newline and then match nothing. Neither may be redirected there.
    let dir = lookaround_fixture();
    for pattern in [r"marker_(", "marker\nkeep"] {
        let mut bad = request(pattern);
        bad.base.regex = true;
        let err = run_grep(dir.path(), &bad).expect_err("must not be silently accepted");
        assert!(
            !err.contains("pcre2"),
            "a malformed pattern must not be blamed on the engine: {err}"
        );
    }
}

#[test]
fn pcre2_and_rust_engines_agree_on_a_plain_query() {
    let dir = fixture();
    let rust = {
        let mut req = request("marker");
        req.engine = EngineMode::Rust;
        req
    };
    let pcre2 = {
        let mut req = request("marker");
        req.engine = EngineMode::Pcre2;
        req
    };

    let rust_result = run_grep(dir.path(), &rust).expect("rust");
    let pcre2_result = run_grep(dir.path(), &pcre2).expect("pcre2");
    assert_eq!(pcre2_result.total_matches, rust_result.total_matches);
    assert_eq!(matched_paths(&pcre2_result), matched_paths(&rust_result));
    assert_eq!(
        render(&pcre2_result, &pcre2, Some(200)),
        render(&rust_result, &rust, Some(200)),
        "the two engines must render a plain query identically"
    );
}

#[test]
fn pcre2_engine_honors_case_word_and_per_file_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("ids.rs"),
        "let MARKER_id = 1;\nlet id = 2;\nlet id2 = 3;\n",
    )
    .unwrap();

    // A literal query under PCRE2 goes through `fixed_strings`, so regex meta
    // characters in it stay literal.
    let mut literal = request("MARKER_id = 1;");
    literal.engine = EngineMode::Pcre2;
    assert_eq!(
        run_grep(dir.path(), &literal).expect("pcre2").total_matches,
        1
    );

    let mut insensitive = request("marker_ID");
    insensitive.engine = EngineMode::Pcre2;
    insensitive.case = CaseMode::Insensitive;
    assert_eq!(
        run_grep(dir.path(), &insensitive)
            .expect("pcre2")
            .total_matches,
        1
    );

    let mut worded = request("id");
    worded.engine = EngineMode::Pcre2;
    worded.word = true;
    let worded_result = run_grep(dir.path(), &worded).expect("pcre2");
    assert_eq!(
        worded_result.files[0]
            .matches
            .iter()
            .map(|line_match| line_match.line_number)
            .collect::<Vec<_>>(),
        vec![2],
        "word=true must skip MARKER_id and id2 under PCRE2 too"
    );

    let mut capped = request("id");
    capped.engine = EngineMode::Pcre2;
    capped.max_matches_per_file = 1;
    let capped_result = run_grep(dir.path(), &capped).expect("pcre2");
    assert_eq!(capped_result.files[0].matches.len(), 1);
    assert!(capped_result.files[0].matches_truncated);
}
