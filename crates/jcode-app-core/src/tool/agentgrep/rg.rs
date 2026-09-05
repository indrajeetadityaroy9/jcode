//! Grep mode backed by ripgrep's own engine, linked as a library.
//!
//! The `agentgrep` crate's `run_grep` shells out to an `rg` binary and, when
//! the binary is missing, silently falls back to a slower hand-rolled walker.
//! Two searches over the same tree could therefore disagree on which files are
//! visible depending only on whether Homebrew happened to have installed
//! ripgrep, and nothing in the output said which engine had answered.
//!
//! This module removes both problems by owning grep mode: file discovery runs
//! through `ignore` (ripgrep's walker) and matching runs through
//! `grep-searcher`. No subprocess, no `PATH` lookup, no silent walker swap.
//!
//! Two matchers are wired in: `grep-regex` (Rust's `regex`, linear time, the
//! default) and `grep-pcre2` (backtracking, with look-around and
//! backreferences). Which one runs is the caller's explicit choice and nothing
//! here infers it — that is the whole point, given what this module replaced.
//! A Rust-engine refusal PCRE2 could take names `engine="pcre2"` in its error,
//! so the second engine costs one retry to reach and never runs unasked.
//!
//! `MatchGroup`'s `match_indices` field is private upstream and the struct has
//! no public constructor, so a caller that produces its own matches cannot
//! build a grouped `agentgrep::search::GrepResult` and cannot reuse
//! `agentgrep::render::render_grep_output`. Grouping and rendering are
//! therefore reimplemented here against the same layout. The pieces upstream
//! does expose are reused rather than duplicated:
//! `structure::extract_file_structure` for symbol extraction and
//! `render::compact_rendered_match_line` for per-line truncation.

use ::agentgrep::cli::GrepArgs;
use ::agentgrep::render::compact_rendered_match_line;
use ::agentgrep::structure::{StructureItem, extract_file_structure};
use ::agentgrep::workspace::{normalize_display_path, normalize_file_type};
use globset::{Glob, GlobSetBuilder};
use grep_matcher::Matcher;
use grep_pcre2::{RegexMatcher as Pcre2Matcher, RegexMatcherBuilder as Pcre2MatcherBuilder};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch,
};
use ignore::{WalkBuilder, WalkState};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

/// Matches at or above this count skip structure extraction entirely: reading
/// and parsing a file that matched hundreds of times costs more than the
/// grouping is worth. Mirrors the upstream threshold so dense output is
/// unchanged.
const DENSE_MATCH_SKIP_STRUCTURE_THRESHOLD: usize = 24;
/// Above this count grouping stays on but is capped, so a dense file cannot
/// crowd out every other file in the rendered output.
const DENSE_MATCH_LIMITED_GROUPING_THRESHOLD: usize = 12;
const DENSE_GROUPS_LIMIT: usize = 8;
const DENSE_OTHER_SYMBOLS_LIMIT: usize = 2;
const OTHER_SYMBOLS_LIMIT: usize = 4;
/// Stored match lines are capped before rendering so a minified bundle cannot
/// put a megabyte-long line into the result set.
const MAX_MATCH_LINE_CHARS: usize = 240;
const MATCH_LINE_PREFIX_CONTEXT_CHARS: usize = 80;
const MAX_NON_CODE_MATCH_LINES_PER_FILE: usize = 3;

/// Per-file match cap. Collecting every hit in a large data file costs time and
/// memory for matches the renderer then discards.
pub(super) const DEFAULT_MAX_MATCHES_PER_FILE: usize = 1000;

/// How the query's letter case is treated. `Smart` is ripgrep's `-S`: a pattern
/// whose literals are all lowercase matches case-insensitively, and a pattern
/// containing any uppercase literal stays case-sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CaseMode {
    Sensitive,
    Insensitive,
    #[default]
    Smart,
}

/// Which regex engine compiles the pattern.
///
/// Rust's `regex` is the default: linear time, no backtracking. PCRE2 is
/// opt-in for the two features the Rust engine refuses by design, look-around
/// and backreferences. The choice is always the caller's — a Rust-engine
/// refusal that PCRE2 could take names `engine="pcre2"` in its error, so
/// nothing here switches engines on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EngineMode {
    #[default]
    Rust,
    Pcre2,
}

