#!/usr/bin/env bash
# setup_friction_eval.sh - deterministic install / setup / retention friction
# scorecard.
#
# The TUI onboarding evaluator (onboarding_eval.rs) scores the in-app flow, but
# most first-run friction happens BEFORE the TUI: the installer, PATH
# persistence, and whether an upgrade quietly preserves the user's state. This
# script measures that surface deterministically, with no network and no real
# user data, by running the REAL scripts/install.sh inside a sandbox with a
# mocked release endpoint, then probing the result with REAL shells.
#
#   Section A  fresh-install PATH resolution - after one `curl | sh`-equivalent
#              install, does `jcode` resolve in a brand-new login/interactive
#              shell of every kind we claim to support (bash -l, bash -i,
#              sh -l, fish, zsh)? This is the exact "it wasn't on my PATH"
#              complaint, asked of the real rc files the installer wrote.
#   Section B  idempotency - three installs must leave exactly one PATH line
#              per rc file (no duplicate exports piling up run after run).
#   Section C  retention - an upgrade must preserve ~/.jcode config and auth,
#              keep both immutable version binaries (rollback stays possible),
#              and the launcher must serve the new version.
#
# Every case prints PASS/FAIL/SKIP with expected-vs-actual on failure. The
# composite is passed/(passed+failed); SKIPs (shell not installed) don't count
# against the score but are reported. Exits nonzero on any FAIL.
set -u

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
install_sh="$repo_dir/scripts/install.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

passed=0
failed=0
skipped=0
declare -a failures=()

pass() { passed=$((passed + 1)); printf 'PASS  %s\n' "$1"; }
skip() { skipped=$((skipped + 1)); printf 'SKIP  %s (%s)\n' "$1" "$2"; }
fail() {
  failed=$((failed + 1))
  failures+=("$1")
  printf 'FAIL  %s\n' "$1"
  printf '      expected: %s\n' "$2"
  printf '      actual:   %s\n' "$3"
}

check() { # check <name> <expected-desc> <actual-desc> <condition-exit-status>
  local name="$1" expected="$2" actual="$3" status="$4"
  if [ "$status" -eq 0 ]; then pass "$name"; else fail "$name" "$expected" "$actual"; fi
}

# ---------------------------------------------------------------------------
# Sandbox: mocked release endpoint + tools, identical shape to
# test_install_conversion.sh so both exercise the same installer code paths.
# ---------------------------------------------------------------------------
mkdir -p "$work/bin"

cat > "$work/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf '%s\n' "${EVAL_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${EVAL_UNAME_M:-x86_64}" ;;
  *) printf '%s\n' "${EVAL_UNAME_S:-Linux}" ;;
esac
EOF

