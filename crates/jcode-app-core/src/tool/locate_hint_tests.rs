//! Tests for nearest-match hints on failed edit locates.

use super::*;

const FILE: &str = "\
fn build_matcher(request: &GrepRequest) -> Result<RegexMatcher, String> {
    let pattern = rust_pattern(request);
    let mut builder = RegexMatcherBuilder::new();
    builder.word(request.word);
    builder.build(&pattern)
}

fn unrelated() {
    println!(\"nothing to see\");
}
";

#[test]
fn a_renamed_symbol_is_located_with_its_line_number() {
    // The caller's copy is stale: the parameter was renamed since they read it.
    let hint = closest_match(
        FILE,
        "fn build_matcher(args: &GrepRequest) -> Result<RegexMatcher, String> {",
    )
    .expect("a one-token drift must be reported");

    assert_eq!(hint.line, 1);
    assert!(
        hint.ratio > 0.9,
        "a single renamed identifier should score high, got {}",
        hint.ratio
    );
    assert!(
        hint.excerpt
            .contains("fn build_matcher(request: &GrepRequest)"),
        "the excerpt must quote the file's actual text so it can be copied: {}",
        hint.excerpt
    );
    assert!(
        hint.describe().contains("at line 1"),
        "the message must carry the location: {}",
        hint.describe()
    );
}

#[test]
fn a_multi_line_needle_reports_the_start_of_the_region() {
    let hint = closest_match(
        FILE,
        "    let pattern = rust_pattern(req);\n    let mut builder = RegexMatcherBuilder::new();",
    )
    .expect("a two-line drift must be reported");

    assert_eq!(
        hint.line, 2,
        "the hint must point at the first line of the matched region, not the anchor line"
    );
}

#[test]
fn the_anchor_may_sit_anywhere_inside_the_needle() {
    // The distinctive line is the *second* one here; a hint anchored on it
    // without correcting for the offset would report line 4 instead of 3.
    let hint = closest_match(
        FILE,
        "    let mut bldr = RegexMatcherBuilder::new();\n    builder.word(request.wordy);",
    )
    .expect("hint");

    assert_eq!(hint.line, 3);
}

#[test]
fn an_absent_target_yields_no_hint_rather_than_a_wrong_one() {
    assert_eq!(
        closest_match(FILE, "impl Iterator for CompletelyDifferentThing {"),
        None,
        "pointing at an unrelated line would send the caller to the wrong place"
    );
}

#[test]
fn needles_made_only_of_punctuation_yield_no_hint() {
    // `}` matches everywhere, so any "closest match" would be arbitrary.
    assert_eq!(closest_match(FILE, "}"), None);
    assert_eq!(closest_match(FILE, "}\n"), None);
}

#[test]
fn empty_inputs_are_not_a_panic() {
    assert_eq!(closest_match("", "anything"), None);
    assert_eq!(closest_match(FILE, ""), None);
}

#[test]
fn a_long_line_is_clipped_in_the_excerpt() {
    let long = format!("    let data = \"{}\";", "x".repeat(400));
    let file = format!("fn f() {{\n{long}\n}}\n");
    let hint = closest_match(&file, &long.replace("let data", "let payload")).expect("hint");

    let quoted = hint
        .excerpt
        .lines()
        .next()
        .expect("at least one quoted line");
    assert!(
        quoted.chars().count() < 200,
        "a data line must not fill the reply: {} chars",
        quoted.chars().count()
    );
    assert!(
        quoted.ends_with(" ..."),
        "clipping must be visible: {quoted}"
    );
}

#[test]
fn work_is_bounded_when_the_anchor_repeats_throughout_the_file() {
    // 5,000 identical candidate lines: the cap must keep this from turning a
    // failed edit into a slow failed edit.
    let body = "    let value = compute(input);\n".repeat(5_000);
    let file = format!("fn f() {{\n{body}}}\n");
    let start = std::time::Instant::now();
    let hint = closest_match(&file, "    let value = compute(inputs);");
    let elapsed = start.elapsed();

    assert!(
        hint.is_some(),
        "the repeated line is still the closest match"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "hint search took {elapsed:?}; the candidate cap is not holding"
    );
}
