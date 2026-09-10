#!/usr/bin/env python3
"""Fail when a slash command is advertised but never dispatched.

Commands are declared once in `command_spec.rs` (name, aliases, summary,
`/help` page) but dispatched by hand-written string comparison spread across the
TUI. Nothing in the compiler ties the two together, so a command can be offered
in autocomplete and documented by `/help` while no handler claims it.

That is not hypothetical: `/fast-release`, `/fast-macos-release` and
`/remote-release` stayed advertised after `scripts/quick-release.sh` was
deleted, so typing them handed the agent a prompt telling it to run a script
that did not exist. Nothing caught it.

This checks the one direction that is mechanically decidable: every advertised
name must appear as a string literal in at least one dispatch site. It does not
prove local/remote parity - the remote dispatcher re-implements most commands
inline instead of delegating, so "appears in the local files" cannot distinguish
"remote delegates to it" from "remote forgot it". Closing that gap needs the
dispatchers rewritten as exhaustive matches over a command enum; until then this
catches the advertise-without-implement class.

Aliases are exempt: `command_spec::canonical_input` rewrites them before any
dispatcher sees them, and `command_spec`'s own tests prove each alias resolves
to its owner.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC_FILE = REPO_ROOT / "crates/jcode-tui/src/tui/app/command_spec.rs"
#: Handlers do not all live in the TUI crate - `/overnight`, for example, is
#: parsed in `jcode-overnight-core`. Scanning the whole workspace removes a
#: class of false failures where a command is dispatched from a crate this
#: script did not think to look in.
DISPATCH_ROOTS = (
    REPO_ROOT / "crates",
    REPO_ROOT / "src",
)

#: Commands whose dispatch is intentionally computed rather than string-matched.
#: Each needs a reason a reviewer can check, not just a name. Empty today: every
#: command is matched by literal, and a stale entry here is itself a failure, so
#: this cannot silently accumulate exemptions.
ALLOWLIST: dict[str, str] = {}


def advertised_names() -> list[str]:
    """Canonical names of every non-hidden command, plus hidden ones.

    Hidden commands are included on purpose: they are still dispatched when
    typed, so an unreachable hidden command is the same defect with a quieter
    symptom.
    """
    source = SPEC_FILE.read_text(encoding="utf-8")
    table = source[source.index("pub(crate) const COMMANDS"):]
    table = table[: table.index("\n];")]
    return re.findall(r'^        name: "(/[^"]+)",$', table, re.M)


def dispatch_sources() -> str:
    chunks: list[str] = []
    for root in DISPATCH_ROOTS:
        paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
        for path in paths:
            rel = path.relative_to(REPO_ROOT).as_posix()
            # The spec table itself declares the names; it is not a dispatch
            # site, so counting it would make this check vacuous.
            if rel.endswith("command_spec.rs"):
                continue
            if "/tests/" in rel or rel.endswith("_tests.rs") or rel.endswith("tests.rs"):
                continue
            chunks.append(path.read_text(encoding="utf-8"))
    return "\n".join(chunks)


def main() -> int:
    names = advertised_names()
    if len(names) < 50:
        print(
            f"error: only {len(names)} commands parsed from {SPEC_FILE.name}; "
            "the table format changed and this check would pass vacuously",
            file=sys.stderr,
        )
        return 1

    sources = dispatch_sources()
    unreachable = [
        name
        for name in names
        if f'"{name}"' not in sources and name not in ALLOWLIST
    ]

    stale_allowlist = [
        name for name in ALLOWLIST if name not in names
    ]

    if unreachable or stale_allowlist:
        if unreachable:
            print("Advertised slash commands with no dispatch site:", file=sys.stderr)
            for name in unreachable:
                print(f"  - {name}", file=sys.stderr)
            print(
                "Either add a handler, remove the entry from command_spec.rs, or "
                "allowlist it with a reason.",
                file=sys.stderr,
            )
        for name in stale_allowlist:
            print(
                f"error: allowlisted command {name} is no longer in the table; "
                "remove the allowlist entry",
                file=sys.stderr,
            )
        return 1

    print(f"command parity: {len(names)} commands, all dispatched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