cat > "$work/bin/curl" <<'EOF'
#!/usr/bin/env bash
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    --data) shift 2 ;;
    http*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  *jcode.sh/releases/latest/version) printf 'v%s\n' "${EVAL_VERSION:-1.2.3}" ;;
  *jcode.sh/releases/v*/download-bases)
    printf 'https://github.com/1jehuang/jcode/releases/download/v%s\n' "${EVAL_VERSION:-1.2.3}"
    ;;
  *SHA256SUMS)
    # Checksum of the deterministic fake archive written by the tar mock's
    # sibling below (the literal bytes "fake archive").
    printf '8d57abb57a0dae3ff23c8f0df1f51951b7772822e0d560e860d6f68c24ef6d3d  %s\n' \
      "${EVAL_CHECKSUM_ASSET:-jcode-linux-x86_64.tar.gz}"
    ;;
  *github.com*/releases/latest)
    printf 'https://github.com/1jehuang/jcode/releases/tag/v%s' "${EVAL_VERSION:-1.2.3}"
    ;;
  *github.com*/releases/download/*)
    [ -n "$output" ] || exit 2
    printf 'fake archive' > "$output"
    ;;
  *) exit 2 ;;
esac
EOF

cat > "$work/bin/tar" <<'EOF'
#!/usr/bin/env bash
dest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -C) dest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
artifact="${EVAL_ARCHIVE_ARTIFACT:-jcode-linux-x86_64}"
cat > "$dest/$artifact" <<BIN
#!/usr/bin/env bash
if [ "\${1:-}" = "--version" ]; then printf 'jcode ${EVAL_VERSION:-1.2.3}\n'; fi
exit 0
BIN
chmod +x "$dest/$artifact"
EOF
chmod +x "$work/bin/uname" "$work/bin/curl" "$work/bin/tar"

# Run the real installer into an isolated HOME. $1 = home dir, $2 = version.
run_install() {
  local home="$1" version="$2"
  mkdir -p "$home"
  EVAL_VERSION="$version" \
  PATH="$work/bin:/usr/bin:/bin" \
  HOME="$home" \
  XDG_CONFIG_HOME="$home/.config" \
  JCODE_HOME="$home/.jcode" \
  JCODE_SKIP_SERVER_RELOAD=1 \
  bash "$install_sh" 2>&1
}

# Probe: does `jcode` resolve and run in a fresh shell of the given kind, with
# only the sandbox HOME's rc files to set it up? PATH starts minimal (no
# ~/.local/bin) so resolution can only come from what the installer wrote.
probe_shell() { # probe_shell <home> <shell-cmd...>
  local home="$1"; shift
  HOME="$home" \
  XDG_CONFIG_HOME="$home/.config" \
  ENV="$home/.profile" \
  PATH="/usr/bin:/bin" \
  "$@" 'command -v jcode >/dev/null 2>&1 && jcode --version' 2>/dev/null </dev/null
}

echo "================ SETUP FRICTION SCORECARD ================"

# ---------------------------------------------------------------------------
# Section A: fresh install, then PATH resolution in real new shells.
# ---------------------------------------------------------------------------
echo ""
echo "-- Section A: fresh-install PATH resolution (real shells) --"
home_a="$work/home-a"
install_out=$(run_install "$home_a" "1.2.3")
install_status=$?
check "installer completes on a fresh home" \
  "exit 0" "exit $install_status" "$install_status"

launcher="$home_a/.local/bin/jcode"
[ -x "$launcher" ]; check "launcher exists and is executable" \
  "executable at ~/.local/bin/jcode" "missing or not executable: $launcher" "$?"

ver=$("$launcher" --version 2>/dev/null || true)
[ "$ver" = "jcode 1.2.3" ]; check "launcher runs and reports the installed version" \
  "jcode 1.2.3" "${ver:-<no output>}" "$?"

# The success message must not dead-end the user: either jcode is already
# resolvable or the copy explicitly says future shells will have it.
printf '%s' "$install_out" | grep -q "Run 'jcode' to get started\|Future terminal sessions will have jcode on PATH automatically"
check "install output gives a working next step (no dead end)" \
  "a 'run jcode' or 'future sessions' line" "neither line found in installer output" "$?"

probe_case() { # probe_case <label> <binary> <shell-cmd...>
  local label="$1" binary="$2"; shift 2
  if ! command -v "$binary" >/dev/null 2>&1; then
    skip "$label" "$binary not installed on this machine"
    return
  fi
  local out
  out=$(probe_shell "$home_a" "$@")
  [ "$out" = "jcode 1.2.3" ]
  check "$label" "jcode resolves and prints 'jcode 1.2.3'" "${out:-<not found on PATH>}" "$?"
}

probe_case "bash login shell (bash -lc) finds jcode"        bash bash -lc
probe_case "bash interactive shell (bash -ic) finds jcode"  bash bash -ic
probe_case "sh login shell (sh -lc) finds jcode"            sh   sh -lc
probe_case "fish shell (fish -c) finds jcode"               fish fish -c
probe_case "zsh login shell (zsh -lc) finds jcode"          zsh  zsh -lc

# ---------------------------------------------------------------------------
# Section B: idempotency - reinstalling must not stack PATH lines.
# ---------------------------------------------------------------------------
echo ""
echo "-- Section B: idempotency (3x install) --"
run_install "$home_a" "1.2.3" >/dev/null 2>&1
run_install "$home_a" "1.2.3" >/dev/null 2>&1

for rc in .bashrc .profile .zshenv .config/fish/config.fish; do
  file="$home_a/$rc"
  [ -f "$file" ] || continue
  # Each install appends one "# Added by jcode installer" stanza when missing;
  # a correct idempotency guard leaves exactly one after any number of runs.
  count=$(grep -cF "# Added by jcode installer" "$file" || true)
  [ "$count" -le 1 ]
  check "~/$rc has at most one jcode PATH stanza after 3 installs" \
    "<= 1 installer stanza" "$count installer stanzas" "$?"
done

# ---------------------------------------------------------------------------
# Section C: retention - upgrade preserves user state and rollback stays
# possible.
# ---------------------------------------------------------------------------
echo ""
echo "-- Section C: retention across upgrade --"
home_c="$work/home-c"
run_install "$home_c" "1.2.3" >/dev/null 2>&1

# Simulate accumulated user state between installs.
mkdir -p "$home_c/.jcode"
printf 'model = "kept"\n' > "$home_c/.jcode/config.toml"
printf '{"kept":true}\n' > "$home_c/.jcode/auth.json"

run_install "$home_c" "1.3.0" >/dev/null 2>&1

ver=$("$home_c/.local/bin/jcode" --version 2>/dev/null || true)
[ "$ver" = "jcode 1.3.0" ]; check "upgrade switches the launcher to the new version" \
  "jcode 1.3.0" "${ver:-<no output>}" "$?"

[ "$(cat "$home_c/.jcode/config.toml" 2>/dev/null)" = 'model = "kept"' ]
check "upgrade preserves ~/.jcode/config.toml" \
  "file unchanged" "missing or modified" "$?"

[ "$(cat "$home_c/.jcode/auth.json" 2>/dev/null)" = '{"kept":true}' ]
check "upgrade preserves ~/.jcode/auth.json" \
  "file unchanged" "missing or modified" "$?"

[ -x "$home_c/.jcode/builds/versions/1.2.3/jcode" ] && [ -x "$home_c/.jcode/builds/versions/1.3.0/jcode" ]
check "both immutable version binaries kept (rollback possible)" \
  "versions/1.2.3 and versions/1.3.0 both executable" \
  "$(ls "$home_c/.jcode/builds/versions" 2>/dev/null | tr '\n' ' ')" "$?"

stable_ver=$(cat "$home_c/.jcode/builds/stable-version" 2>/dev/null || true)
[ "$stable_ver" = "1.3.0" ]; check "stable channel marker points at the new version" \
  "1.3.0" "${stable_ver:-<missing>}" "$?"

# A second post-upgrade login shell must still resolve jcode (PATH survives
# upgrades, not just fresh installs).
out=$(probe_shell "$home_c" bash -lc)
[ "$out" = "jcode 1.3.0" ]; check "post-upgrade login shell still finds jcode" \
  "jcode 1.3.0" "${out:-<not found on PATH>}" "$?"

# Uninstall (default, no --purge) must remove binaries but KEEP user data, so
# a returning user's reinstall lands on their old config/auth. This is the
# retention contract of leaving: coming back is cheap.
uninstall_sh="$repo_dir/scripts/uninstall.sh"
# uninstall.sh pkills running jcode servers; neuter that inside the sandbox so
# the eval never touches real processes on the machine running it.
printf '#!/usr/bin/env bash\nexit 0\n' > "$work/bin/pkill"
chmod +x "$work/bin/pkill"
PATH="$work/bin:/usr/bin:/bin" \
HOME="$home_c" \
bash "$uninstall_sh" --yes >/dev/null 2>&1
uninstall_status=$?
check "uninstall (no --purge) completes" "exit 0" "exit $uninstall_status" "$uninstall_status"

[ ! -e "$home_c/.local/bin/jcode" ] && [ ! -d "$home_c/.jcode/builds" ]
check "uninstall removes launcher and build channels" \
  "launcher and builds gone" "still present" "$?"

[ "$(cat "$home_c/.jcode/config.toml" 2>/dev/null)" = 'model = "kept"' ] \
  && [ "$(cat "$home_c/.jcode/auth.json" 2>/dev/null)" = '{"kept":true}' ]
check "uninstall keeps config/auth for a cheap return" \
  "config.toml and auth.json intact" "user data lost" "$?"

# ---------------------------------------------------------------------------
# Scorecard.
# ---------------------------------------------------------------------------
echo ""
echo "-- SCORE --"
total=$((passed + failed))
if [ "$total" -gt 0 ]; then
  composite=$((passed * 100 / total))
else
  composite=0
fi
echo "cases passed  : $passed"
echo "cases failed  : $failed"
echo "cases skipped : $skipped (shell not installed; not scored)"
echo "COMPOSITE     : $composite / 100"
echo "=========================================================="

if [ "$failed" -gt 0 ]; then
  echo ""
  echo "failures:"
  for f in "${failures[@]}"; do echo "  - $f"; done
  exit 1
fi
