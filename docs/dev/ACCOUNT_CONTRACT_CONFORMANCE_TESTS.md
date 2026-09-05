# Account Contract Conformance Tests and Test Vectors

Status: **proposal.** None of the fixtures, harnesses, or vectors described
here exist. There is no `tests/fixtures/account-contract/`, no
`manifest.json`, and no vector-driven test; the existing device-login tests
are hand-written cases against a scripted server. The "Grounding" section
below has been re-derived from the current code (2026-09-05); the DL/ME/CK
tables have **not** been re-derived, and several of their expectations no
longer match the client — see "Vectors that contradict the current client".

Executable conformance design for the jcode subscription account contract:
device login, browser approval/denial, account state (`/v1/me`), checkout,
billing portal, webhook ordering, revocation, and mixed-version compatibility.

The client half of the contract lives in this repo; the server half lives in
the private `solosystems-backend` repo. This document defines the shared test
vectors, the harnesses that execute them on each side, and who owns what.

## Grounding (current code)

The flow was rewritten **browser-first**: no email and no secret is ever typed
into the terminal. CLI orchestration is `src/cli/login/jcode_device.rs` (225
lines); all protocol parsing, HTTP behavior, and redaction live in
`crates/jcode-base/src/subscription_api.rs` so the CLI and TUI share one
contract (`jcode_device.rs:1-4`).

- **Device authorization**: `request_device_authorization`
  (`subscription_api.rs:236-288`).
  `POST {api_base}/auth/device` with body `{"client_name":"jcode-cli"}` plus an
  optional `requested_tier` (`:242-245`) — **no `email` field**. Response is
  parsed into `DeviceAuthorization { device_code, flow_id, verification_uri,
  verification_uri_complete, expires_in, interval }` (`:67-77`);
  `device_code`, `flow_id`, and both URIs are **required** and a missing one
  is `InvalidResponse` (`:272-284`). A body carrying the legacy `verify_url`
  without `verification_uri_complete` is rejected as `LegacyBackend`
  (`:269-271`, wire type at `:175-192`). Defaults and clamps:
  `expires_in` defaults to **600** and clamps to `1..=3600`; `interval`
  defaults to **3** and clamps to `1..=60` (`:285-286`). Non-success maps
  401 -> `Unauthorized`, 403 -> `Forbidden`, 404 -> `LegacyBackend`, else
  `Http { status, code }` (`:256-264`). Request timeout
  `DEVICE_REQUEST_TIMEOUT` = 15s (`:15`).
- **Token exchange**: `poll_device_token_once`
  (`subscription_api.rs:290-353`) -> `TokenPollOutcome` (`:88-95`).
  `POST {api_base}/auth/token {"device_code"}`, then:
  429 -> `SlowDown { retry_after }` parsed from the `Retry-After` header
  (`:304-313`); 428 or 202 -> `Pending` (`:315-317`); 2xx whose body carries
  code `authorization_pending|pending` -> `Pending` (`:319-324`); other 2xx ->
  `Approved`, requiring a non-empty `api_key` and **required**
  `account_id`/`email`/`tier`/`status` (`:325-336`, `ApprovedAccountKey` at
  `:79-86`, wire at `:194-201`). Otherwise by error code:
  `authorization_pending|pending`, `slow_down`,
  `expired_token|expired|expired_device_code`, `access_denied|denied`
  (`:340-344`); then 401/403/404 -> `Unauthorized`/`Forbidden`/`LegacyBackend`;
  else `Http { status, code }` (`:345-352`). The error code is read from
  either `{"error":"code"}`, `{"error":{"code":...}}`, or a top-level
  `{"status":...}`, truncated to 80 chars (`ErrorEnvelope`, `:150-173`,
  `error_code`, `:229-234`).
