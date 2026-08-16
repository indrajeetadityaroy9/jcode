#!/usr/bin/env bash
# Inventory what upstream ADDED, so new features are not silently lost.
#
# scripts/purge-guard.sh and scripts/classify-upstream.sh are both DEFENSIVE:
# they find purged code that should not be present. Neither can find the
# opposite failure — upstream work that should be present and is not. A feature
# dropped during conflict resolution leaves no trace: no conflict marker, no
# compile error, no guard hit. It simply never arrives, and nobody knows to look
# for it because nobody read what upstream shipped.
#
# This script produces the checklist that closes that hole.
#
# Modes:
#   inventory  (default) run in the tmp clone BEFORE resolving. Prints upstream's
#              own release notes plus the structural surface it grew, each entry
#              marked KEEP or PURGED-BY-POLICY.
#   verify     run in $MAIN AFTER the merge. Asserts the structural items from
#              the KEEP set are actually present in the working tree.
#
# Usage:
#   ./scripts/upstream-features.sh inventory <base-ref> <upstream-ref>
#   ./scripts/upstream-features.sh verify    <base-ref> <upstream-ref>
set -uo pipefail

MODE="${1:-inventory}"
BASE="${2:?usage: upstream-features.sh <inventory|verify> <base-ref> <upstream-ref>}"
REF="${3:?usage: upstream-features.sh <inventory|verify> <base-ref> <upstream-ref>}"

# Features this fork drops on purpose. Keep aligned with §1 of
# docs/FORK_WORKFLOW.md — both the purge table and the "excluded by choice" list.
#
# Deliberately TIGHT. Bare `subscription` would match subscription_api.rs and
# `jcode login --provider jcode`, which §1 explicitly KEEPS as provider auth;
# only the onboarding pill is excluded.
PURGED_DESC='telemetry|sponsor|gateway|discovery|integration tools|request_permission|ambient permission|iOS|pair your phone'
EXCLUDED_DESC='subscription onboarding|onboarding[^.]*subscription|defaults to the [Jj]code subscription|subscription[^.]*onboarding'

# Prose patterns do NOT match bare identifiers: a changelog line says "pair your
# phone", but Cargo.toml says `jcode-gateway-types` and args.rs says `Pair {`.
# Structural items need their own vocabulary, aligned with purge-guard.sh.
#
# Each subsystem needs a BARE stem, not just its crate name. `jcode-gateway-types`
# matches the crate but not `GatewayConfig`, which is how three purged config
# fields were first misreported as dropped upstream work.
PURGED_IDENT='telemetry|sponsors|gateway|discover|integration_tools|request_permission|jcode-tui-permissions|^[[:space:]]*(Pair|Permissions|Telemetry)\b'

hdr() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
mark() {
  if   grep -qiE "$PURGED_DESC"   <<<"$1"; then printf 'PURGED'
  elif grep -qiE "$EXCLUDED_DESC" <<<"$1"; then printf 'EXCLUDED'
  else                                          printf 'KEEP'
  fi
}
# Structural-item marker. Returns PURGED for anything this fork deletes.
mark_i() { grep -qiE "$PURGED_IDENT" <<<"$1" && printf 'PURGED' || printf 'KEEP'; }
# Print a structural line with its marker, stripping the leading diff '+'.
emit_i() { while IFS= read -r l; do
    t=$(sed 's/^+//' <<<"$l"); printf '    %-6s %s\n' "$(mark_i "$t")" "$(sed 's/^ *//' <<<"$t")"
  done; }

# ---------------------------------------------------------------- inventory --
if [ "$MODE" = inventory ]; then

  hdr "1. Upstream release notes (authoritative feature list)"
  notes=$(git diff --name-only --diff-filter=AM "$BASE".."$REF" -- changelog/ \
            | grep -E 'changelog/v[0-9]' | sort -V)
  if [ -z "$notes" ]; then
    echo "  no new changelog entries in this range"
  else
    for f in $notes; do
      body=$(git show "$REF:$f" 2>/dev/null) || continue
      printf '\n  %s — %s\n' "$(jq -r .version <<<"$body")" "$(jq -r .title <<<"$body")"
      for kind in highlights improvements fixes; do
        case $kind in
          highlights)   label=HIGH ;;
          improvements) label=IMPR ;;
          fixes)        label=FIX  ;;
        esac
        while IFS= read -r line; do
          [ -z "$line" ] && continue
          printf '    %-4s %-8s %s\n' "$label" "$(mark "$line")" "$line"
        done < <(jq -r ".${kind}[]? // empty" <<<"$body")
      done
    done
  fi

  hdr "2. Feature-shaped commits"
  echo "  (upstream uses conventional commits inconsistently — treat as a supplement"
  echo "   to section 1, not a replacement)"
  git log --format='%h %s' "$BASE".."$REF" 2>/dev/null \
    | grep -iE ' (feat|perf)(\(|:)' | sed 's/^/    /' || echo "    none"

  hdr "3. Structural surface added (each must be verifiable post-merge)"

  echo "  -- new workspace members --"
  git diff "$BASE".."$REF" -- Cargo.toml 2>/dev/null \
    | grep -E '^\+\s+"crates/' | emit_i

  echo "  -- new CLI subcommands (src/cli/args.rs) --"
  git diff "$BASE".."$REF" -- src/cli/args.rs 2>/dev/null \
    | grep -E '^\+\s{4}[A-Z][A-Za-z]+\s*[({,]' | emit_i

  # Registrations span several lines, so the tool NAME is usually on its own
  # line rather than beside the insert_tool call. Pull the quoted names.
  echo "  -- new registered tools --"
  git diff "$BASE".."$REF" -- crates/jcode-app-core/src/tool/mod.rs 2>/dev/null \
    | grep '^+' | grep -oE '"[a-z][a-z_]{2,}"' | sort -u | emit_i

  echo "  -- new config fields --"
  git diff "$BASE".."$REF" -- crates/jcode-config-types/src/lib.rs 2>/dev/null \
    | grep -E '^\+\s+pub [a-z_]+:' | emit_i

  echo "  -- new scripts --"
  git diff --name-only --diff-filter=A "$BASE".."$REF" -- scripts/ 2>/dev/null | emit_i

  hdr "Next"
  cat <<'EOF'
  Every KEEP line above is a claim to verify after the merge. The dangerous ones
  are KEEP features whose commits landed in the MIXED bucket of
  classify-upstream.sh: those needed hand-surgery, so that is exactly where a
  feature gets stripped along with the purged code sharing its hunk.

  Re-run with `verify` once the merge is in $MAIN.
