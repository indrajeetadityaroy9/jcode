"""Shared ledger mechanics for the four ratchet scripts.

`check_code_size_budget`, `check_test_size_budget`, `check_panic_budget` and
`check_swallowed_error_budget` each keep a JSON ledger of per-file counts and
fail when a file grows past its recorded entry. They were 67-88% duplicated
line-for-line, and every copy shared the same five gaps:

1. **The aggregate was never checked against its own table.** Each ledger
   carries a `total` (and the swallowed one a `totals_by_pattern`) alongside
   `tracked_files`. Nothing verified that the summary equals the sum of the
   parts, so a hand edit or an interrupted update left a ledger that reports
   confident nonsense: a `total` of 999999 against zeroed pattern totals still
   loaded, and then every pattern read as a huge regression.
   `load_baseline` now recomputes both and refuses to run on a ledger that
   disagrees with itself.

2. **`--update` was all-or-nothing.** It rebased *every* entry, so accepting
   one intentional growth silently laundered every unrelated drift in the tree.
   `--update PATH ...` now records exactly the paths named.

3. **A pure file move read as a regression.** Splitting a file re-keys its
   counts under a new path, which the checker reported as "new usage" with no
   notion that the total was conserved. `--moved OLD=NEW[,NEW2...]` re-keys an
   entry and *refuses* the move if the counts do not conserve.

4. **Intent could only be recorded as a number.** A deliberate `let _ = ...`
   could only be absorbed into an opaque count, leaving the reason nowhere in
   the code. Counting scripts now honour a `// budget-ok: <reason>` annotation,
   so an exemption is justified where a reviewer reads it.

5. **Dead entries lingered forever.** Entries for deleted files, or for files
   that dropped under the threshold, stayed in the ledger and re-reported as
   "improvements" on every run. `--prune` retires them.

Two ledger shapes exist and both are supported: a scalar count per file
(`{"path": 1260}`) and a per-pattern breakdown (`{"path": {"dot_ok": 2}}`).
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any, Iterable, Mapping

REPO_ROOT = Path(__file__).resolve().parent.parent

#: Opt a single line out of a counting budget, with the reason in the code.
#:
#: Written as a trailing or preceding comment: `// budget-ok: <reason>`. A bare
#: `// budget-ok` with no reason is deliberately NOT matched - an exemption
#: without a justification is the thing this is meant to prevent.
EXEMPTION_RE = re.compile(r"//\s*budget-ok:\s*\S")


def exemption_reason(line: str) -> str | None:
    """The reason text of a `// budget-ok:` annotation on `line`, if any."""
    match = re.search(r"//\s*budget-ok:\s*(.+?)\s*$", line)
    return match.group(1) if match else None


def self_check(
    name: str,
    detects: Any,
    must_count: Iterable[str],
    must_ignore: Iterable[str],
) -> None:
    """Refuse to report on a detector that no longer detects.

    A ratchet compares "what I measure now" against a recorded number, and
    trusts its own measurement absolutely. So a detector that stops matching
    does not fail - it reports a triumphant improvement and exits 0. Neutering
    `check_panic_budget`'s pattern to something unmatchable turned
    `total=58` into `"total panic-prone count shrank: 58 -> 0"` with a passing
    exit code, which is a guard silently switching itself off while the output
    congratulates you for it.

    Every guard therefore proves itself against fixtures before it is allowed
    to draw a conclusion: lines it must count, and lines it must leave alone.
    The cost is a handful of regex evaluations per run.
    """
    missed = [line for line in must_count if not detects(line)]
    caught = [line for line in must_ignore if detects(line)]
    if not missed and not caught:
        return

    report = [f"error: the {name} detector failed its own self-check."]
    if missed:
        report.append("       it no longer counts lines it must count:")
        report += [f"         + {line}" for line in missed]
    if caught:
        report.append("       it counts lines it must ignore:")
        report += [f"         - {line}" for line in caught]
    report.append(
        "       Refusing to report a budget from a detector that cannot find its own "
        "fixtures; a broken detector reads as an improvement."
    )
    raise SystemExit("\n".join(report))

def entry_total(entry: int | Mapping[str, int]) -> int:
    """Sum one ledger entry, whichever shape it has."""
    if isinstance(entry, Mapping):
        return sum(entry.values())
    return int(entry)


def grand_total(tracked: Mapping[str, Any]) -> int:
    return sum(entry_total(value) for value in tracked.values())


def pattern_totals(tracked: Mapping[str, Any], patterns: Iterable[str]) -> dict[str, int]:
    return {
        name: sum(entry.get(name, 0) for entry in tracked.values() if isinstance(entry, Mapping))
        for name in patterns
    }


def validate_baseline(
    data: Mapping[str, Any], baseline_file: Path, table_key: str = "tracked_files"
) -> None:
    """Fail loudly when a ledger's summary disagrees with its own table.

    This is the check whose absence let a corrupt aggregate through. It is a
    hard error rather than a warning: every downstream comparison is against
    these numbers, so a ledger that cannot add up its own rows cannot report a
    trustworthy regression either.

    `table_key` exists because the five ledgers in this repo do not agree on a
    name for their table - four say `tracked_files`, the wildcard one says
    `files`.
    """
    tracked = data.get(table_key)
    if not isinstance(tracked, dict):
        raise SystemExit(f"error: invalid {table_key} in {baseline_file}")

    if "total" in data:
        expected = grand_total(tracked)
        if data["total"] != expected:
            raise SystemExit(
                f"error: {baseline_file} is internally inconsistent: total={data['total']} "
                f"but its {len(tracked)} entries sum to {expected}.\n"
                f"       A hand edit or an interrupted --update left it that way. Re-derive "
                f"the summary with --repair, or fix the entry that is wrong."
            )

    if "totals_by_pattern" in data:
        declared = data["totals_by_pattern"]
        if not isinstance(declared, dict):
            raise SystemExit(f"error: invalid totals_by_pattern in {baseline_file}")
        expected_patterns = pattern_totals(tracked, declared.keys())
        if declared != expected_patterns:
            disagree = {
                name: (declared.get(name), expected_patterns[name])
                for name in expected_patterns
                if declared.get(name) != expected_patterns[name]
            }
            raise SystemExit(
                f"error: {baseline_file} is internally inconsistent: totals_by_pattern "
                f"disagrees with tracked_files for {disagree} (declared vs actual).\n"
                f"       Re-derive the summary with --repair."
            )


def repair_summary(data: dict[str, Any], table_key: str = "tracked_files") -> dict[str, Any]:
    """Re-derive `total`/`totals_by_pattern` from `tracked_files`.

    Tightening only: the table is the source of truth, so this can lower an
    inflated aggregate but never invents headroom.
    """
    tracked = data[table_key]
    if "totals_by_pattern" in data:
        data["totals_by_pattern"] = pattern_totals(tracked, data["totals_by_pattern"].keys())
    if "total" in data:
        data["total"] = grand_total(tracked)
    return data


def add_ledger_args(parser: argparse.ArgumentParser) -> None:
    """The shared ledger-editing verbs, identical across all four scripts."""
    parser.add_argument(
        "--update",
        nargs="*",
        metavar="PATH",
        default=None,
        help=(
            "record current counts. With no PATH it rebases the whole ledger "
            "(this launders unrelated drift - prefer naming paths)."
        ),
    )
    parser.add_argument(
        "--moved",
        action="append",
        default=[],
        metavar="OLD=NEW[,NEW...]",
        help=(
            "re-key an entry after a pure file move or split; refuses the move "
            "if the counts do not conserve"
        ),
    )
    parser.add_argument(
        "--prune",
        action="store_true",
        help="retire entries for files that no longer exist or no longer qualify",
    )
    parser.add_argument(
        "--repair",
        action="store_true",
        help="re-derive the summary fields from tracked_files",
    )
    parser.add_argument(
        "--allow-regression",
        action="store_true",
        help="permit --update to record a count that is worse than the baseline",
    )


def apply_update(
    tracked: dict[str, Any],
    current: Mapping[str, Any],
    paths: list[str] | None,
    allow_regression: bool,
) -> list[str]:
    """Record `current` counts for `paths` (or every path when `paths` is None)."""
    notes: list[str] = []
    if not paths:
        for path in sorted(set(tracked) | set(current)):
            if path in current:
                tracked[path] = current[path]
            else:
                tracked.pop(path, None)
        notes.append(f"rebased every entry ({len(tracked)} files)")
        return notes

    for path in paths:
        if path not in current:
            tracked.pop(path, None)
            notes.append(f"{path}: no longer qualifies -> entry retired")
            continue
        before = tracked.get(path)
        after = current[path]
        if before is not None and entry_total(after) > entry_total(before):
            if not allow_regression:
                raise SystemExit(
                    f"error: {path} would be recorded worse than its baseline "
                    f"({entry_total(before)} -> {entry_total(after)}). Fix the code, or pass "
                    f"--allow-regression to record it deliberately."
                )
            notes.append(
                f"{path}: RECORDED REGRESSION {entry_total(before)} -> {entry_total(after)}"
            )
        else:
            notes.append(f"{path}: {entry_total(before) if before is not None else 0} -> {entry_total(after)}")
        tracked[path] = after
    return notes


def apply_moves(
    tracked: dict[str, Any],
    current: Mapping[str, Any],
    moves: list[str],
) -> list[str]:
    """Re-key entries after a pure move/split, refusing non-conserving moves.

    `OLD=NEW[,NEW2...]` because a split sends one file's counts to several. The
    conservation check is the point: it is what distinguishes "I moved code" from
    "I added code and would like the ledger to forget".
    """
    notes: list[str] = []
    for spec in moves:
        if "=" not in spec:
            raise SystemExit(f"error: --moved expects OLD=NEW[,NEW...], got {spec!r}")
        old, new_spec = spec.split("=", 1)
        new_paths = [p for p in new_spec.split(",") if p]
        if old not in tracked:
            raise SystemExit(f"error: --moved source {old} has no ledger entry")

        before = entry_total(tracked[old])
        after_entries = {p: current[p] for p in new_paths if p in current}
        surviving = entry_total(current.get(old, 0)) if old in current else 0
        after = surviving + sum(entry_total(v) for v in after_entries.values())
        if after > before:
            raise SystemExit(
                f"error: --moved {old} -> {new_paths} does not conserve: {before} -> {after}. "
                f"That is new code, not a move; use --update --allow-regression if it is intended."
            )

        tracked.pop(old, None)
        if old in current:
            tracked[old] = current[old]
        for path, value in after_entries.items():
            tracked[path] = value
        notes.append(
            f"{old} -> {', '.join(new_paths)}: {before} conserved as {after}"
            + (" (shrank)" if after < before else "")
        )
    return notes


def apply_prune(tracked: dict[str, Any], current: Mapping[str, Any]) -> list[str]:
    """Retire entries that no longer describe anything the checker measures."""
    notes: list[str] = []
    for path in sorted(tracked):
        if path in current:
            continue
        reason = "file no longer exists" if not (REPO_ROOT / path).exists() else "no longer qualifies"
        tracked.pop(path)
        notes.append(f"{path}: retired ({reason})")
    return notes


def run_ledger_edits(
    args: argparse.Namespace,
    data: dict[str, Any],
    current: Mapping[str, Any],
    write: Any,
    table_key: str = "tracked_files",
) -> bool:
    """Apply whichever ledger verbs were requested. True when one ran."""
    tracked: dict[str, Any] = data[table_key]
    notes: list[str] = []
    touched = False

    if args.repair:
        repair_summary(data, table_key)
        notes.append("re-derived the summary from tracked_files")
        touched = True
    if args.moved:
        notes += apply_moves(tracked, current, args.moved)
        touched = True
    if args.prune:
        notes += apply_prune(tracked, current)
        touched = True
    if args.update is not None:
        notes += apply_update(tracked, current, args.update, args.allow_regression)
        touched = True

    if not touched:
        return False

    repair_summary(data, table_key)
    write(data)
    for note in notes:
        print(f"  - {note}")
    print(f"ledger updated: {len(tracked)} tracked files, total {grand_total(tracked)}")
    return True


def write_json(path: Path, data: Mapping[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
