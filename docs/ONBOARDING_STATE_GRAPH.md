# Robust Onboarding: An Explicit State-Space Graph

Status: partially implemented (steps 1, 2, 5 landed; 3 and 4 not started; see §4)
Owner: onboarding
Related code: `crates/jcode-tui/src/tui/app/onboarding_flow.rs`,
`onboarding_flow_control.rs`, `onboarding_graph.rs`, `onboarding_repair.rs`,
`onboarding_sim.rs`,
`crates/jcode-tui/src/tui/app/tests/{onboarding_eval,onboarding_golden}.rs`,
`crates/jcode-base/src/auth/{env_facts,login_diagnostics,refresh_state,status_types}.rs`

---

## 1. Why onboarding keeps breaking

Onboarding is not one flow. It is a **product of independent state spaces** that we
currently model only partially and in three different places:

| Axis | Values (roughly) | Where it lives today |
| --- | --- | --- |
| UI phase | `Login`, `LoginOpenAi`, `ModelSelect`, `ContinuePrompt`, `StartChoice`, `Suggestions`, `Done` (7 variants, `onboarding_flow.rs:275-318`) | `OnboardingPhase` |
| Credential state, per provider | absent / present / verified / expired / **permanently rejected** | scattered: `auth-refresh-state.json`, `AuthStatus`, ad-hoc strings |
| Environment capability | tty? browser? bindable port? writable config dir? container? proxy? | probed up front by `auth::env_facts` (§2.3); network and clock skew are still discovered *by failing*, then string-matched in `classify_auth_failure_message` (`login_diagnostics.rs:38`) |
| Import candidates | 5 external CLIs x present/absent/importable/stale | `ImportReview` |
| Transport mode | local / client-server / remote / sandbox | implicit |

The bug in today's log is exactly a cross-axis bug: OpenAI's refresh token was
**permanently invalidated** (`refresh_token_invalidated`, Jul 31 16:54 last success),
but the *credential axis* has no terminal `Rejected` state for OpenAI, so the catalog
sweep force-refreshed a dead token every ~15 minutes for two days. Claude *does* have
that state (`rejected_refresh_fingerprint` in `auth-refresh-state.json`) because someone
hit the bug there once and patched it locally. That is the tell: **we are patching cells
of a matrix we have never written down.**

Second tell from the same log: the UI said `GitHub Copilot - login expired` while every
`auth_status_check_fast` line said `copilot=not_configured`. Two code paths derive a
user-facing label from different notions of "credential state". A single enum makes that
class of bug unrepresentable.

So: the goal is not "fix onboarding". It is **make the state space explicit, make every
state reachable in tests, and make real-world traversals observable.**

---

## 2. The model

### 2.1 A node is a (phase, facts) pair, not just a phase

```rust
/// Everything the transition function is allowed to read. Pure data, cheap to
/// clone, cheap to construct in tests, and serializable so a real user's trace
/// can be replayed offline.
pub struct OnboardingWorld {
    pub env: EnvFacts,                     // capability probe results
    pub creds: BTreeMap<ProviderId, CredState>,
    pub imports: ImportFacts,
    pub transport: Transport,
}

pub struct OnboardingNode {
    pub phase: Phase,
    pub world: OnboardingWorld,
}
```

Transitions are a **pure function**:

```rust
fn step(node: &OnboardingNode, ev: Event) -> Transition
// Transition { next: Phase, effects: Vec<Effect>, edge: EdgeId }
```

`Effect` is a *description* of side effects (`OpenBrowser`, `BindCallbackPort(u16)`,
`WriteCreds`, `SpawnValidationPing`), never the side effect itself. That single
change is what makes the whole thing testable: the test harness executes effects
against a simulated world, production executes them against the real one, and
**both traverse identical edges**. `onboarding_sim.rs` today re-seeds phases by hand
and therefore can drift from the live flow; under this model the sim is just a
different `Effect` interpreter.

**Not built.** There is no `OnboardingWorld`, `OnboardingNode`, `ImportFacts`,
`Transport`, `Effect`, or `step()` in the tree; the live flow is still
`onboarding_flow_control.rs`'s methods mutating `App`. What exists is the
*descriptive* half: `onboarding_graph.rs` has the closed node/edge vocabulary
(`NodeId`, `onboarding_graph.rs:36-66`; `EdgeId`, `:110`) and a `Violation`
type (`:583`), but no transition function and no effect interpreter, so the
graph is a separate model of the flow rather than the flow itself. Keeping
those two in sync is manual today.