EOF

# ------------------------------------------------------------------- verify --
elif [ "$MODE" = verify ]; then

  fail=0
  hdr "Verifying KEEP structural items landed"

  # NOTE: BSD sed (macOS) does not understand \s. Use [[:space:]] in every sed
  # expression here — with \s the extraction silently returns the whole diff
  # line instead of the identifier, and every check then reports a false MISS.
  check() { # check <label> <pattern> <file>
    if [ ! -e "$3" ]; then printf '  SKIP  %s (no %s)\n' "$1" "$3"; return; fi
    if grep -qE "$2" "$3" 2>/dev/null; then
      printf '  OK    %s\n' "$1"
    else
      printf '  MISS  %s  — expected /%s/ in %s\n' "$1" "$2" "$3"; fail=1
    fi
  }

  while IFS= read -r m; do
    crate=$(sed -E 's/.*"(crates\/[^"]+)".*/\1/' <<<"$m")
    [ "$(mark_i "$crate")" = PURGED ] && { printf '  SKIP  workspace member %s (purged by policy)\n' "$crate"; continue; }
    [ -d "$crate" ] && printf '  OK    workspace member %s\n' "$crate" \
                    || { printf '  MISS  workspace member %s\n' "$crate"; fail=1; }
  done < <(git diff "$BASE".."$REF" -- Cargo.toml 2>/dev/null | grep -E '^\+\s+"crates/')

  while IFS= read -r name; do
    [ -z "$name" ] && continue
    [ "$(mark_i "$name")" = PURGED ] && { printf '  SKIP  CLI %s (purged by policy)\n' "$name"; continue; }
    check "CLI subcommand $name" "^[[:space:]]+${name}[[:space:]]*[({,]" src/cli/args.rs
  done < <(git diff "$BASE".."$REF" -- src/cli/args.rs 2>/dev/null \
             | grep -E '^\+[[:space:]]{4}[A-Z][A-Za-z]+[[:space:]]*[({,]' \
             | sed -E 's/^\+[[:space:]]+([A-Za-z]+).*/\1/' | sort -u)

  # A bare field name carries no subsystem: `bind_addr`, `port` and `endpoint`
  # all belong to GatewayConfig, which this fork purges. Classifying on the
  # field alone reports three false MISSes. Track the enclosing struct from the
  # diff context and classify on that instead.
  while IFS= read -r entry; do
    [ -z "$entry" ] && continue
    struct=${entry%%.*}; field=${entry#*.}
    if [ "$struct" = "?" ]; then
      printf '  REVIEW config field %s (enclosing struct not in diff context — classify by hand)\n' "$field"
      continue
    fi
    [ "$(mark_i "$struct")" = PURGED ] && { printf '  SKIP  config %s.%s (purged by policy)\n' "$struct" "$field"; continue; }
    check "config field $struct.$field" "pub ${field}:" crates/jcode-config-types/src/lib.rs
  done < <(git diff -U15 "$BASE".."$REF" -- crates/jcode-config-types/src/lib.rs 2>/dev/null | awk '
      /^[+ ]pub struct [A-Za-z_]+/ { s=$0; sub(/^[+ ]pub struct /,"",s); sub(/[^A-Za-z_].*/,"",s); next }
      /^@@/ { s="?"; next }
      /^\+[[:space:]]+pub [a-z_]+:/ {
        f=$0; sub(/^\+[[:space:]]+pub /,"",f); sub(/:.*/,"",f)
        print (s=="" ? "?" : s) "." f
      }' | sort -u)

  echo
  if [ "$fail" -eq 0 ]; then echo "FEATURE VERIFY: all KEEP items present"
  else echo "FEATURE VERIFY: ITEMS MISSING — upstream work was dropped in resolution"; fi
  exit $fail

else
  echo "unknown mode: $MODE (expected 'inventory' or 'verify')" >&2; exit 2
fi