- **Poll loop**: `poll_for_api_key`
  (`src/cli/login/jcode_device.rs:27-92`). Base delay
  `Duration::from_secs(interval.max(1))`; deadline
  `now + expires_in.max(interval.max(1))`; retry timing is
  `PollingBackoff` (`subscription_api.rs:470-509`), where `slow_down` uses the
  server's `Retry-After` or adds 5s, floored at base and **capped at 60s**
  (`:491-496`), and transport errors double up to 30s (`:498-500`).
  Ctrl-C cancels via `tokio::select!` on the delay only (`:52-58`):
  cancellation is deliberately *not* polled while an exchange request is in
  flight, so a one-time credential the backend already consumed is never
  stranded (rationale at `jcode_device.rs:60-64`). `Expired` and `Denied` bail
  with distinct messages (`:77-82`).
- **Credential persistence**: `persist_approved_key`
  (`src/cli/login/jcode_device.rs:95-104`) ->
  `subscription_catalog::persist_account_credentials`
  (`crates/jcode-base/src/subscription_catalog.rs:406-426`), writing
  `JCODE_API_KEY`, `JCODE_ACCOUNT_ID`, `JCODE_ACCOUNT_EMAIL`, `JCODE_TIER`
  (consts at `:3-7`) into the jcode-subscription env file, then
  `ensure_account_credential_permissions` (`:455-473`) — which hardens *and
  verifies* owner-only mode, erroring when `mode & 0o077 != 0`. An empty key
  is refused outright (`:412-415`). So SN-06 is already enforced in
  production, not just testable.
- **Hosted-billing activation** (new stage, unmodelled by the vectors below):
  after persisting the key the flow polls `poll_for_paid_activation`
  (`subscription_api.rs:429-468`) for up to `ACTIVATION_TIMEOUT` = 10 minutes
  (`:16`), yielding `ActivationOutcome::{Active, Canceled, TimedOut {
  last_error_was_offline }, Revoked, Denied}` (`:97-104`). `Revoked`/`Denied`
  clear local credentials and fail; every other non-active outcome keeps the
  valid key and prints recovery actions (`jcode account status|manage|logout`,
  `jcode_device.rs:217-221`). The flow returns
  `LoginCompletion::{Active, KeySavedPlanPending, CanceledBeforeApproval}`
  (`jcode_device.rs:14-19`).
- **Account state client**: `fetch_subscription_me_with`
  (`subscription_api.rs:355-385`) and `fetch_subscription_me`
  (`:388-398`) -> `SubscriptionMe` (`:38-65`, with `parsed_tier()`,
  `has_active_paid_plan()`, `checkout_was_canceled()`), `ME_FETCH_TIMEOUT` =
  5s (`:14`). A successful fetch persists the parsed tier (`:381`).
- **Tier gating**: `effective_tier()` =
  `cached_tier().unwrap_or(JcodeTier::Plus)`
  (`crates/jcode-base/src/subscription_catalog.rs:356-358`), so an
  unknown/absent tier still gates like Plus — but its own doc comment says the
  value is legacy and metered hosted billing does **not** use it for
  client-side model gates (`:353-355`).
- **Revocation client call**: `revoke_current_key`
  (`subscription_api.rs:400-425`); local teardown is
  `subscription_catalog::clear_account_credentials` (`:431-442`).
- **Redaction is a type property**: `AccountApiError`
  (`subscription_api.rs:106-115`) keeps only a status and a bounded code —
  "Response bodies and bearer values are never retained" (`:106`) — with
  `is_temporary()` classifying retryable transport failures (`:118-121`).
- **Existing executable harness to extend**: `spawn_scripted_http_server`
  (`src/cli/login/jcode_device/tests.rs:7-29`) plus `test_client` (`:31-37`).
  The five current cases are `polling_pending_slow_down_then_approval`
  (`:39`), `polling_denied_has_clear_redacted_error` (`:72`),
  `polling_timeout_is_deterministic_before_first_request` (`:94`),
  `cancellation_during_consumed_exchange_finishes_and_returns_the_key`
  (`:109`), and
  `approved_key_persistence_is_owner_only_and_clear_is_deterministic`
  (`:138`). There are no `poll_state_machine_*` tests.
- **Browser opening**: `maybe_open_browser`
  (`src/cli/login.rs:1036`), called with `device.verification_uri_complete`
  (`jcode_device.rs:121`).
- **Auth failure classification** for negative-path assertions:
  `crates/jcode-base/src/auth/login_diagnostics.rs`
  (`classify_auth_failure_message`, `:38`).