### 2.2 Credential state gets a real lifecycle

This is the highest-value single change and it directly fixes today's bug.

```
        ┌──────────────── refresh ok ────────────────┐
        v                                            │
   Absent ──login──> Present ──verify ok──> Verified ─┴─ expiry ─> Stale
      ^                                       │                    │
      │                                  server 401             refresh
      │                                       v                    │
      └──── re-login ─────────────── Rejected(fingerprint) <────────┘   [TERMINAL]
```

As shipped, `CredState` (`crates/jcode-base/src/auth/refresh_state.rs:20-32`)
is exactly `Absent | Present | Verified | Stale | Rejected`. The
`Unusable(reason)` state this section originally sketched was not built:
classified failure detail lives in `AuthFailureReason`
(`login_diagnostics.rs`) rather than in the credential enum, and the only
terminal state is `Rejected`.

Rules that fall out for free:

- `Rejected` is **terminal for that credential fingerprint**. No background sweep,
  catalog refresh, or retry may attempt it again. Only a *new* fingerprint (a real
  re-login) clears it. This landed for every provider:
  `record_permanent_rejection` (`refresh_state.rs:200-214`) stores
  `rejected_refresh_fingerprint` and `ensure_refresh_allowed`
  (`refresh_state.rs:105`) is the guard callers take first. The fingerprint is
  a `DefaultHasher` digest of the trimmed token
  (`refresh_state.rs:190-196`), not the `sha256(...)[..8]` this section
  originally specified; it never stores the token, but it is not a
  cryptographic hash and `DefaultHasher` output is not guaranteed stable
  across Rust releases even though the value is persisted to
  `auth-refresh-state.json`.
- The UI label is a `match` on this enum, one function, one place. **Landed** for
  the onboarding credential hint (`onboarding_flow_control.rs:1349-1362`);
  `not_configured` can no longer render there as "login expired".
- Fallback ranking (`Ctrl+Y` to Gemini) becomes a sort over `CredState`, not a pile
  of conditionals. *Not built.*
- `provider bootstrap` gets a precondition:
  `debug_assert!(!matches!(state, Rejected))`. *Not built* — the guard is the
  explicit `ensure_refresh_allowed` call, not an assertion.

### 2.3 Environment capabilities are probed, not discovered by failing

`classify_auth_failure_message` is a 50-line string matcher over English error text.
It works, but it runs *after* we have already burned the user's first 90 seconds on a
flow that could never succeed. Invert it:

```rust
// As shipped: crates/jcode-base/src/auth/env_facts.rs:67-86
pub struct EnvFacts {
    pub tty: Tri,               // interactive stdin/stdout
    pub browser: Tri,           // a launcher exists and, on Linux, a display server
    pub loopback_bind: Tri,     // can we bind a loopback socket for the callback
    pub config_writable: Tri,   // jcode config dir exists (or can be created) and is writable
    pub container: Tri,         // container/SSH/remote shell -> redirects land on the wrong machine
    pub proxy: Tri,             // HTTP(S) proxy configured; changes failure modes
}
```

Six fields, not the nine this section originally listed. `network` and
`clock_skew_ok` were deliberately dropped from the probe because they need a
provider round-trip and therefore belong to the login attempt, not to a
startup probe (`env_facts.rs:92-94`); `keyring` was never added.

`Tri = Yes | No | Unknown` (`env_facts.rs:27-32`), probed by syscall/env lookup
only, memoized per boot at the one call site
(`auth::browser_unusable_here`, `crates/jcode-base/src/auth/mod.rs:126-133`).
Then **method selection is a lookup, not a hope**
(`preferred_auth_method`, `env_facts.rs:115-139`):

| Facts | Chosen auth method | `AuthMethodChoice` |
| --- | --- | --- |
| config_writable=No | Refuse to start login; explain the real problem first | `BlockedConfigUnwritable` |
| tty=No | API key from env/stdin, otherwise fail *fast* with a copyable command | `ApiKeyNonInteractive` |
| browser=No, or container=Yes without a confirmed browser | Device code flow | `DeviceCode` |
| browser ok, loopback=No | OAuth with paste-back callback URL | `OAuthPasteCallback` |
| browser ok, loopback ok | OAuth loopback (best) | `OAuthLoopback` |