/// A grep-mode search: upstream's argument struct plus the ripgrep engine
/// options the tool schema exposes on top of it.
#[derive(Debug, Clone)]
pub(super) struct GrepRequest {
    pub base: GrepArgs,
    pub case: CaseMode,
    pub word: bool,
    pub multiline: bool,
    /// Lines of context on each side of a match; already clamped by the caller.
    pub context_lines: usize,
    /// Per-file match cap enforced by the searcher itself.
    pub max_matches_per_file: usize,
    /// Regex engine; the caller's choice, never inferred.
    pub engine: EngineMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LineMatch {
    pub line_number: usize,
    pub line_text: String,
    /// Lines the match covers; always 1 unless a multiline match spans several.
    pub line_span: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MatchGroup {
    pub kind: String,
    pub label: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub match_indices: Vec<usize>,
}

impl MatchGroup {
    fn file_scope(match_indices: Vec<usize>) -> Self {
        Self {
            kind: "file-scope".to_string(),
            label: "<file scope>".to_string(),
            start_line: None,
            end_line: None,
            match_indices,
        }
    }

    fn resolved_matches<'a>(
        &'a self,
        matches: &'a [LineMatch],
    ) -> impl Iterator<Item = &'a LineMatch> + 'a {
        self.match_indices
            .iter()
            .filter_map(move |index| matches.get(*index))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileMatches {
    pub path: String,
    pub language: String,
    pub role: String,
    pub matches: Vec<LineMatch>,
    pub groups: Vec<MatchGroup>,
    pub total_symbols: usize,
    pub matched_symbol_count: usize,
    pub other_symbols: Vec<StructureItem>,
    pub other_symbols_omitted_count: usize,
    /// Context lines keyed by line number; empty unless context was requested.
    pub context: BTreeMap<usize, String>,
    /// Whether the per-file match cap stopped the search early.
    pub matches_truncated: bool,
}

/// Raw per-file search output, before symbol grouping.
struct FileHits {
    matches: Vec<LineMatch>,
    context: BTreeMap<usize, String>,
    truncated: bool,
}

/// Collects matches and, when context is requested, the surrounding lines.
///
/// `grep_searcher::sinks::UTF8` implements only `Sink::matched`, so it silently
/// drops the context lines the searcher reports; this sink keeps both.
struct MatchSink<'a, M> {
    matcher: &'a M,
    multiline: bool,
    matches: Vec<LineMatch>,
    context: BTreeMap<usize, String>,
}

impl<M: Matcher> Sink for MatchSink<'_, M> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        let bytes = mat.bytes();
        let raw = String::from_utf8_lossy(bytes);
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        // A multiline match arrives as a block. Join it onto one line so the
        // renderer's line-oriented layout still holds.
        let text = if self.multiline && trimmed.contains('\n') {
            trimmed
                .split('\n')
                .map(|line| line.trim_end_matches('\r'))
                .collect::<Vec<_>>()
                .join(" ⏎ ")
        } else {
            trimmed.to_string()
        };
        // Ask the matcher where the hit is so a truncated line keeps context
        // around the match. A matcher error here can only mean the engine gave
        // up on this one line, in which case truncation falls back to the head.
        let span = match self.matcher.find(text.as_bytes()) {
            Ok(Some(found)) => Some((found.start(), found.end())),
            Ok(None) | Err(_) => None,
        };
        let line_span = bytes.iter().filter(|byte| **byte == b'\n').count()
            + usize::from(!bytes.ends_with(b"\n"));
        self.matches.push(LineMatch {
            line_number: mat.line_number().unwrap_or(0) as usize,
            line_text: compact_match_line(&text, span),
            line_span: line_span.max(1),
        });
        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        // Keyed by line number, so a line that is context for two nearby
        // matches is stored once. `kind()` is not needed: before/after is
        // implied by position relative to the match being rendered.
        let raw = String::from_utf8_lossy(context.bytes());
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        self.context.insert(
            context.line_number().unwrap_or(0) as usize,
            compact_match_line(trimmed, None),
        );
        Ok(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GrepResult {
    pub query: String,
    pub root: String,
    pub files: Vec<FileMatches>,
    pub total_files: usize,
    pub total_matches: usize,
}

/// Search `root` with ripgrep's engine.
///
/// Semantics follow `rg`'s defaults: ignore files are honored (`.gitignore`
/// only inside a git repository, matching `rg`'s `--no-require-git` default),
/// hidden entries are skipped, binary files quit early, and the query is a
/// literal unless `request.base.regex` is set.
pub(super) fn run_grep(root: &Path, request: &GrepRequest) -> Result<GrepResult, String> {
    match request.engine {
        EngineMode::Rust => search_with_matcher(root, request, build_rust_matcher(request)?),
        EngineMode::Pcre2 => search_with_matcher(root, request, build_pcre2_matcher(request)?),
    }
}

/// The search itself, generic over which of ripgrep's two matchers compiled the
/// pattern. `grep-searcher` is generic over `Matcher`, so nothing below this
/// point knows or cares which engine is running.
fn search_with_matcher<M>(
    root: &Path,
    request: &GrepRequest,
    matcher: M,
) -> Result<GrepResult, String>
where
    M: Matcher + Clone + Send + Sync,
{
    let glob_set = build_glob_set(&request.base)?;
    let file_type = request
        .base
        .file_type
        .as_deref()
        .map(normalize_file_type)
        .filter(|ext| !ext.is_empty());

    // Search in parallel the way ripgrep does. A single-threaded walk measured
    // ~4x slower than the `rg` subprocess it replaced on this repo, which would
    // have made "use ripgrep's engine" a regression; `build_parallel` closes
    // that gap because the walker and the per-file search share the thread pool.
    let hits_by_path: Mutex<BTreeMap<String, FileHits>> = Mutex::new(BTreeMap::new());
    let collected = &hits_by_path;
    let match_cap = request.max_matches_per_file;
    build_walker(root, &request.base).build_parallel().run(|| {
        let matcher = matcher.clone();
        let glob_set = glob_set.clone();
        let file_type = file_type.clone();
        let multiline = request.multiline;
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .before_context(request.context_lines)
            .after_context(request.context_lines)
            .multi_line(request.multiline)
            // One past the cap, so hitting it is distinguishable from a file
            // that happens to contain exactly `cap` matches.
            .max_matches(Some(match_cap as u64 + 1))
            .build();

        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.path();
            if !passes_filters(root, path, glob_set.as_ref(), file_type.as_deref()) {
                return WalkState::Continue;
            }

            let mut sink = MatchSink {
                matcher: &matcher,
                multiline,
                matches: Vec::new(),
                context: BTreeMap::new(),
            };
            let outcome = searcher.search_path(&matcher, path, &mut sink);
            // A single unreadable or non-UTF-8 file must not fail the whole
            // search; ripgrep skips it and continues, so mirror that.
            if outcome.is_err() || sink.matches.is_empty() {
                return WalkState::Continue;
            }

            let truncated = sink.matches.len() > match_cap;
            if truncated {
                sink.matches.truncate(match_cap);
            }
            collected
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    normalize_display_path(root, path),
                    FileHits {
                        matches: sink.matches,
                        context: sink.context,
                        truncated,
                    },
                );
            WalkState::Continue
        })
    });