## Vectors that contradict the current client

These rows below were written against the pre-rewrite flow and would fail as
specified. Fix the vector, not the code, unless noted:

- **DL-01/DL-02 defaults.** The defaults are 600/3, not 900/5
  (`subscription_api.rs:285-286`), and both are clamped.
- **DL-07 "gone".** 404 maps to `LegacyBackend`, not `Expired`
  (`subscription_api.rs:347`), and 410 has no special case at all — it falls
  into `Http { status, code }`. Decide which the contract wants; the client
  currently distinguishes "old backend" from "expired code" on purpose.
- **DL-09 empty api_key.** Still correct, but the check is
  `api_key.trim().is_empty()` inside the approved branch
  (`subscription_api.rs:327-329`) *and* again in
  `persist_account_credentials` (`subscription_catalog.rs:412-415`).
- **DL-12 device reject.** 401/403/404 are distinct typed errors, not one
  "abort" (`subscription_api.rs:256-264`).
- **CK-05.** The device flow never prints a pricing prompt.
  `JCODE_PRICING_URL` (`subscription_catalog.rs:12`) has no callers anywhere in
  the tree; the login tail prints `jcode account status|manage|logout` instead
  (`jcode_device.rs:217-221`). Either drop CK-05 or respecify it against
  `print_recovery_actions`.
- **Missing vector family.** Nothing here covers `ActivationOutcome`. The
  post-approval activation poll is where a user now spends most of the login,
  and its five outcomes (including "key saved, plan pending") are the states
  support will actually be asked about.
- **SN-06 is already enforced**, so it is a regression test to keep rather than
  a gap to close (`subscription_catalog.rs:455-473`,
  `jcode_device/tests.rs:138`).

## Repository ownership

| Concern | Owner repo | Harness |
|---|---|---|
| Client poll state machine, persistence, `/v1/me` parsing, tier gating | jcode (this repo) | Rust unit/integration tests against scripted HTTP server |
| Shared wire test vectors (JSON fixtures) | jcode, mirrored into solosystems-backend by version tag | `tests/fixtures/account-contract/` (proposed) |
| Email delivery, approval/denial web page, checkout session creation, Stripe webhooks, key revocation, `/v1/me` truth | solosystems-backend (private) | Backend integration tests replaying the same fixtures against real handlers |
| End-to-end smoke (live staging) | solosystems-backend CI; on the jcode side an opt-in local script gated on staging creds (this fork has no `.github/` CI, so there is no jcode CI job to add) | `jcode account login` scriptable flow against staging `JCODE_API_BASE` |

Rule: a fixture change is a contract change. Fixtures are versioned
(`schema_version` field per vector file); both repos pin the fixture set and a
mixed-version matrix (below) proves old clients still pass against new server
vectors and vice versa.

## Fixture layout (proposed, this repo)

```
tests/fixtures/account-contract/
  v1/
    device_auth/           # responses to POST /v1/auth/device
    token_poll/            # scripted sequences for POST /v1/auth/token
    me/                    # GET /v1/me bodies
    webhook_order/         # backend-only, mirrored for documentation
    manifest.json          # {schema_version, vectors: [...]}
```

Each vector: `{name, request, response_script: [(status, body)...],
expected_outcome, notes}`. The Rust harness deserializes the manifest and
drives `spawn_scripted_http_server` so vectors are data, not code.

## 1. Device login vectors

| ID | Script | Expected |
|---|---|---|
| DL-01 happy path | device 200 full body; token 202, 200 approved | `TokenApprovedState` populated; env file has all four keys |
| DL-02 defaults | device 200 without `expires_in`/`interval` | defaults 900/5 applied |
| DL-03 legacy pending | token 200 `{"status":"pending"}` then approved | pending classified, then approved |
| DL-04 nested error | token 400 `{"error":{"code":"authorization_pending"}}` | Pending |
| DL-05 flat OAuth error | token 400 `{"error":"slow_down"}` | SlowDown, wait += 5s |
| DL-06 expired | token 400 `expired_token` (also `expired`, `expired_device_code`) | Expired, clear rerun message |
| DL-07 gone | token 404 / 410 with empty body | Expired |
| DL-08 denied | token 403 `{"error":{"code":"access_denied","message":"..."}}` | Denied with server message surfaced |
| DL-09 empty api_key | token 200 `{"api_key":"  "}` | hard error, nothing persisted |
| DL-10 garbage 200 | token 200 non-JSON | parse error, nothing persisted |
| DL-11 unexpected 5xx | token 500 | error includes status + trimmed body |
| DL-12 device reject | device 400/422/429 | login aborts before any poll |

