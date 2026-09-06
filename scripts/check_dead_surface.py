#!/usr/bin/env python3
"""Fail when the repo grows surface nothing consumes.

Three conditions this fork has actually shipped, each mechanically detectable
and each previously found only by hand:

1. **A workspace member with no dependents.** `crates/jcode-sdk` (4,398 lines)
   sat in `[workspace] members` with zero dependents after its only consumer
   was deleted. Nothing noticed, because a member still compiles.
2. **A declared `[[bin]]` with no invoker.** The `jcode-harness-api-bridge`
   target's only spawner was inside that same orphaned crate.
3. **A script referenced by nothing.** 47 accumulated, 12,000+ lines.

The third case is why this exists as a *guard* rather than a one-off audit:
deleting 43 orphaned scripts orphaned 4 more, because each had been referenced
only by one of the deleted ones. Cascades are invisible to a manual sweep — the
audit that removed the 43 reported "fixpoint" while 4 fresh orphans existed.

An orphan may be accepted, but only with a stated reason, recorded in
`scripts/dead_surface_allowlist.json`. The 13 `test_*` harnesses are there
because reference counting is the wrong test for a script a human runs by hand;
that judgement belongs in the file, next to the path, not in someone's memory.

Usage:
    check_dead_surface.py                     # gate
    check_dead_surface.py --allow PATH REASON # accept one orphan, with why
    check_dead_surface.py --prune             # drop entries that are no longer orphans
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import sys
from pathlib import Path

import budget_common as bc

REPO_ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST_FILE = REPO_ROOT / "scripts" / "dead_surface_allowlist.json"

#: Extensions worth searching for a reference. A consumer that is not text in
#: this repo (a human, a shell history, a CI system that does not live here)
#: cannot be detected, which is exactly what the allowlist is for.
INDEX_SUFFIXES = (".rs", ".toml", ".sh", ".py", ".md", ".yml", ".yaml")
SKIP_DIRS = ("./target", "./.git", "./.jcode", "./node_modules")


def index_repo() -> dict[str, str]:
    """Every searchable file's text, read once.

    Reading the tree once and searching in memory rather than shelling out per
    candidate is the difference between ~2 seconds and a 20-minute run: the
    naive form greps the whole repo once per item.
    """
    out: dict[str, str] = {}
    for dirpath, dirnames, filenames in os.walk(REPO_ROOT):
        rel = "./" + os.path.relpath(dirpath, REPO_ROOT)
        if any(rel == skip or rel.startswith(skip + "/") for skip in SKIP_DIRS):
            dirnames[:] = []
            continue
        for name in filenames:
            if name.endswith(INDEX_SUFFIXES):
                path = os.path.join(dirpath, name)
                try:
                    out["./" + os.path.relpath(path, REPO_ROOT)] = Path(path).read_text(
                        encoding="utf-8", errors="ignore"
                    )
                except OSError:
                    pass
    return out


def referenced_by(index: dict[str, str], needle: str, exclude: str | None = None) -> list[str]:
    return [f for f, text in index.items() if needle in text and f != exclude]


def orphan_crates(index: dict[str, str]) -> dict[str, str]:
    """Workspace members that no other manifest depends on.

    Membership is not a dependency: a member is compiled by `--workspace` and
    shipped by nothing.
    """
    root = index.get("./Cargo.toml", "")
    found: dict[str, str] = {}
    for member in re.findall(r'^\s*"(crates/[^"]+)",', root, re.M):
        name = os.path.basename(member)
        dependents = [
            f
            for f in referenced_by(index, name)
            if f.endswith("Cargo.toml") and f != f"./{member}/Cargo.toml"
        ]
        # The root manifest counts only if it names the crate as a dependency,
        # not merely as a member.
        if dependents == ["./Cargo.toml"] and not re.search(rf"^\s*{re.escape(name)}\s*=", root, re.M):
            dependents = []
        if not dependents:
            found[member] = "workspace member with no dependent manifest"
    return found


def orphan_bins(index: dict[str, str]) -> dict[str, str]:
    """Declared binary targets that nothing in the repo invokes."""
    found: dict[str, str] = {}
    for manifest, text in index.items():
        if not manifest.endswith("Cargo.toml"):
            continue
        for block in re.findall(r"\[\[bin\]\](.*?)(?=\n\[|\Z)", text, re.S):
            match = re.search(r'name\s*=\s*"([^"]+)"', block)
            if not match:
                continue
            name = match.group(1)
            if name == "jcode":
                continue  # the product itself
            invokers = [
                f
                for f in referenced_by(index, name)
                if f.endswith((".sh", ".py", ".md", ".yml", ".yaml"))
            ]
            if not invokers:
                found[f"{manifest}::{name}"] = "declared [[bin]] with no script or doc invoker"
    return found


def orphan_scripts(index: dict[str, str]) -> dict[str, str]:
    """Scripts no other file in the repo names."""
    found: dict[str, str] = {}
    for path in sorted(glob.glob("scripts/*.sh") + glob.glob("scripts/*.py")):
        base = os.path.basename(path)
        if not referenced_by(index, base, exclude=f"./{path}"):
            found[path] = "script referenced by no file in the repo"
    return found

def is_test_path(path: str) -> bool:
    parts = path.split("/")
    stem = os.path.splitext(parts[-1])[0]
    return (
        stem.endswith(("_tests", "_test"))
        or stem == "tests"
        or any(p in ("tests", "test") or p.endswith("_tests") for p in parts[:-1])
    )


def orphan_pub_mods(index: dict[str, str]) -> dict[str, str]:
    """`pub mod`s with no production consumer - release weight for nothing.

    `jcode_plan::dag::sim` was one: a `pub mod` whose only users were the
    crate's own tests, so it compiled into every release build to serve a test
    harness. It is now `#[cfg(test)]`.

    Two refinements this needs to avoid drowning in false positives, both found
    by checking the first draft's output by hand rather than trusting it:

    - **A re-export in the declaring file counts as consumption.** If `lib.rs`
      says `pub use math::render_inline_latex;`, callers reach the items through
      the parent path and never name `math`. That alone accounted for 9 of 17
      first-draft hits.
    - **Usage in the declaring file counts.** A parent consuming its own child
      (`translate::BridgeState` inside the `lib.rs` that declares
      `pub mod translate`) is ordinary structure, not dead code. That accounted
      for 4 more.
    """
    found: dict[str, str] = {}
    for path, text in index.items():
        if not path.endswith(".rs"):
            continue
        for name in re.findall(r"^\s*pub mod ([a-z_][a-z0-9_]*)\s*;", text, re.M):
            # A re-export publishes the module under the parent's path. All
            # three spellings count, including the aliased form that made the
            # first version of this detector wrongly flag `embedding_stub`
            # (`pub use embedding_stub as embedding;` - no `::` to match on).
            if re.search(
                rf"pub use (?:self::|crate::[a-z_:]*)?{name}\s*(?:::|as\s|;)", text
            ):
                continue
            if re.search(rf"\b{name}::", text):
                continue
            needles = (f"{name}::", f"use {name}", f"mod {name}")
            refs = [
                f
                for f, other in index.items()
                if f != path and f.endswith(".rs") and any(n in other for n in needles)
            ]
            if any(not is_test_path(f) for f in refs):
                continue
            found[f"{path}::{name}"] = (
                "pub mod consumed only by tests" if refs else "pub mod with no consumer anywhere"
            )
    return found


def orphan_examples(index: dict[str, str]) -> dict[str, str]:
    """Example targets nothing names.

    Examples are compiled by `cargo check --all-targets --all-features`, which
    `check_guardrails.sh` runs on every sweep, so an unreferenced example is
    build cost with no reader.
    """
    found: dict[str, str] = {}
    for path in sorted(glob.glob("crates/*/examples/*.rs") + glob.glob("examples/*.rs")):
        stem = os.path.splitext(os.path.basename(path))[0]
        if not referenced_by(index, stem, exclude=f"./{path}"):
            found[path] = "example target referenced by no file in the repo"
    return found


DETECTORS = {
    "crate": orphan_crates,
    "bin": orphan_bins,
    "script": orphan_scripts,
    "pub-mod": orphan_pub_mods,
    "example": orphan_examples,
}

#: The self-check: a synthetic index in which each detector must find its own
#: planted orphan, and must not flag the wired counterpart beside it. Without
#: this, a detector that silently stops matching reports "no dead surface".
def self_check() -> None:
    wired_manifest = '[workspace]\nmembers = [\n    "crates/live",\n]\n\n[dependencies]\nlive = { path = "crates/live" }\n'
    fixture = {
        "./Cargo.toml": wired_manifest + '\n[[bin]]\nname = "used_bin"\n\n[[bin]]\nname = "unused_bin"\n',
        "./crates/live/Cargo.toml": 'name = "live"\n',
        "./scripts/run.sh": "used_bin\n",
    }
    bins = orphan_bins(fixture)
    if "./Cargo.toml::unused_bin" not in bins:
        raise SystemExit(
            "error: the dead-surface bin detector failed its own self-check: it did not "
            "flag a [[bin]] with no invoker.\n"
            "       Refusing to report 'no dead surface' from a detector that cannot find "
            "a planted orphan."
        )
    if "./Cargo.toml::used_bin" in bins:
        raise SystemExit(
            "error: the dead-surface bin detector failed its own self-check: it flagged a "
            "[[bin]] that scripts/run.sh invokes."
        )

    orphan_member = '[workspace]\nmembers = [\n    "crates/ghost",\n]\n'
    crates = orphan_crates({"./Cargo.toml": orphan_member, "./crates/ghost/Cargo.toml": 'name = "ghost"\n'})
    if "crates/ghost" not in crates:
        raise SystemExit(
            "error: the dead-surface crate detector failed its own self-check: it did not "
            "flag a workspace member with no dependent."
        )

    # The pub-mod detector needs both of its refinements exercised, because a
    # regression in either direction is silent: lose them and it drowns the
    # gate in false positives; lose the core match and it finds nothing.
    mods = orphan_pub_mods(
        {
            # dead: nothing anywhere names it
            "./crates/a/src/lib.rs": "pub mod ghost;\n",
            # alive via re-export from the declaring file
            "./crates/b/src/lib.rs": "pub mod inner;\npub use inner::Thing;\n",
            # alive via the parent consuming its own child
            "./crates/c/src/lib.rs": "pub mod child;\nfn f() { child::go(); }\n",
            # test-only consumer: reported, but distinctly
            "./crates/d/src/lib.rs": "pub mod probe;\n",
            "./crates/d/src/lib_tests.rs": "probe::run();\n",
        }
    )
    expected_dead = "./crates/a/src/lib.rs::ghost"
    if expected_dead not in mods:
        raise SystemExit(
            "error: the dead-surface pub-mod detector failed its own self-check: it did "
            "not flag a `pub mod` with no consumer."
        )
    for alive in ("./crates/b/src/lib.rs::inner", "./crates/c/src/lib.rs::child"):
        if alive in mods:
            raise SystemExit(
                f"error: the dead-surface pub-mod detector failed its own self-check: it "
                f"flagged {alive}, which is consumed (re-export / parent usage)."
            )
    if mods.get("./crates/d/src/lib.rs::probe") != "pub mod consumed only by tests":
        raise SystemExit(
            "error: the dead-surface pub-mod detector failed its own self-check: a "
            "test-only consumer must be reported as such, not as 'no consumer anywhere'."
        )

    examples = orphan_examples({})
    if not isinstance(examples, dict):
        raise SystemExit("error: the dead-surface example detector returned a non-map")


def load_allowlist() -> dict[str, str]:
    if not ALLOWLIST_FILE.exists():
        return {}
    data = json.loads(ALLOWLIST_FILE.read_text(encoding="utf-8"))
    allowed = data.get("allowed")
    if not isinstance(allowed, dict):
        raise SystemExit(f"error: invalid 'allowed' map in {ALLOWLIST_FILE}")
    unexplained = [path for path, reason in allowed.items() if not str(reason).strip()]
    if unexplained:
        raise SystemExit(
            f"error: {ALLOWLIST_FILE} has entries with no reason: {unexplained}.\n"
            f"       An unexplained exemption is what this guard exists to prevent."
        )
    return allowed


def write_allowlist(allowed: dict[str, str]) -> None:
    bc.write_json(ALLOWLIST_FILE, {"version": 1, "allowed": dict(sorted(allowed.items()))})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow", nargs=2, metavar=("PATH", "REASON"), action="append", default=[])
    parser.add_argument("--prune", action="store_true", help="drop entries that are no longer orphans")
    args = parser.parse_args()

    self_check()
    allowed = load_allowlist()
    index = index_repo()
    current: dict[str, str] = {}
    for kind, detect in DETECTORS.items():
        for path, why in detect(index).items():
            current[path] = f"{kind}: {why}"

    if args.allow:
        for path, reason in args.allow:
            if not reason.strip():
                raise SystemExit("error: --allow requires a non-empty reason")
            allowed[path] = reason.strip()
            print(f"  - allowed {path}: {reason.strip()}")
        write_allowlist(allowed)
        return 0

    if args.prune:
        stale = [p for p in allowed if p not in current]
        for path in stale:
            allowed.pop(path)
            print(f"  - {path}: retired (no longer orphaned, or gone)")
        write_allowlist(allowed)
        print(f"allowlist: {len(allowed)} accepted orphans")
        return 0

    new = {p: why for p, why in current.items() if p not in allowed}
    if new:
        print("Dead-surface check failed: the repo grew surface nothing consumes:")
        for path, why in sorted(new.items()):
            print(f"  - {path} ({why})")
        print(
            "\nRemove it, wire it to a consumer, or accept it with a stated reason:\n"
            "  scripts/check_dead_surface.py --allow <path> '<why this is fine>'"
        )
        return 1

    resolved = [p for p in allowed if p not in current]
    print(f"Dead-surface check passed: 0 new orphans, {len(allowed)} accepted.")
    if resolved:
        print(f"  {len(resolved)} allowlist entries are no longer orphaned; run --prune to retire them.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