    let matched = hits_by_path
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .into_iter()
        .collect::<Vec<_>>();

    // Structure extraction reads and parses each matched file, so it dominates
    // once a query touches many files. Fan it out too, keeping the BTreeMap's
    // path ordering by reassembling in index order.
    let files = enrich_files(root, matched)?;
    let total_matches = files.iter().map(|file| file.matches.len()).sum();

    Ok(GrepResult {
        query: request.base.query.clone(),
        root: root.to_string_lossy().into_owned(),
        total_files: files.len(),
        total_matches,
        files,
    })
}

/// Turn raw per-file matches into rendered-ready `FileMatches`, in parallel.
///
/// A worker panic is surfaced rather than absorbed: silently returning an
/// empty chunk would drop matched files from the result and make the search
/// look like it found less than it did.
fn enrich_files(
    root: &Path,
    matched: Vec<(String, FileHits)>,
) -> Result<Vec<FileMatches>, String> {
    let worker_count = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
        .min(matched.len().max(1));

    if worker_count <= 1 || matched.len() <= 8 {
        return Ok(matched
            .into_iter()
            .map(|(path, hits)| build_file_matches(root, path, hits))
            .collect());
    }

    let chunk_size = matched.len().div_ceil(worker_count);
    let mut chunks: Vec<Vec<(String, FileHits)>> = Vec::new();
    let mut current: Vec<(String, FileHits)> = Vec::with_capacity(chunk_size);
    for entry in matched {
        current.push(entry);
        if current.len() == chunk_size {
            chunks.push(std::mem::replace(
                &mut current,
                Vec::with_capacity(chunk_size),
            ));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    let mut per_chunk: Vec<Vec<FileMatches>> = Vec::with_capacity(chunks.len());

    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .into_iter()
                        .map(|(path, hits)| build_file_matches(root, path, hits))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(files) => per_chunk.push(files),
                Err(_) => return Err("a search worker panicked while grouping matches".to_string()),
            }
        }
        Ok(())
    })?;

    Ok(per_chunk.into_iter().flatten().collect())
}

