#!/usr/bin/env python3
"""Enforce a ratcheting budget for swallowed-error-like Rust patterns.

This is intentionally a broad guardrail. It tracks production occurrences of
patterns that commonly hide failures and should either be removed, logged,
propagated, or explicitly accepted as best-effort:

- `let _ = ...`
- `.ok()`
- `.unwrap_or_default()`

Policy:
- Existing files may not increase their count.
- New production files may not introduce these patterns.
- Total count may not increase.
- A single line may opt out with a justified `// budget-ok: <reason>` comment,
  which keeps the reason next to the code instead of absorbing it into an
  opaque number. An unexplained `// budget-ok` is not honoured.
- Shapes that discard no error are excluded outright (`NOT_AN_ERROR`), and the
  detector proves itself against `MUST_COUNT`/`MUST_IGNORE` on every run.
- `--update [PATH ...]` records current counts; naming paths avoids laundering
  unrelated drift. `--moved OLD=NEW` re-keys after a pure move, `--prune`
  retires dead entries, `--repair` re-derives the summary fields.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

import budget_common as bc

REPO_ROOT = Path(__file__).resolve().parent.parent
BASELINE_FILE = REPO_ROOT / "scripts" / "swallowed_error_budget.json"
SCAN_ROOTS = (REPO_ROOT / "src", REPO_ROOT / "crates")
PATTERNS = {
    "let_underscore": re.compile(r"\blet\s+_\s*="),
    "dot_ok": re.compile(r"\.ok\(\)"),
    "unwrap_or_default": re.compile(r"\.unwrap_or_default\(\)"),
}

#: Shapes that match a pattern above but discard no error at all, so counting
#: them only dilutes the signal. Each is here because this session's audits
#: judged every instance benign, and each is provably not an error path:
#:
#: - `env::var(..).ok()`: `VarError` is only NotPresent/NotUnicode, so an unset
#:   variable is an ordinary state and the `Option` is the intended shape.
#: - `<option-producing call>.unwrap_or_default()`: the receiver returns
#:   `Option`, not `Result` - `strip_prefix` guarded by `starts_with` accounted
#:   for 14 of the 19 hits in one TUI file alone.
#:
#: Deliberately NOT excluded: `let _ = tx.send(..)`. Its `SendError` only fires
#: once the receiver is gone, but events silently dropped because a consumer
#: died early is a real defect class, so those stay countable and are opted out
#: individually with `// budget-ok:` where a reviewer can see the reason.
NOT_AN_ERROR = (
    re.compile(r"(?:std::)?env::var(?:_os)?\s*\([^)]*\)\s*\.ok\(\)"),
    # Option-only receivers. `map`, `and_then`, `as_ref` and `as_deref` are
    # deliberately absent: they exist on `Result` too, so
    # `fs::read_to_string(p).map(..).unwrap_or_default()` is a real discarded
    # error and the first draft of this list wrongly excluded it. MUST_COUNT
    # pins that case.
    re.compile(
        r"\.(?:strip_prefix|strip_suffix|get|get_mut|first|last|next|next_back|find"
        r"|pop|front|back)\s*\([^;]*\)\s*\.unwrap_or_default\(\)"
    ),
)

#: What this detector must and must not count, proven on every run.
MUST_COUNT = (
    "let _ = std::fs::write(&path, body);",
    "let value = serde_json::from_str(text).ok();",
    "let items: Vec<Item> = serde_json::from_str(raw).unwrap_or_default();",
    "let _ = tx.send(ServerEvent::Pong { id }).await;",
    # `Result::map` then `unwrap_or_default` discards the io error.
    "let n = std::fs::read_to_string(&p).map(|s| s.len()).unwrap_or_default();",
)
MUST_IGNORE = (
    'let path = std::env::var("HOME").ok();',
    'let rest = trimmed.strip_prefix("/model").unwrap_or_default();',
    "let _ = tx.send(event); // budget-ok: receiver dropped means nobody is listening",
    "let value = compute(input);",
)


def counts_line(line: str) -> bool:
    """Whether this line contributes to the budget."""
    if bc.EXEMPTION_RE.search(line):
        return False
    if any(pattern.search(line) for pattern in NOT_AN_ERROR):
        return False
    return any(pattern.search(line) for pattern in PATTERNS.values())

CFG_TEST_RE = re.compile(r"^\s*#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?test\s*[,)]")
ITEM_START_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:mod|fn)\b")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    bc.add_ledger_args(parser)
    return parser.parse_args()


def is_test_rust_file(path: Path) -> bool:
    rel = path.relative_to(REPO_ROOT).as_posix()
    if path.suffix != ".rs":
        return False
    parts = rel.split("/")
    if parts[0] == "tests" or any(
        part == "tests" or part.endswith("_tests") or part.endswith("_test") or part.startswith("tests_")
        for part in parts
    ):
        return True
    name = path.name
    return (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or name.endswith("_test.rs")
        or name.startswith("tests_")
    )


def production_rust_files() -> list[Path]:
    files: list[Path] = []
    for root in SCAN_ROOTS:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            if path.suffix == ".rs" and not is_test_rust_file(path):
                files.append(path)
    return files


def brace_delta(line: str) -> int:
    return line.count("{") - line.count("}")


def production_lines(path: Path) -> list[str]:
    lines = path.read_text(encoding="utf-8", errors="ignore").splitlines()
    output: list[str] = []
    skip_stack: list[int] = []
    pending_cfg_test = False

    for line in lines:
        stripped = line.strip()
        current_depth = sum(skip_stack)
        if current_depth == 0:
            if pending_cfg_test and ITEM_START_RE.match(line):
                delta = brace_delta(line)
                if delta > 0:
                    skip_stack.append(delta)
                pending_cfg_test = False
                continue
            if pending_cfg_test and stripped and not stripped.startswith("#"):
                pending_cfg_test = False
            if CFG_TEST_RE.match(line):
                pending_cfg_test = True
                continue
            output.append(line)
        else:
            skip_stack[-1] += brace_delta(line)
            if skip_stack[-1] <= 0:
                skip_stack.pop()
    return output


def zero_counts() -> dict[str, int]:
    return {name: 0 for name in PATTERNS}


def current_counts() -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {}
    for path in production_rust_files():
        file_counts = zero_counts()
        for line in production_lines(path):
            # `counts_line` owns the decision - exemptions and provably
            # non-error shapes are filtered there - so the per-pattern
            # attribution below only runs on lines that actually count.
            if not counts_line(line):
                continue
            for name, pattern in PATTERNS.items():
                if pattern.search(line):
                    file_counts[name] += 1
        if sum(file_counts.values()) > 0:
            counts[path.relative_to(REPO_ROOT).as_posix()] = file_counts
    return counts


def file_total(counts: dict[str, int]) -> int:
    return sum(counts.values())


def total_counts(counts: dict[str, dict[str, int]]) -> dict[str, int]:
    totals = zero_counts()
    for file_counts in counts.values():
        for name, count in file_counts.items():
            totals[name] = totals.get(name, 0) + count
    return totals


def grand_total(counts: dict[str, dict[str, int]]) -> int:
    return sum(file_total(file_counts) for file_counts in counts.values())


def load_baseline(validate: bool = True) -> dict[str, Any]:
    if not BASELINE_FILE.exists():
        return {"version": 1, "total": 0, "totals_by_pattern": zero_counts(), "tracked_files": {}}
    data = json.loads(BASELINE_FILE.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise SystemExit(f"error: invalid baseline file format: {BASELINE_FILE}")
    tracked = data.get("tracked_files")
    totals_by_pattern = data.get("totals_by_pattern")
    total = data.get("total")
    if not isinstance(total, int) or total < 0:
        raise SystemExit(f"error: invalid total in {BASELINE_FILE}")
    if not isinstance(totals_by_pattern, dict):
        raise SystemExit(f"error: invalid totals_by_pattern in {BASELINE_FILE}")
    if not isinstance(tracked, dict):
        raise SystemExit(f"error: invalid tracked_files in {BASELINE_FILE}")
    for path, file_counts in tracked.items():
        if not isinstance(path, str) or not isinstance(file_counts, dict):
            raise SystemExit(f"error: invalid tracked_files entry in {BASELINE_FILE}")
        if any(not isinstance(v, int) or v < 0 for v in file_counts.values()):
            raise SystemExit(f"error: invalid count in tracked_files entry for {path}")
    # Type checks alone let a ledger through whose summary contradicted its own
    # rows, and every regression is measured against that summary.
    if validate:
        # Skipped for --repair, whose whole job is to fix an inconsistent
        # ledger: validating first would make the remedy unreachable.
        bc.validate_baseline(data, BASELINE_FILE)
    return data


def write_baseline(counts: dict[str, dict[str, int]]) -> None:
    BASELINE_FILE.write_text(
        json.dumps(
            {
                "version": 1,
                "total": grand_total(counts),
                "totals_by_pattern": total_counts(counts),
                "tracked_files": counts,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )


def main() -> int:
    args = parse_args()
    bc.self_check("swallowed-error", counts_line, MUST_COUNT, MUST_IGNORE)
    baseline = load_baseline(validate=not args.repair)
    current = current_counts()
    current_total = grand_total(current)
    current_pattern_totals = total_counts(current)

    if bc.run_ledger_edits(args, baseline, current, lambda data: bc.write_json(BASELINE_FILE, data)):
        return 0

    tracked: dict[str, dict[str, int]] = baseline["tracked_files"]
    regressions: list[str] = []
    improvements: list[str] = []

    if current_total > baseline["total"]:
        regressions.append(f"total swallowed-error-like count grew: {baseline['total']} -> {current_total}")
    elif current_total < baseline["total"]:
        improvements.append(f"total swallowed-error-like count shrank: {baseline['total']} -> {current_total}")

    baseline_pattern_totals: dict[str, int] = baseline["totals_by_pattern"]
    for name, count in sorted(current_pattern_totals.items()):
        old_count = baseline_pattern_totals.get(name, 0)
        if count > old_count:
            regressions.append(f"{name} count grew: {old_count} -> {count}")
        elif count < old_count:
            improvements.append(f"{name} count shrank: {old_count} -> {count}")

    for path, file_counts in sorted(current.items()):
        old_counts = tracked.get(path)
        if old_counts is None:
            regressions.append(f"new swallowed-error-like usage: {path} ({file_total(file_counts)})")
            continue
        old_total = file_total(old_counts)
        new_total = file_total(file_counts)
        if new_total > old_total:
            regressions.append(f"swallowed-error-like usage grew: {path} ({old_total} -> {new_total})")
        elif new_total < old_total:
            improvements.append(f"swallowed-error-like usage shrank: {path} ({old_total} -> {new_total})")

    for path, old_counts in sorted(tracked.items()):
        if path not in current:
            improvements.append(f"swallowed-error-like usage removed: {path} ({file_total(old_counts)} -> 0)")

    if regressions:
        print("Swallowed-error budget exceeded:", file=sys.stderr)
        for entry in regressions:
            print(f"  - {entry}", file=sys.stderr)
        print("Run scripts/check_swallowed_error_budget.py --update only after intentional cleanup.", file=sys.stderr)
        return 1

    if improvements:
        print("Swallowed-error budget improved:")
        for entry in improvements:
            print(f"  - {entry}")
        print("Consider running: scripts/check_swallowed_error_budget.py --update")
    else:
        print(
            "Swallowed-error budget OK: "
            f"total={current_total} files={len(current)} patterns={current_pattern_totals}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