## 2. Browser approval/denial (backend-owned, vector-mirrored)

Client cannot test the web page; the backend must have executable tests for:

- BA-01 approve link marks device_code approved exactly once (idempotent).
- BA-02 deny link yields `access_denied` on next poll with the denial reason.
- BA-03 approving an expired code returns an error page, poll stays Expired.
- BA-04 the magic-link token is single-use: second click is a no-op/error.
- BA-05 approval from a different account/session than the email target fails.
- BA-06 `verify_url` host must match the auth service origin (client-side
  negative: reject/refuse to auto-open non-HTTPS or foreign-origin URLs; today
  `maybe_open_browser` opens whatever the server sends — add this check).

## 3. Account state (`/v1/me`)

| ID | Body | Expected |
|---|---|---|
| ME-01 full | active flagship w/ usage | parsed, tier cached |
| ME-02 minimal | missing `resets_at`, unknown tier `"mystery"` | tolerated; `parsed_tier()` None; gating falls back to Plus |
| ME-03 401 | `{"error":"invalid_key"}` | error surfaced; cached tier NOT overwritten (revocation is explicit, see 6) |
| ME-04 5xx/timeout | delay > 5s | `ME_FETCH_TIMEOUT` fires; offline gating uses cached tier |
| ME-05 status values | `active`, `past_due`, `canceled`, `trialing` | client renders status verbatim; no crash on unknown |

## 4. Checkout and portal

Checkout/portal are web-only today, but the client no longer hands off to a
pricing URL: `JCODE_PRICING_URL`
(`crates/jcode-base/src/subscription_catalog.rs:12`) has no callers, and the
login tail points at `jcode account manage` instead
(`src/cli/login/jcode_device.rs:217-221`). Conformance:

- CK-01 (backend) creating a checkout session for a signed-in device links the
  resulting subscription to the same `account_id` the device login returned.
- CK-02 (backend) completed checkout updates `/v1/me` tier within N seconds;
  vector asserts eventual consistency bound (suggest N=60 for staging test).
- CK-03 (client) after checkout, a fresh `/v1/me` fetch upgrades cached tier
  without re-login (test: ME-01 with new tier over old cached value).
- CK-04 (backend) portal cancel flows set `status:"canceled"` while keeping
  the key valid until period end; client vector ME-05 covers rendering.
- CK-05 (client) tier==none/empty after login prints the recovery actions
  (`print_recovery_actions`, `src/cli/login/jcode_device.rs:217-221`) —
  snapshot test on stderr text. **Respecified**: the original wording expected
  a pricing prompt that the flow does not emit.

## 5. Webhook ordering (backend-owned)

Stripe delivers webhooks out of order and at-least-once. Backend tests must
replay these orderings against the webhook handler and assert final state:

- WH-01 `checkout.session.completed` then `invoice.paid` (normal).
- WH-02 `invoice.paid` before `checkout.session.completed` (reorder).
- WH-03 duplicate delivery of each event (idempotency keys).
- WH-04 `customer.subscription.deleted` racing a same-second `invoice.paid`:
  terminal states win by event `created` timestamp, not arrival order.
- WH-05 signature invalid / stale timestamp -> 400, no state change.
- WH-06 unknown event type -> 2xx ack, no state change (forward compat).

Client-observable contract: after any WH sequence settles, `/v1/me` reflects
exactly one coherent `{tier, status}`; mirrored fixtures in `me/` enumerate
the reachable final states so the client test matrix stays closed.

## 6. Revocation

- RV-01 (backend) portal/admin revocation invalidates the API key: model API
  and `/v1/me` return 401 within a bounded lag (assert <= 60s in staging).