/// Whether PCRE2 could run a pattern the Rust engine refused, i.e. whether the
/// refusal should tell the caller about `engine="pcre2"`.
///
/// `grep_regex::Error` flattens every syntax failure into one
/// `ErrorKind::Regex(String)` variant, so the engine's own error carries no
/// machine-readable cause. Re-parsing the pattern with `regex-syntax` — the
/// same parser the Rust engine uses, already a direct dependency for `escape`
/// — recovers the structured kind. Both arms are unsupported-*feature* errors,
/// never malformed input: pointing a caller at PCRE2 for a pattern that is
/// simply invalid would send them to an engine that often *accepts* it and
/// then matches nothing, turning a clear error into a silent empty result.
fn pcre2_can_take_over(pattern: &str) -> bool {
    use regex_syntax::ast::ErrorKind;

    match regex_syntax::ast::parse::Parser::new().parse(pattern) {
        Ok(_) => false,
        Err(err) => matches!(
            err.kind(),
            ErrorKind::UnsupportedLookAround | ErrorKind::UnsupportedBackreference
        ),
    }
}

/// The pattern handed to the Rust engine: the query verbatim in regex mode,
/// escaped otherwise.
fn rust_pattern(request: &GrepRequest) -> String {
    if request.base.regex {
        request.base.query.clone()
    } else {
        regex_syntax::escape(&request.base.query)
    }
}

/// PCRE2 has no linear-time guarantee: a pathological pattern can backtrack for
/// a long time on one line. The per-file match cap bounds how many matches are
/// collected, not how long a single match attempt runs, which is why PCRE2 is
/// opt-in rather than the default.
fn build_pcre2_matcher(request: &GrepRequest) -> Result<Pcre2Matcher, String> {
    let mut builder = Pcre2MatcherBuilder::new();
    // `fixed_strings` escapes the pattern with PCRE2's own rules, which is why
    // the literal path does not reuse `regex_syntax::escape` here.
    builder.fixed_strings(!request.base.regex);
    match request.case {
        CaseMode::Sensitive => {}
        CaseMode::Insensitive => {
            builder.caseless(true);
        }
        CaseMode::Smart => {
            builder.case_smart(true);
        }
    }
    builder.word(request.word);
    // Match the Rust engine's Unicode semantics for `\w`, `\b`, `\d` and `.`;
    // `ucp` implies UTF mode. JIT is a pure speedup, so take it when the build
    // of PCRE2 we linked has it and carry on when it does not.
    builder.ucp(true).utf(true).jit_if_available(true);
    if request.multiline {
        builder.multi_line(true);
    }

    builder
        .build(&request.base.query)
        .map_err(|err| format!("invalid search pattern: {err}"))
}

fn build_rust_matcher(request: &GrepRequest) -> Result<RegexMatcher, String> {
    let pattern = rust_pattern(request);

    let mut builder = RegexMatcherBuilder::new();
    match request.case {
        CaseMode::Sensitive => {}
        CaseMode::Insensitive => {
            builder.case_insensitive(true);
        }
        CaseMode::Smart => {
            builder.case_smart(true);
        }
    }
    builder.word(request.word);

    if request.multiline {
        // Pinning a line terminator tells the searcher the matcher can never
        // match across one, and `Searcher::multi_line_with_matcher` then
        // silently downgrades to single-line search. Leave it unset here, and
        // enable the regex `m` flag so `^`/`$` anchor to lines.
        builder.multi_line(true);
    } else {
        builder.line_terminator(Some(b'\n'));
    }

    builder.build(&pattern).map_err(|err| {
        // A caller who pinned `engine: "rust"`, or who hit this under `Auto`
        // for a pattern the parser could not classify, would otherwise have to
        // know the second engine exists. Say so, once, on the failure itself.
        if pcre2_can_take_over(&pattern) {
            format!("invalid search pattern: {err}; retry with engine=\"pcre2\"")
        } else {
            format!("invalid search pattern: {err}")
        }
    })
}

