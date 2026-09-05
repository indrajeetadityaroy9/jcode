# Account Flows: Observability, Privacy, and Support Diagnostics

Status: **mixed.** The client-side grounding below is accurate; everything
from "Audit events" onward is **unbuilt backend policy** for the private
`solosystems-backend` repo, not behavior of this fork. In particular:
**client telemetry was purged from this fork**, so no metric, trace, span, or
analytics event described here is emitted by jcode. Where this document names
a metric or trace, read it as a requirement on the backend and as
*not-implemented-here* on the client.

Security logging, audit events, metrics, traces, alerting, redaction,
retention, and support diagnostics for the six subscription account flows:
(1) device login, (2) browser approval/denial, (3) account state (`/v1/me`),
(4) checkout, (5) billing portal, (6) webhooks/revocation.

Companion to `docs/dev/ACCOUNT_CONTRACT_CONFORMANCE_TESTS.md`. Client-side
surfaces live in this repo; backend surfaces live in `solosystems-backend`.

## Existing surfaces (grounding)

- Client telemetry: **removed in this fork.** There is no client analytics
  pipeline, no anonymous install id, and no outbound event endpoint. Anything
  below describing client-side event collection is backend guidance only.
- Redaction is structural, not a logging convention: `AccountApiError`
  (`crates/jcode-base/src/subscription_api.rs:106-115`) can only carry a
  status code and an 80-char error code, never a response body or bearer
  value.
- Failure classification labels: `crates/jcode-base/src/auth/login_diagnostics.rs`
  (`AuthFailureReason::label`, `:19`) — the only failure detail that may be
  reported.
- Credential-state labels: `CredState::label`
  (`crates/jcode-base/src/auth/refresh_state.rs:37-45`) and refresh-token
  fingerprints (`refresh_token_fingerprint`, `:190-196`) — closed vocabulary,
  never the token.
- Local logs: `~/.jcode/logs/jcode-YYYY-MM-DD.log`.
- Backend analytics store: lives in `solosystems-backend`, not this repo.

## Never-log list (both repos)

These values must never appear in logs, traces, crash reports, or
support bundles, at any log level:

- `api_key` (JCODE_API_KEY) — full or truncated beyond `jc_...last4`.
- `device_code`, magic-link tokens, approval URL query params.
- Raw email addresses (local logs may show what the user already sees on
  screen; the backend audit log stores email only in the audit table, not
  app logs).
- Stripe secrets: webhook signing secret, customer payment details, full
  checkout/portal session URLs (they embed session secrets).
- Env-file contents (`jcode-subscription` env file) and `Authorization`
  headers in any HTTP client debug logging.

Enforcement: none yet. SN-03 in the companion doc proposes an output-capture
test over the login flow; it does not exist. What does hold today is
structural: `AccountApiError` cannot carry a body or bearer value, and the
device flow never accepts a secret on stdin.

## Audit events (backend, durable) — **not built here**

This is a requirement on `solosystems-backend`. Nothing in this fork writes an
audit row; there is no audit table, no `flow_id` propagation, and no event
emission of any kind (telemetry was purged).

Append-only audit table keyed by `account_id`, retained 400 days:

| Event | Required fields |
|---|---|
| `device_auth.requested` | account_id?, email_hash, ip_hash, user_agent class, device_code_id (opaque id, not the code) |
| `device_auth.approved` / `denied` | device_code_id, approver session id, reason |
| `device_auth.expired` | device_code_id |
| `key.issued` | key_id (not the key), tier |
| `key.revoked` | key_id, actor (user/portal/admin/system), reason |
| `checkout.started` / `completed` / `abandoned` | stripe session id, tier target |
| `portal.opened`, `subscription.updated` / `canceled` | stripe ids, old->new tier/status |
| `webhook.received` / `applied` / `rejected` | stripe event id, type, signature result, dedup outcome |
| `me.tier_changed` | old, new, cause (webhook/admin) |

Correlation requirements: every audit row carries `account_id`,
`request_id` (per HTTP request), and `flow_id` (one device-login attempt or
one checkout attempt). `device_code_id` links flows 1-2; `stripe_event_id`
links 4-6. The client sends no correlation IDs. The one identifier it *does*
receive is `DeviceAuthorization::flow_id`
(`crates/jcode-base/src/subscription_api.rs:72`), which it parses and requires
but never transmits back or logs; wiring it into client-side correlation would
be new work.

## Metrics and alerting (backend) — **not built here**