- RV-02 (client) 401 from the model API classifies as an auth failure with a
  recovery hint pointing at `/login jcode`
  (`crates/jcode-base/src/auth/login_diagnostics.rs`).
- RV-03 (client) revoked key must not silently fall back to another provider
  without surfacing the auth failure (account failover tests in
  `crates/jcode-base/src/provider/account_failover.rs`).
- RV-04 (backend) re-login after revocation issues a NEW key; old key stays
  dead (no resurrection).

## 7. Mixed-version compatibility matrix

Run the DL/ME vector suites in a 2x2 matrix:

| | old vectors (v1) | new vectors (v1.x) |
|---|---|---|
| released client (stable channel) | must pass | must pass ignoring unknown fields |
| head client | must pass | must pass |

Rules encoded as tests: unknown JSON fields ignored (serde default behavior —
add `deny_unknown_fields` NEVER); absent optional fields default (DL-02,
ME-02); new error codes fall into the "unexpected error" branch with the raw
body preserved (DL-11) rather than being misclassified as pending.

## 8. Security negative tests

- SN-01 device_code entropy: backend test asserts >= 128 bits, not guessable
  sequential IDs; token endpoint rate-limits per code and per IP (429 path is
  already client-handled: DL-05).
- SN-02 email enumeration: the client no longer sends an email at all
  (`subscription_api.rs:242`), so this is purely a backend property of
  whatever identifies the user on the approval page.
- SN-03 client never prints `api_key` or full `device_code` to stdout/stderr
  or logs (grep-based test over captured output of the login flow; see the
  observability doc's never-log list).
- SN-04 HTTP (non-TLS) `auth_base` refused outside tests unless
  `127.0.0.1`/`localhost` (client change + test; today any base is accepted).
- SN-05 oversized/hostile bodies: 10 MB body, wrong content-type, NUL bytes —
  client errors cleanly, no panic (fuzz-style vectors in `token_poll/`).
- SN-06 env-file permissions: persisted credentials file is owner-only on Unix
  (test on `persist_account_credentials`; already asserted in production by
  `ensure_account_credential_permissions`).
- SN-07 verify_url scheme/host allowlist before auto-opening browser (BA-06).
- SN-08 poll after approval: reusing a consumed device_code returns expired,
  never a second key (backend; client covered by DL-07 semantics).

## 9. Clocks and races

- CR-01 `interval: 0` -> clamped to 1s (unit test exists implicitly via
  `interval.max(1)`; make it explicit).
- CR-02 `expires_in: 0` -> deadline is `max(expires_in, interval)`; loop
  terminates with expiry error, no hot spin.
- CR-03 repeated `slow_down` grows wait monotonically; cap total at deadline.
- CR-04 approval lands between deadline check and poll: client accepts the
  approved response even if past deadline check happens next iteration only —
  vector: pending until t=deadline-1, then approved.
- CR-05 client timing uses `Instant` (monotonic), so wall-clock skew must not
  matter: test with mocked large `expires_in` and manual outcome injection.
- CR-06 two concurrent logins for the same email: last writer wins on the env
  file; no interleaved/corrupt file (serialize via file lock or accept and
  document last-write-wins with a test).
- CR-07 backend: approve and expire racing at the same second — exactly one
  outcome persisted.

## Execution plan

1. Add `tests/fixtures/account-contract/v1/` with the DL/ME vectors above and
   a manifest; port `spawn_scripted_http_server` into a shared test util.
2. Convert existing `jcode_device/tests.rs` cases to load from the manifest,
   keeping current assertions (no behavior change).
3. Add the client-side gaps found while writing this spec: SN-03, SN-04,
   SN-07, CR-01/02/06, ME-03 cache-preservation. (SN-06 is already enforced;
   see the grounding section.) Also add the missing `ActivationOutcome`
   family.
4. Mirror `manifest.json` into solosystems-backend and wire the backend suites
   (BA, WH, RV, CK, SN-01/02/08, CR-07) there.
5. Add the mixed-version check as a `scripts/check_guardrails.sh` gate (this
   fork has no `.github/` CI): run the stable-channel binary's login flow
   against head fixtures via the scripted server.
