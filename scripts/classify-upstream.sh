#!/usr/bin/env bash
# Classify upstream commits by how much fork-specific judgement they need.
#
# Most upstream commits never touch purged code and merge automatically. This
# script tells you, BEFORE you resolve anything, how many actually need
# surgery — so you can budget the work and know which shas to hand-audit.
#
# Buckets:
#   PURE_KEPT   - touches no purged path, adds no purged identifier. The merge
#                 handles these. No action.
#   PURE_PURGED - touches ONLY purged paths. Our deletions win; expect `DU`
#                 conflicts resolved by `git rm`.
#   MIXED       - touches files we keep AND adds purged identifiers into them.
#                 This is the dangerous bucket: real feature and purged code in
#                 one commit. Every one needs a human decision (§3.3).
#   EMPTY       - no file changes (merge commits, empty commits).
#
# Usage, from inside the tmp clone (see §3.1):
#   ./scripts/classify-upstream.sh <base-sha> upstream/master
#   ./scripts/classify-upstream.sh "$BASE" upstream/master --list-mixed
#
# Patterns are kept in sync with scripts/purge-guard.sh. If you change one,
# change both.
set -uo pipefail

BASE="${1:?usage: classify-upstream.sh <base-ref> <upstream-ref> [--list-mixed]}"
REF="${2:?usage: classify-upstream.sh <base-ref> <upstream-ref> [--list-mixed]}"
LIST_MIXED=false
[ "${3:-}" = "--list-mixed" ] && LIST_MIXED=true

# Paths this fork deletes wholesale.
PURGED_PATHS='^(ios/|telemetry-worker/|\.github/|scripts/(remote|phone-server)/|scripts/[^/]*\.ps1$|crates/jcode-(telemetry-core|tui-permissions|gateway-types)/|crates/jcode-base/src/(gateway|sponsors)|crates/jcode-app-core/src/tool/discover|crates/jcode-setup-hints/src/windows_(setup|hotkeys)\.rs|crates/jcode-transport/src/windows\.rs|src/cli/telemetry\.rs|TELEMETRY\.md|docs/(SAFETY_SYSTEM|IOS_APP|WINDOWS)\.md)'

# Identifiers that must never enter a file we keep.
PURGED_IDENT='crate::telemetry::|jcode_telemetry_core|crate::gateway::|jcode_gateway_types|jcode_tui_permissions|crate::sponsors|DiscoverToolsTool|RequestPermissionTool|"integration_tools"|"request_permission"|Command::(Pair|Permissions)|handle_telemetry_command|SummaryPill::(Subscription|Telemetry)|TelemetryChoice|TelemetryLevel|record_[a-z_]+\(|windows_setup::|windows_hotkeys::|listen_windows_hotkey'

n_kept=0; n_purged=0; n_mixed=0; n_empty=0
mixed_shas=()

while read -r sha; do
  [ -z "$sha" ] && continue
  paths=$(git show --name-only --format= "$sha" 2>/dev/null | sed '/^$/d')
  if [ -z "$paths" ]; then
    n_empty=$((n_empty + 1)); continue
  fi
  kept_paths=$(printf '%s\n' "$paths" | grep -vE "$PURGED_PATHS")
  if [ -z "$kept_paths" ]; then
    n_purged=$((n_purged + 1)); continue
  fi
  # Added lines in files we keep, excluding the diff header (+++ path).
  added=$(git show --format= --unified=0 "$sha" -- $(printf '%s\n' "$kept_paths" | tr '\n' ' ') 2>/dev/null \
            | grep '^+' | grep -v '^+++')
  if printf '%s\n' "$added" | grep -qE "$PURGED_IDENT"; then
    n_mixed=$((n_mixed + 1)); mixed_shas+=("$sha")
  else
    n_kept=$((n_kept + 1))
  fi
done < <(git rev-list --reverse "$BASE..$REF" 2>/dev/null)

total=$((n_kept + n_purged + n_mixed + n_empty))
printf '\n%s..%s\n' "$BASE" "$REF"
printf '  PURE_KEPT   %4d   merge handles these\n' "$n_kept"
printf '  MIXED       %4d   NEEDS SURGERY (§3.3)\n' "$n_mixed"
printf '  PURE_PURGED %4d   our deletions win (DU -> git rm)\n' "$n_purged"
printf '  EMPTY       %4d\n' "$n_empty"
printf '  ---------------------\n'
printf '  TOTAL       %4d\n\n' "$total"

if [ "$LIST_MIXED" = true ] && [ "${#mixed_shas[@]}" -gt 0 ]; then
  echo "MIXED commits — audit each by hand:"
  for s in "${mixed_shas[@]}"; do
    printf '  %s  %s\n' "$(git rev-parse --short "$s")" "$(git log -1 --pretty=%s "$s")"
  done
  echo
fi