fn build_glob_set(args: &GrepArgs) -> Result<Option<globset::GlobSet>, String> {
    let Some(pattern) = args.glob.as_deref().filter(|glob| !glob.is_empty()) else {
        return Ok(None);
    };
    let glob =
        Glob::new(pattern).map_err(|err| format!("invalid glob {pattern:?}: {err}"))?;
    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    builder
        .build()
        .map(Some)
        .map_err(|err| format!("invalid glob {pattern:?}: {err}"))
}

fn build_walker(root: &Path, args: &GrepArgs) -> WalkBuilder {
    let respect_ignores = !args.no_ignore;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!args.hidden)
        .git_ignore(respect_ignores)
        .git_global(respect_ignores)
        .git_exclude(respect_ignores)
        .ignore(respect_ignores)
        .parents(respect_ignores)
        .follow_links(false);
    builder
}

fn passes_filters(
    root: &Path,
    path: &Path,
    glob_set: Option<&globset::GlobSet>,
    file_type: Option<&str>,
) -> bool {
    if let Some(ext) = file_type
        && path.extension().and_then(|ext| ext.to_str()) != Some(ext)
    {
        return false;
    }
    let Some(glob_set) = glob_set else {
        return true;
    };
    // Globs are matched against the display path and the bare file name so both
    // `src/**/*.rs` and `*.rs` behave the way callers expect.
    let display = normalize_display_path(root, path);
    if glob_set.is_match(&display) {
        return true;
    }
    path.file_name()
        .map(|name| glob_set.is_match(name.to_string_lossy().as_ref()))
        .unwrap_or(false)
}

/// Attach language, role and symbol grouping to one file's matches.
///
/// Structure extraction needs a second read of the file. When that read fails
/// — a permissions change or a rewrite between the search and this pass — the
/// matches are still reported under file scope rather than the file being
/// dropped from the result, which would understate what the search found.
fn build_file_matches(root: &Path, path: String, hits: FileHits) -> FileMatches {
    let FileHits {
        matches,
        context,
        truncated,
    } = hits;
    let absolute_path = root.join(&path);

    let structure = if matches.len() >= DENSE_MATCH_SKIP_STRUCTURE_THRESHOLD {
        // Reading and parsing a file that matched dozens of times costs more
        // than the grouping is worth, so dense files skip structure entirely.
        None
    } else {
        std::fs::read_to_string(&absolute_path)
            .ok()
            .map(|text| extract_file_structure(&absolute_path, &path, &text))
    };

    let Some(structure) = structure else {
        return FileMatches {
            language: infer_language(&absolute_path),
            role: infer_role(&path),
            path,
            groups: vec![MatchGroup::file_scope((0..matches.len()).collect())],
            matches,
            total_symbols: 0,
            matched_symbol_count: 0,
            other_symbols: Vec::new(),
            other_symbols_omitted_count: 0,
            context,
            matches_truncated: truncated,
        };
    };

    let grouping = group_matches(&structure.items, &matches);
    FileMatches {
        path,
        language: structure.language,
        role: structure.role,
        matches,
        groups: grouping.groups,
        total_symbols: structure.items.len(),
        matched_symbol_count: grouping.matched_symbol_count,
        other_symbols: grouping.other_symbols,
        other_symbols_omitted_count: grouping.other_symbols_omitted_count,
        context,
        matches_truncated: truncated,
    }
}

struct Grouping {
    matched_symbol_count: usize,
    groups: Vec<MatchGroup>,
    other_symbols: Vec<StructureItem>,
    other_symbols_omitted_count: usize,
}

