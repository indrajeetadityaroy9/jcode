<!--
This file IS the swarm config. Swarms are complicated, dynamic systems, so
routing policy is passed to the models as a prompt rather than as options in
a standard config file. Edit freely: override globally at
~/.jcode/swarm-prompt.md or per-project at ./.jcode/swarm-prompt.md.
-->

Model routing guidance for spawned swarm agents. Run `swarm list_models` when
you need to confirm which models/routes are actually available.

- Default: omit `model` so every worker inherits the coordinator's model. A
  swarm should run on one model unless there is a reason for it not to, so that
  worker output is consistent with the coordinator's and the whole swarm bills
  to one provider.
- Pass `model` only when the user asked for a specific model for that worker,
  or when the task genuinely needs a different one. Say which in the task
  prompt, so the choice is reviewable.
- Pass a bare model id. A route prefix (e.g. `claude-oauth:`) pins one
  credential and fails on a machine authenticated the other way; without it the
  available route for that model is used.
- `effort` is independent of `model`: tune it freely (e.g. `effort: "low"` for
  mechanical implementation, `"none"` for bulk reading) without switching model.

Structure guidance for spawned swarm agents:

- Always pass `label` when spawning (e.g. `label: "api reviewer"`) so the swarm
  UI shows what each agent is for. The explicit `spawn` action rejects missing or
  blank labels.
- In normal and light-swarm mode, only the root session may spawn agents. Workers
  must complete their assigned task directly and report back rather than creating
  another generation.
- Recursive spawning is reserved for a root running in `swarm-deep` mode. In that
  mode the spawner owns its children, and manager-style decomposition may create
  deeper subtrees when it materially improves coverage.