The table is ordered most-blocking-first, matching the function. Two rows from
the original sketch are absent: there is no `clock_skew_ok=No`
"fix-the-clock" outcome (no clock probe), and `--print-auth-url` is not part
of the `DeviceCode` choice. `BlockedConfigUnwritable` and
`ApiKeyNonInteractive` carry user-facing `precondition_message()` text
(`env_facts.rs:174-184`); the other three carry none.

Every one of these is a documented node with a documented recovery edge. Today most
of them are a generic error toast plus `onboarding_repair.rs`'s "ask another AI agent
to fix it", which is a great last resort and a bad first resort.

### 2.4 Invariants the graph must satisfy (checked by a gate, not by review)

These are the payoff. Once the graph is data, you can assert over it:

1. **No dead ends.** Every non-terminal node has ≥1 outgoing edge reachable by a key
   the user can actually press, and that edge is named on screen.
2. **Every failure node has a recovery edge** that is not "restart jcode".
3. **Bounded work.** `max steps-to-ready ≤ N` and `max keystrokes ≤ K` over all paths
   (Tier 1 of `onboarding_eval.rs` already counts this; the graph makes it exhaustive
   instead of authored-by-hand).
4. **Reachability.** Every node is reachable from `Start` under *some* `EnvFacts`, and
   any node reachable under *no* `EnvFacts` is dead code and must be deleted.
5. **Escape hatch everywhere.** Every node accepts `Esc`/skip and lands in a usable
   app, possibly degraded. Nobody is ever trapped in first-run.