/// Bucket matches under their enclosing symbol.
///
/// `items` arrives sorted by start line, and matches arrive sorted by line
/// number, so a single forward cursor over `items` is enough: no per-match
/// scan of the symbol table.
fn group_matches(items: &[StructureItem], matches: &[LineMatch]) -> Grouping {
    let (max_groups, other_symbols_limit) = if matches.len() >= DENSE_MATCH_LIMITED_GROUPING_THRESHOLD
    {
        (DENSE_GROUPS_LIMIT, DENSE_OTHER_SYMBOLS_LIMIT)
    } else {
        (usize::MAX, OTHER_SYMBOLS_LIMIT)
    };

    let mut symbol_groups: Vec<MatchGroup> = Vec::new();
    let mut matched_indices: Vec<usize> = Vec::new();
    let mut file_scope_matches: Vec<usize> = Vec::new();
    let mut item_idx = 0usize;
    let mut last_grouped_item_idx: Option<usize> = None;

    for (match_idx, line_match) in matches.iter().enumerate() {
        while item_idx < items.len() && items[item_idx].end_line < line_match.line_number {
            item_idx += 1;
        }

        let enclosing = items.get(item_idx).filter(|item| {
            item.start_line <= line_match.line_number && line_match.line_number <= item.end_line
        });

        let Some(item) = enclosing else {
            file_scope_matches.push(match_idx);
            continue;
        };

        if matched_indices.last().copied() == Some(item_idx) {
            // `last_grouped_item_idx` is only set alongside a push into
            // `symbol_groups`, so the group is present; matching on it keeps
            // that invariant local instead of asserting it at runtime.
            match last_grouped_item_idx
                .filter(|idx| *idx == item_idx)
                .and_then(|_| symbol_groups.last_mut())
            {
                Some(group) => group.match_indices.push(match_idx),
                None => file_scope_matches.push(match_idx),
            }
            continue;
        }

        matched_indices.push(item_idx);
        if symbol_groups.len() < max_groups {
            symbol_groups.push(MatchGroup {
                kind: item.kind.clone(),
                label: item.label.clone(),
                start_line: Some(item.start_line),
                end_line: Some(item.end_line),
                match_indices: vec![match_idx],
            });
            last_grouped_item_idx = Some(item_idx);
        } else {
            file_scope_matches.push(match_idx);
            last_grouped_item_idx = None;
        }
    }

    let mut groups = Vec::with_capacity(symbol_groups.len() + 1);
    if !file_scope_matches.is_empty() {
        groups.push(MatchGroup::file_scope(file_scope_matches));
    }
    groups.extend(symbol_groups);

    let matched_symbol_count = matched_indices.len();
    let mut other_symbols = Vec::new();
    let mut other_symbols_omitted_count = 0usize;
    let mut matched_iter = matched_indices.into_iter().peekable();
    for (idx, item) in items.iter().enumerate() {
        if matched_iter.peek().copied() == Some(idx) {
            matched_iter.next();
            continue;
        }
        if other_symbols.len() < other_symbols_limit {
            other_symbols.push(item.clone());
        } else {
            other_symbols_omitted_count += 1;
        }
    }

    Grouping {
        matched_symbol_count,
        groups,
        other_symbols,
        other_symbols_omitted_count,
    }
}

fn infer_language(path: &Path) -> String {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => "rust",
        Some("ts" | "tsx") => "typescript",
        Some("js" | "jsx") => "javascript",
        Some("py") => "python",
        Some("md") => "markdown",
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        _ => "text",
    }
    .to_string()
}

fn infer_role(relative_path: &str) -> String {
    let path = relative_path.to_ascii_lowercase();
    if path.contains("/tests/") || path.contains("_test") || path.contains("test_") {
        "test"
    } else if path.contains("/docs/") || path.ends_with(".md") {
        "docs"
    } else if path.contains("/ui/") || path.contains("/tui/") || path.contains("view") {
        "ui"
    } else if path.contains("auth") {
        "auth"
    } else if path.contains("provider") {
        "provider"
    } else if path.contains("config") {
        "config"
    } else if path.contains("handler") || path.contains("router") {
        "handler"
    } else if path.contains("src/") {
        "implementation"
    } else {
        "other"
    }
    .to_string()
}

