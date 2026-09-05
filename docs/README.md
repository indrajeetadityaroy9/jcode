# jcode Docs

Reference documentation for the jcode codebase.

## Layout

- `docs/*.md` — architecture, feature, and behavior docs. **Current state of the system**: every
  claim here should resolve to code that exists today.
- `docs/plans/` — forward-looking plans and partially-landed work. Each plan states which phases
  shipped and which did not.
- `docs/proposals/` — design proposals not committed to. Nothing here is built unless the doc
  says so explicitly.
- `docs/audits/` — point-in-time audits. Historical snapshots, not kept up to date; each carries
  a staleness header naming what has since moved.
- `docs/dev/` — developer-facing process and testing notes.

## Key entry points

- Architecture: `SERVER_ARCHITECTURE.md`, `MODULAR_ARCHITECTURE_RFC.md`, `CRATE_OWNERSHIP_BOUNDARIES.md`
- Swarm: `SWARM_ARCHITECTURE.md`, `SWARM_TASK_GRAPH.md`
- Memory: `MEMORY_ARCHITECTURE.md`, `MEMORY_BUDGET.md`, `MEMORY_INCIDENT_RUNBOOK.md`
- Refactoring and quality: `REFACTORING.md`, `plans/CODE_QUALITY_10_10_PLAN.md`, and the live
  ratchet baselines in `scripts/*_budget.json`
- Harness API / SDK: `HARNESS_API.md`
- Providers: `PROVIDER_DOCTOR.md`, `AUTH_CREDENTIAL_SOURCES.md`
- Platform: `TERMINAL_CAPABILITIES.md`
- What this fork removed and why: `FORK_WORKFLOW.md` §1

## Conventions

- Docs describing current behavior live at the top level; anything speculative goes in `plans/`
  or `proposals/`. A top-level doc that turns out to describe unbuilt work belongs in one of
  those two directories, not at the root.
- Prefer updating an existing doc over adding a near-duplicate.
- Root of the repo should only hold README, CONTRIBUTING, RELEASING, AGENTS, LICENSE, and similar
  meta files. Put everything else here.
- When a subsystem is purged, update or delete the docs that describe it in the same change, and
  record the removal in `FORK_WORKFLOW.md` §1.
