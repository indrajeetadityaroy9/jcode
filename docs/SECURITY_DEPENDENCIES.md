# Dependency Security Triage

Last reviewed: 2026-09-05

This file tracks the current `cargo audit` findings for jcode and the intended remediation path.
It is not an allowlist. It is a triage record so advisories are visible and actionable.

## Current advisories

| Advisory | Crate | Dependency path | Affected area in jcode | Triage | Planned action |
|---|---|---|---|---|---|
| `RUSTSEC-2025-0141` | `bincode` | `syntect -> bincode` | Markdown/code highlighting in the TUI | Unmaintained transitive dependency. No direct exposure in the provider/auth flow. | Track `syntect` upgrades or replace `syntect` if upstream does not move off `bincode` soon. |
| `RUSTSEC-2024-0436` | `paste` | `tokenizers -> paste`, `tikv-jemalloc-ctl -> paste`, `macro_rules_attribute -> paste` | Tokenizers/embedding support, jemalloc stats, macro helper | Unmaintained transitive dependency. `ratatui` and `tract-*` no longer pull it. | Prefer upstream dependency upgrades before any local workaround. Re-evaluate after bumping `tokenizers`. |
| `RUSTSEC-2026-0002` | `lru` | `ratatui-core -> lru` | TUI rendering/cache internals | Unsoundness warning in a UI dependency. Not in auth/provider logic, but still ships in-process. | Upgrade `ratatui` / `ratatui-image` together once compatible. |
| `RUSTSEC-2026-0097` | `rand` | three majors coexist: `0.9.3` (all `jcode-*` crates, `tokenizers`, `quinn-proto`), `0.10.1` (`tract-onnx-opl`, `typespec_client_core`, `lopdf`), `0.8.5` (`ratatui-image`, `tungstenite`, `phf_generator`) | Azure auth, websocket, embedding, and UI transitive paths | Unsoundness warning involving custom loggers using `rand::rng()`. Jcode does not intentionally use that pattern, but the crate is broad in the graph. | Prefer upstream upgrades onto a single `rand` major. |
| `RUSTSEC-2026-0098` | `rustls-webpki` | `rustls` dependency stack | TLS certificate validation in rustls consumers | Name constraints for URI names incorrectly accepted. Transitive via TLS libraries. | Upgrade rustls/webpki stack when compatible releases are available. |
| `RUSTSEC-2026-0099` | `rustls-webpki` | `rustls` dependency stack | TLS certificate validation in rustls consumers | Name constraints accepted for wildcard certificates. Transitive via TLS libraries. | Upgrade rustls/webpki stack when compatible releases are available. |
| `RUSTSEC-2026-0104` | `rustls-webpki` | `rustls` dependency stack | TLS certificate revocation list parsing | Reachable panic in CRL parsing. Transitive via TLS libraries. | Upgrade rustls/webpki stack when compatible releases are available. |

## Priority order

1. `rustls-webpki` TLS advisories via rustls stack
2. `lru` via `ratatui-core`
3. `bincode` via `syntect`
4. `paste` / `rand` via multiple transitive dependencies

## Notes

- None of the advisories above were introduced by the provider-auth refactor.
- The provider/auth hardening work should continue independently of these dependency upgrades.
- `RUSTSEC-2026-0217` (`tract-nnef` 0.21.10, integer overflow in the NNEF tensor
  parser) was resolved on 2026-07-30 by moving `jcode-embedding` to `tract` 0.23.
  The in-line `0.21.16` fix was unreachable at the time: `tract-data 0.21.16`
  pins `half =2.4.1` while `naga` (via `vello` in the since-removed desktop
  app) required `half ^2.5`. The 0.23 line drops that pin. This mattered
  because the parser runs over a model downloaded at runtime rather than one
  shipped in the binary, so an ignore would not have been clearly safe. See
  #657.
- `RUSTSEC-2024-0320` (`yaml-rust`) was removed from the dependency graph on 2026-03-05 by trimming `syntect` features to built-in syntax/theme dumps instead of YAML loading.
- `RUSTSEC-2026-0194` / `RUSTSEC-2026-0195` (`quick-xml` 0.39.2): reached only through `wayland-scanner`, a build-time proc-macro in the desktop app's winit stack. Both `quick-xml` and `wayland-scanner` left the dependency graph entirely when the desktop app was removed, so the advisories are no longer reachable and their `--ignore` entries were dropped from `scripts/security_preflight.sh`. If either package ever reappears, the advisories will fail CI again by design.
- `RUSTSEC-2026-0141` (`lettre`), `RUSTSEC-2023-0086` (`lexical-core` via `imap-proto`) and `RUSTSEC-2026-0049` (CRL Distribution Point matching, which needed rustls-webpki >=0.103.10) all left the dependency graph when the email notification channel was removed: `crates/jcode-notify-email` was the only consumer of `lettre`, `imap`, and `mail-parser`, and `imap`/`rustls-connector` held the last rustls 0.22 stack. The graph now resolves a single `rustls-webpki 0.103.13`, so the three advisories are unreachable and their `--ignore` entries were dropped from `scripts/security_preflight.sh`. If any of those packages reappear, the advisories will fail CI again by design.
- `RUSTSEC-2026-0187` (`lopdf`, stack overflow on deeply nested PDF objects) no longer applies: the graph now resolves `pdf-extract 0.12.0 -> lopdf 0.42.0`, and `>=0.42` is the fixed line. The doc previously claimed `pdf-extract 0.8.2` pinned `lopdf 0.34`; that stopped being true after the `pdf-extract` bump, so the row and its `--ignore` entry were dropped from `scripts/security_preflight.sh`.
- `scripts/security_preflight.sh` ignores only the `rustls-webpki` advisories triaged above so CI can remain actionable. New vulnerabilities still fail CI by default.
- Before changing dependency versions, run:
  - `cargo check`
  - `cargo test -j 1`
  - `scripts/security_preflight.sh`