/// Cap a stored match line, keeping a window around the match so a single
/// minified or data-dump line cannot dominate the result set.
///
/// `match_span` is a byte range into `line`. The window keeps
/// `MATCH_LINE_PREFIX_CONTEXT_CHARS` of leading context before the hit and is
/// then clamped to `MAX_MATCH_LINE_CHARS`, so the match itself stays visible
/// even when it sits thousands of characters into the line.
fn compact_match_line(line: &str, match_span: Option<(usize, usize)>) -> String {
    let char_count = line.chars().count();
    if char_count <= MAX_MATCH_LINE_CHARS {
        return line.to_string();
    }

    let (match_start, match_end) = match_span.unwrap_or((0, 0));
    let match_start_char = line[..match_start.min(line.len())].chars().count();
    let match_end_char = line[..match_end.min(line.len())].chars().count();
    let match_len_chars = match_end_char.saturating_sub(match_start_char).max(1);

    let start_char = match_start_char.saturating_sub(MATCH_LINE_PREFIX_CONTEXT_CHARS);
    let mut end_char = start_char
        .saturating_add(MAX_MATCH_LINE_CHARS)
        .max(match_start_char.saturating_add(match_len_chars));
    if end_char > char_count {
        end_char = char_count;
    }
    let start_char = end_char.saturating_sub(MAX_MATCH_LINE_CHARS).min(start_char);

    let omitted_prefix = start_char;
    let omitted_suffix = char_count.saturating_sub(end_char);
    let snippet: String = line
        .chars()
        .skip(start_char)
        .take(end_char.saturating_sub(start_char))
        .collect();

    match (omitted_prefix > 0, omitted_suffix > 0) {
        (true, true) => format!(
            "…{snippet} … [truncated: {omitted_prefix} chars before, {omitted_suffix} chars after]"
        ),
        (true, false) => format!("…{snippet} [truncated: {omitted_prefix} chars before]"),
        (false, true) => format!("{snippet} … [truncated: {omitted_suffix} chars after]"),
        (false, false) => snippet,
    }
}

/// Restrict a result to one exact file, used when the caller named a single
/// file rather than a directory.
pub(super) fn filter_to_exact_file(result: GrepResult, exact_file: Option<&str>) -> GrepResult {
    let Some(exact_file) = exact_file else {
        return result;
    };
    let files: Vec<FileMatches> = result
        .files
        .into_iter()
        .filter(|file| file.path == exact_file)
        .collect();
    let total_matches = files.iter().map(|file| file.matches.len()).sum();
    GrepResult {
        total_files: files.len(),
        total_matches,
        files,
        ..result
    }
}