None of these metrics are emitted by jcode: this fork has no telemetry or
metrics pipeline at all. They are specifications for the backend. The only
client-side value referenced below that is real is `ME_FETCH_TIMEOUT`.

Metrics (per flow, labeled by outcome and tier where applicable):

- `device_auth_requests_total`, `device_auth_approvals_total`,
  `device_auth_denials_total`, `device_auth_expiries_total`;
  alert: approval rate < 50% over 1h, or expiry rate > 40% (email delivery
  problem), or request spike > 10x baseline (enumeration/abuse).
- `token_poll_requests_total{result}`; alert on `slow_down` ratio > 20%
  (client misbehavior or attack) and on unexpected-5xx ratio > 1%.
- `me_requests_total{status}`; alert on 401 spike (mass revocation bug) and
  p99 latency > 2s (client timeout is 5s: `ME_FETCH_TIMEOUT`,
  `crates/jcode-base/src/subscription_api.rs:14`).
- `webhook_events_total{type, outcome=applied|duplicate|rejected}` and
  `webhook_apply_lag_seconds` (Stripe `created` -> state applied);
  alert: rejected signatures > 0 sustained, lag p95 > 60s (this is the
  RV-01/CK-02 conformance bound), any DLQ depth > 0 for 15m.
- `key_revocations_total{actor}`; alert on system/admin bulk revocations.

## Traces — **not built here**

Backend: one trace per request; parent span per flow_id so a device-login
attempt shows request -> email send -> approval -> token issue as linked
spans. Span attributes limited to the audit-field set (ids and hashes, never
secrets/emails). Stripe webhook handling gets a span per event with
`stripe_event_id` and dedup decision.

Client: no distributed tracing for auth (privacy). Local log lines around the
login flow use the daily log with the flow outcome only.

## Retention

| Data | Where | Retention |
|---|---|---|
| Audit events | backend | 400 days, append-only |
| Backend app logs | backend | 30 days |
| Traces | backend | 7-14 days |
| Stripe webhook payload archive | backend (encrypted) | 90 days |
| Local client logs | `~/.jcode/logs/` | user-owned; must satisfy never-log list |
| Support bundles | created on demand | delete after case close (<= 90 days) |

## Support diagnostics — partially built

What exists: `jcode account status` (`src/cli/account.rs:9-49`) fetches
`/v1/me` and prints email, plan + status, metered usage against budget, reset
time, and a management URL passed through an HTTPS-origin allowlist
(`public_manage_url`, `:102`). A revoked key (401) clears local credentials
and says so (`:40-46`). `--json` dumps the parsed `SubscriptionMe` verbatim
(`:18-21`). It never prints the key.

What is proposed and **not built**: a `jcode doctor`-style account section
(extend the provider doctor, `crates/jcode-provider-doctor/`) printing
auth_base, masked key (`jc_...last4`), cached tier
(`subscription_catalog::cached_tier`,
`crates/jcode-base/src/subscription_catalog.rs:362`), last `/v1/me` status,
and env-file path plus permissions. There is no masked-key output anywhere
today, so "ask the user for the masked key id" has no client-side source yet.
Support asks the user for: masked key id, approximate login time, and the
`AuthFailureReason` label; backend support joins on `key_id`/`email_hash` in
the audit table. No flow requires the user to paste a key or link.

## Per-flow one-page summary (target state, not current)

1. Device login: client logs outcome label only; backend audits request/
   approve/expire with device_code_id; metric+alert on approval/expiry rates.
2. Browser approval/denial: backend-only; audit approver session, alert on
   denial spikes; page must not log the magic token (only its hash).
3. `/v1/me`: backend request logs with account_id + request_id; client caches
   tier and logs nothing sensitive; alert on 401 spikes and latency.
4. Checkout: audit started/completed/abandoned via Stripe ids; funnel metric;
   never log payment details (Stripe owns them).
5. Portal: audit opened + resulting subscription changes; portal URLs are
   secrets (never logged).
6. Webhooks/revocation: audit every event with signature + dedup outcome;
   lag and DLQ alerts; revocation audit rows are the support source of truth.

## Follow-ups (actionable in this repo)

- Add an SN-03-style output-capture test asserting the never-log list for the
  login flow. Nothing enforces it today.
- Add the account section to the provider doctor, including the first
  masked-key (`jc_...last4`) renderer in the client.
- Decide whether `DeviceAuthorization::flow_id` should be surfaced in local
  log lines so a user-reported login can be joined to a backend flow without
  any new telemetry. It is currently parsed and discarded.
