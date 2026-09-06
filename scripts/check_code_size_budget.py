#!/usr/bin/env python3
"""Enforce a ratcheting Rust file-size budget.

This script keeps the current oversized-file debt from getting worse while the
larger refactor program is underway.

Policy:
- Production Rust files above the configured LOC threshold are tracked in a
  baseline file.
- Existing tracked oversized files may not grow.
- New oversized production files may not be introduced.
- If oversized files shrink or disappear, the script reports the improvement.
- `--update` refreshes the baseline after intentional cleanup.

Ledger verbs:
- `--update [PATH ...]` records current counts; name paths to avoid laundering
  unrelated drift, and pass `--allow-regression` to record something worse.
- `--moved OLD=NEW[,NEW...]` re-keys an entry after a pure move or split and
  refuses the move if the counts do not conserve.
- `--prune` retires entries that no longer describe a measured file.
- `--repair` re-derives the summary fields from `tracked_files`.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

import budget_common as bc

REPO_ROOT = Path(__file__).resolve().parent.parent
BASELINE_FILE = REPO_ROOT / "scripts" / "code_size_budget.json"
DEFAULT_THRESHOLD = 1200
SCAN_ROOTS = (REPO_ROOT / "src", REPO_ROOT / "crates")

#: What this classifier must and must not treat as in-scope, proven on every
#: run. The "detector" here is the file-classification rule, and a rule that
#: accidentally excluded everything would silently empty the ledger while
#: reporting that every tracked file had improved.
MUST_CLASSIFY = ('src/cli/acp.rs', 'crates/jcode-base/src/skill.rs', 'crates/jcode-tui/src/tui/ui_prepare.rs')
MUST_SKIP = ('src/cli/acp_tests.rs', 'crates/jcode-base/src/skill_tests.rs', 'crates/jcode-tui/src/tui/app/tests/scroll_copy_02/part_01.rs', 'scripts/budget_common.py')


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    bc.add_ledger_args(parser)
    return parser.parse_args()


def is_production_rust_file(path: Path) -> bool:
    rel = path.relative_to(REPO_ROOT).as_posix()
    if path.suffix != ".rs":
        return False
    parts = rel.split("/")
    if parts[0] == "tests" or any(
        part == "tests" or part.endswith("_tests") or part.endswith("_test") or part.startswith("tests_")
        for part in parts
    ):
        return False
    name = path.name
    if (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or name.endswith("_test.rs")
        or name.startswith("tests_")
    ):
        return False
    return True


def rust_file_line_count(path: Path) -> int:
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for _ in handle)


def current_oversized_files(threshold: int) -> dict[str, int]:
    files: dict[str, int] = {}
    for root in SCAN_ROOTS:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*.rs")):
            if not is_production_rust_file(path):
                continue
            line_count = rust_file_line_count(path)
            if line_count > threshold:
                files[path.relative_to(REPO_ROOT).as_posix()] = line_count
    return files


def load_baseline(validate: bool = True) -> dict[str, Any]:
    if not BASELINE_FILE.exists():
        raise SystemExit(f"error: missing baseline file: {BASELINE_FILE}")
    data = json.loads(BASELINE_FILE.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise SystemExit(f"error: invalid baseline file format: {BASELINE_FILE}")
    threshold = data.get("threshold_loc")
    tracked = data.get("tracked_files")
    if not isinstance(threshold, int) or threshold <= 0:
        raise SystemExit(f"error: invalid threshold_loc in {BASELINE_FILE}")
    if not isinstance(tracked, dict) or any(
        not isinstance(k, str) or not isinstance(v, int) or v <= 0
        for k, v in tracked.items()
    ):
        raise SystemExit(f"error: invalid tracked_files in {BASELINE_FILE}")
    if validate:
        # Skipped for --repair, whose whole job is to fix an inconsistent
        # ledger: validating first would make the remedy unreachable.
        bc.validate_baseline(data, BASELINE_FILE)
    return data


def write_baseline(threshold: int, tracked_files: dict[str, int]) -> None:
    payload = {
        "version": 1,
        "threshold_loc": threshold,
        "tracked_files": tracked_files,
    }
    BASELINE_FILE.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> int:
    args = parse_args()
    bc.self_check(
        "production-file",
        lambda rel: is_production_rust_file(REPO_ROOT / rel),
        MUST_CLASSIFY,
        MUST_SKIP,
    )
    baseline = load_baseline(validate=not args.repair)
    threshold = baseline["threshold_loc"]
    current = current_oversized_files(threshold)

    if bc.run_ledger_edits(args, baseline, current, lambda data: bc.write_json(BASELINE_FILE, data)):
        return 0

    tracked: dict[str, int] = baseline["tracked_files"]
    regressions: list[str] = []
    improvements: list[str] = []

    for path, lines in sorted(current.items()):
        old_lines = tracked.get(path)
        if old_lines is None:
            regressions.append(
                f"new oversized file exceeds {threshold} LOC: {path} ({lines} LOC)"
            )
        elif lines > old_lines:
            regressions.append(
                f"oversized file grew: {path} ({old_lines} -> {lines} LOC)"
            )
        elif lines < old_lines:
            improvements.append(f"oversized file shrank: {path} ({old_lines} -> {lines} LOC)")

    for path, old_lines in sorted(tracked.items()):
        if path not in current:
            improvements.append(
                f"oversized file no longer exceeds {threshold} LOC: {path} ({old_lines} -> OK)"
            )

    if regressions:
        print(
            "Code-size budget exceeded. Existing oversized Rust files must shrink or stay flat, "
            "and new oversized production files are not allowed:",
            file=sys.stderr,
        )
        for entry in regressions:
            print(f"  - {entry}", file=sys.stderr)
        print(
            "Run scripts/check_code_size_budget.py --update only after intentional cleanup.",
            file=sys.stderr,
        )
        return 1

    if improvements:
        print("Code-size budget improved:")
        for entry in improvements:
            print(f"  - {entry}")
        print("Consider running: scripts/check_code_size_budget.py --update")
    else:
        print(
            "Code-size budget OK: "
            f"tracked={len(tracked)} threshold={threshold}LOC no oversized-file regressions"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