6. **Progress.** No cycle without a user-visible state change (kills retry loops like
   the one in today's log).
7. **Terminal-state respect.** No effect targets a provider in `Rejected`.

Enforcement, as shipped: `check_invariants`
(`crates/jcode-tui/src/tui/app/onboarding_graph.rs:592`) walks the 12-node
graph (`NodeId::all()`, `onboarding_graph.rs:90-105`) and returns a
`Vec<Violation>`; the `onboarding_graph::` tests are a gate in
`scripts/check_guardrails.sh:98-99`. This fork has no `.github/` CI, so "the
gate" means that script, run locally or by whoever is landing the change.

The env fact space is 3^6 = 729 (six `Tri` fields); the exhaustive
method-selection test walks 3^5 = 243 of it, holding `proxy` out because it
does not participate in the decision (`env_facts.rs:372-414`).

Not built: the `proptest` model-based test that would drive random event
sequences against the real `App` and assert the `App`'s state always equals
the model's. There is no `proptest` dependency in this workspace. The
anti-drift check today is the wildcard-free `match` in
`classify_phase_surface`
(`crates/jcode-tui/src/tui/app/tests/onboarding_eval.rs:54`), which fails to
compile when a phase is added but does not prove the two models agree.

---

## 3. Runtime robustness policies the graph makes expressible

Once states are explicit, the fixes for today's log are one-liners rather than
whack-a-mole. Only the first of these has landed:

- **Terminal-rejection guard** (fixes the 2-day retry loop): background sweeps filter
  providers by `CredState`, skipping `Rejected`. Applies to OpenAI, Copilot, Cursor,
  Gemini, all of them, because it is a property of the state, not the provider.
  *Landed*, though as three separate call sites rather than one sweep filter:
  `ensure_refresh_allowed` in the OAuth refresh path (`auth/oauth.rs:1237`),
  and `refresh_token_is_known_rejected` in `auth::mod` (`:952`) and the
  Anthropic runtime (`crates/jcode-provider-anthropic-runtime/src/lib.rs:993`).
- **Circuit breaker per (provider, effect)**: exponential backoff with a cap, and a
  hard stop on terminal classifications. *Not built* as a general mechanism.
- **Degraded-ready is a first-class outcome**: if *any* provider is `Verified`, the
  user reaches a working app and the broken provider becomes a dismissible task, not
  a blocking screen. Today's session did offer the Gemini fallback, which is the right
  instinct; make it the default path rather than a `Ctrl+Y` hint after a hard stop.
  *Not built.*
- **Idempotent, atomic credential writes**: write-temp + rename + fsync, with the
  `.bak` rotation that already exists, so a crash mid-login can never produce a
  half-written `openai-auth.json`. *Landed* in the shared storage writer
  (`crates/jcode-storage/src/lib.rs:530-598`): pid+nonce temp file,
  owner-only permissions before any secret bytes, `sync_all`, hard-linked
  `.bak`, then `rename` over the destination.
- **Self-check on boot**: run the invariant checks against the *live* world and log
  (locally) any violated invariant. Cheap, and it catches drift in the field.
  *Not built* — `check_invariants` has no production caller; the module carries
  `#![cfg_attr(not(test), allow(dead_code))]` precisely because of that
  (`onboarding_graph.rs:23-27`).

---

## 4. Implementation plan (incremental, no big bang)

The existing code is in decent shape; this is mostly consolidation.

1. **`CredState` enum + universal rejection fingerprints.** *Landed.*
   `auth::refresh_state::CredState` (`refresh_state.rs:20-32`) is the lifecycle
   in §2.2, and every provider with a refresh flow records outcomes through
   `record_refresh_outcome` — claude and openai (`auth/oauth.rs:1175`,
   `:1323`), gemini (`auth/gemini.rs:322`), cursor (`auth/cursor.rs:623`), and
   antigravity (`auth/antigravity.rs:219`) — so no refreshing provider can
   silently opt out of terminal rejection. (Copilot has no refresh token, so
   there is nothing to record.) `ensure_refresh_allowed`
   (`refresh_state.rs:105`) is the guard callers use before spending a
   round-trip.
2. **`EnvFacts` probe.** *Landed.* `auth::env_facts` probes tty, browser,
   loopback bind, writable config, container, and proxy with nothing but
   syscalls and env lookups; the budget test asserts under 500 ms
   (`env_facts.rs:443-449`). The §2.3 selection table is tested exhaustively
   over the 3^5 decision-relevant fact space (`env_facts.rs:372-414`). It is
   wired into `auth::browser_suppressed` (`auth/mod.rs:109-133`), so a machine
   that positively cannot use a browser skips straight to a device/paste flow
   instead of waiting out a callback timeout.
3. **Extract the transition table.** *Not started.* Move the logic in
   `onboarding_flow_control.rs` (1652 lines) behind `step(node, ev) ->
   Transition`, keeping current behavior byte-identical; the golden tests in
   `tests/onboarding_golden.rs` are the safety net.
4. **Effect interpreter split.** *Not started.* Live interpreter + sim
   interpreter; would delete the hand-seeded phase list in
   `onboarding_sim.rs`, which is still there.
5. **Invariant tests.** *Landed.* `onboarding_graph.rs` declares the graph as
   data — 12 nodes including the `EnvBlocked`, `LoginFailed`, and
   `CredRejected` states the flow always had but never modelled
   (`onboarding_graph.rs:32-66`) — and `check_invariants`
   (`onboarding_graph.rs:592`) enforces the §2.4 properties. Wired into
   `scripts/check_guardrails.sh:98-99`.
6. **Method selection from `EnvFacts`.** *Partially landed.*
   `preferred_auth_method` computes all five outcomes, but the only consumer is
   `browser_unusable_here` (`auth/mod.rs:126-133`), which collapses them to a
   single browser/no-browser bit. Nothing yet distinguishes device-code from
   paste-callback at the login site, and `precondition_message()` is not
   surfaced anywhere.

Rough ordering principle: every step is independently shippable and independently
valuable, and steps 1 and 5 alone would have prevented both bugs visible in the
log that prompted this document.

---

## 5. Risks

- **Over-abstraction.** A state machine framework that is harder to read than the
  conditionals it replaced is a net loss. Mitigation: the transition table must be
  readable as a table by a person who has never seen the code. If it isn't, stop.
- **Probe flakiness.** A wrong `browser=No` sends users down a worse path than
  failing forward would have. Mitigation: `Unknown` biases toward the optimistic path,
  and step 2 validates probes against reality before they gate anything.
- **Closed vocabulary drift.** The node/edge labels are the graph's public
  surface: if a label becomes free text, replaying and checking traversals
  stops working. Mitigation: closed vocabulary enforced by construction and by
  the invariant tests.