/// Render the result in the same layout the upstream renderer produces.
pub(super) fn render(
    result: &GrepResult,
    request: &GrepRequest,
    max_matches: Option<usize>,
) -> String {
    if request.base.paths_only {
        return result
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut lines = vec![
        format!("query: {}", result.query),
        format!(
            "matches: {} in {} files",
            result.total_matches, result.total_files
        ),
    ];

    let mut displayed_matches = 0usize;
    let limit_reached = |displayed: usize| max_matches.is_some_and(|max| displayed >= max);

    for file in &result.files {
        if limit_reached(displayed_matches) {
            break;
        }
        render_file(file, request, max_matches, &mut displayed_matches, &mut lines);
    }

    if let Some(max) = max_matches
        && result.total_matches > displayed_matches
    {
        lines.push(String::new());
        lines.push(format!(
            "... {} more matches omitted (max_regions={})",
            result.total_matches.saturating_sub(displayed_matches),
            max
        ));
    }

    lines.join("\n")
}

fn render_file(
    file: &FileMatches,
    request: &GrepRequest,
    max_matches: Option<usize>,
    displayed_matches: &mut usize,
    lines: &mut Vec<String>,
) {
    lines.push(String::new());
    lines.push(file.path.clone());
    if file.total_symbols > 0 {
        lines.push(format!(
            "  symbols: {} total, {} matched, {} other",
            file.total_symbols,
            file.matched_symbol_count,
            file.total_symbols.saturating_sub(file.matched_symbol_count)
        ));
    } else {
        lines.push("  symbols: no structural items detected".to_string());
    }

    let non_code_cap = non_code_match_cap(file);
    let mut file_displayed_matches = 0usize;

    // Lines a match itself occupies. A multiline match's own trailing lines can
    // arrive through `Sink::context`, because the searcher computes context
    // from the match's start line; skipping them avoids printing them twice.
    let covered: HashSet<usize> = if request.context_lines > 0 {
        file.matches
            .iter()
            .flat_map(|line_match| {
                line_match.line_number..line_match.line_number + line_match.line_span
            })
            .collect()
    } else {
        HashSet::new()
    };
    let mut printed_context: HashSet<usize> = HashSet::new();

    for group in &file.groups {
        let remaining_global = max_matches
            .map(|max| max.saturating_sub(*displayed_matches))
            .unwrap_or(usize::MAX);
        if remaining_global == 0 {
            break;
        }
        let remaining_file = non_code_cap
            .map(|cap| cap.saturating_sub(file_displayed_matches))
            .unwrap_or(usize::MAX);
        let remaining = remaining_global.min(remaining_file);
        if remaining == 0 {
            break;
        }

        let visible: Vec<&LineMatch> = group
            .resolved_matches(&file.matches)
            .take(remaining)
            .collect();
        if visible.is_empty() {
            continue;
        }

        match (group.start_line, group.end_line) {
            (Some(start_line), Some(end_line)) => lines.push(format!(
                "    - {} {} @ {}-{}",
                group.kind, group.label, start_line, end_line
            )),
            _ => lines.push(format!("    - {}", group.label)),
        }
        for line_match in visible {
            push_context_lines(
                file,
                request,
                line_match.line_number.saturating_sub(request.context_lines)
                    ..line_match.line_number,
                &covered,
                &mut printed_context,
                lines,
            );
            let line_text = compact_rendered_match_line(&line_match.line_text, &request.base);
            lines.push(format!(
                "      - @ {} {}",
                line_match.line_number, line_text
            ));
            file_displayed_matches += 1;
            *displayed_matches += 1;
            let after_start = line_match.line_number + line_match.line_span;
            push_context_lines(
                file,
                request,
                after_start..after_start + request.context_lines,
                &covered,
                &mut printed_context,
                lines,
            );
        }
    }

    let global_limit_reached = max_matches.is_some_and(|max| *displayed_matches >= max);
    if non_code_cap.is_some() && !global_limit_reached && file.matches.len() > file_displayed_matches
    {
        lines.push(format!(
            "    - ... {} more non-code matches omitted; narrow path/glob/type or use paths_only for full file list",
            file.matches.len().saturating_sub(file_displayed_matches)
        ));
    }

    if !file.other_symbols.is_empty() {
        let mut summary = file
            .other_symbols
            .iter()
            .map(|item| {
                format!(
                    "{} {} @ {}-{}",
                    item.kind, item.label, item.start_line, item.end_line
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if file.other_symbols_omitted_count > 0 {
            if !summary.is_empty() {
                summary.push_str("; ");
            }
            summary.push_str(&format!("... {} more", file.other_symbols_omitted_count));
        }
        lines.push(format!("    - other: {summary}"));
    }

    if file.matches_truncated {
        lines.push(format!(
            "    - ... per-file cap of {} matches reached; this file has more",
            file.matches.len()
        ));
    }
}

/// Emit stored context lines for `range`, skipping lines a match already
/// occupies and lines this file has printed for an earlier, nearby match.
///
/// Context lines deliberately do not count against the match caps: they are
/// framing for a match that was already counted.
fn push_context_lines(
    file: &FileMatches,
    request: &GrepRequest,
    range: std::ops::Range<usize>,
    covered: &HashSet<usize>,
    printed: &mut HashSet<usize>,
    lines: &mut Vec<String>,
) {
    if request.context_lines == 0 {
        return;
    }
    for line in range {
        if covered.contains(&line) || !printed.insert(line) {
            continue;
        }
        if let Some(text) = file.context.get(&line) {
            lines.push(format!("        ~ @ {line} {text}"));
        }
    }
}

/// Data-shaped files get a hard per-file match cap: a query hitting a large
/// JSON or markdown corpus would otherwise render every line.
fn non_code_match_cap(file: &FileMatches) -> Option<usize> {
    match file.language.as_str() {
        "json" | "yaml" | "markdown" | "text" | "" => Some(MAX_NON_CODE_MATCH_LINES_PER_FILE),
        _ => None,
    }
}

#[cfg(test)]
#[path = "rg_tests.rs"]
mod rg_tests;
