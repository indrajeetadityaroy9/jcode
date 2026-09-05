# OpenAI-Compatible Profile Runtime Migration Plan

Status: **partially landed under different names.** Slices 1, 3, and 4 are in
the tree; slice 2's named structs were never created; slices 5 and 6 are open.
See "Incremental migration slices" for the per-slice status and the plan-name
to real-name mapping.

Name mapping (plan -> what actually exists):

| Plan name | Real name | Location |
|---|---|---|
| `OpenAiCompatibleClient` | *never created*; transport fields live directly on `OpenRouterProvider` (`client`, `api_base`, `auth`) | `crates/jcode-provider-openrouter-runtime/src/lib.rs:871-905` |
| `OpenAiCompatibleProfileRuntime` | *never created*; `OpenRouterProvider` gained `profile_id: Option<String>` and serves both roles | `crates/jcode-provider-openrouter-runtime/src/lib.rs:871`, `:879` |
| profile descriptor (implied) | `OpenAiCompatibleProfile` (static) / `ResolvedOpenAiCompatibleProfile` (owned) | `crates/jcode-provider-metadata/src/lib.rs:116-138` |
| `openai_compatible: RwLock<BTreeMap<..>>` | `openai_compatible_profiles: RwLock<HashMap<String, Arc<dyn Provider>>>` | `crates/jcode-base/src/provider/mod.rs:362` |
| `active_openai_compatible_profile: RwLock<Option<String>>` | same name, as specified | `crates/jcode-base/src/provider/mod.rs:363` |

## Problem

`OpenRouterProvider` currently represents two distinct concepts:

1. Standard OpenRouter, with OpenRouter-specific routing, provider pinning, endpoint metadata, and an `openrouter` catalog namespace.
2. Direct OpenAI-compatible providers such as NVIDIA NIM, Groq, Cerebras, Chutes, and custom endpoints, which reuse the same HTTP transport but have distinct credentials, API bases, catalogs, and model IDs.

Because `MultiProvider` stores only one `openrouter` runtime slot, switching from standard OpenRouter to a direct profile replaces the active runtime/catalog view. This caused issue #274: after switching from `openrouter/owl-alpha` to NVIDIA NIM, `/model` no longer exposed standard OpenRouter and could mis-associate OpenRouter models with NVIDIA.

## Target architecture

Separate transport, profile identity, and route aggregation.

```rust
struct OpenAiCompatibleClient {
    api_base: String,
    api_key_env: String,
    env_file: String,
    auth_header: AuthHeaderConfig,
}

struct OpenAiCompatibleProfileRuntime {
    profile_id: String,          // "openrouter", "nvidia-nim", "groq", ...
    display_name: String,        // "OpenRouter", "NVIDIA NIM", ...
    cache_namespace: String,     // usually profile_id
    default_model: Option<String>,
    provider_routing: bool,      // true for standard OpenRouter features
    client: OpenAiCompatibleClient,
}
```

Neither struct was created; see slice 2. The descriptive half of this shape
does exist, as static/owned profile descriptors:
`OpenAiCompatibleProfile { id, display_name, api_base, api_key_env, env_file,
setup_url, default_model, requires_api_key }` and its owned twin
`ResolvedOpenAiCompatibleProfile` (`crates/jcode-provider-metadata/src/lib.rs:116-138`),
with the 37 built-in profiles in
`crates/jcode-provider-metadata/src/catalog.rs:435`. What has no home is the
*runtime* type: the fields above live scattered on `OpenRouterProvider`.

`MultiProvider` should eventually move from:

```rust
openrouter: RwLock<Option<Arc<openrouter::OpenRouterProvider>>>,
```

to something like:

```rust
openai_compatible: RwLock<BTreeMap<String, Arc<OpenAiCompatibleProfileRuntime>>>,
active_openai_compatible_profile: RwLock<Option<String>>,
```

Standard OpenRouter becomes one profile in this map, not the container for every compatible provider.

The `MultiProvider` move **landed** (slices 3-4), with `HashMap` instead of
`BTreeMap` and `Arc<dyn Provider>` instead of a concrete runtime type:
`crates/jcode-base/src/provider/mod.rs:362-363`. The `openrouter` slot also
survives alongside it (`mod.rs:355`) rather than becoming just another map
entry, so standard OpenRouter is still special-cased.

## Route aggregation rule

`/model` should aggregate routes from every configured profile:

```rust
for profile in configured_openai_compatible_profiles() {
    routes.extend(profile.model_routes());
}
```

Switching active runtime to NVIDIA NIM should only update active selection:

```rust
active_openai_compatible_profile = Some("nvidia-nim".into());
```

It should not remove or relabel `openai_compatible["openrouter"]`.

## Compatibility requirements

Keep existing user-facing forms working:

- `openrouter:<model>` targets standard OpenRouter.
- `nvidia-nim:<model>` targets NVIDIA NIM.
- `openai-compatible:<model>` targets the configured custom endpoint.
- `--provider openrouter` remains standard OpenRouter.
- `--provider openai-compatible` remains the generic/custom profile.
- Existing `OpenRouterProvider` type can remain as a compatibility wrapper while internals move.

## Incremental migration slices

1. **Route aggregation slice, completed in `b1272ae`** — **landed.**
   - Standard OpenRouter cached routes are scoped to the `openrouter` namespace.
   - Direct profiles can be active without hiding standard OpenRouter from `/model`.
   - Regression: OpenRouter `owl-alpha` -> NVIDIA NIM -> `/model` keeps OpenRouter route and does not relabel it as NVIDIA.
   - As built: namespaced disk cache reads
     (`load_disk_cache_entry_for_namespace`,
     `crates/jcode-provider-openrouter-runtime/src/lib.rs:720`) and
     per-profile catalog refresh state keyed by profile id (`:625-676`).

2. **Profile runtime struct** — **not built.**
   - Introduce `OpenAiCompatibleProfileRuntime` around current OpenRouter provider settings.
   - Keep `OpenRouterProvider` as a type alias/wrapper initially.
   - Neither `OpenAiCompatibleProfileRuntime` nor `OpenAiCompatibleClient`
     exists. Instead `OpenRouterProvider`
     (`crates/jcode-provider-openrouter-runtime/src/lib.rs:871-905`) grew a
     `profile_id: Option<String>` (`:879`) plus per-profile capability flags
     (`supports_provider_features`, `supports_model_catalog`,
     `send_openrouter_headers`, `reasoning_effort_support`, `extra_body`,
     `static_models`) and serves as both the OpenRouter runtime and every
     direct profile runtime. `MultiProvider` holds them as `Arc<dyn Provider>`
     rather than as a concrete profile-runtime type
     (`crates/jcode-base/src/provider/mod.rs:362`).

3. **Runtime registry** — **landed.**
   - Add a map of configured compatible profiles to `MultiProvider`.
   - Populate it from configured/saved credentials at startup and auth-change time.
   - As built: `openai_compatible_profiles: RwLock<HashMap<String, Arc<dyn
     Provider>>>` (`crates/jcode-base/src/provider/mod.rs:362`, initialized at
     `:2774`), reached through `ProviderRegistry::compatible_profile` /
     `active_compatible_profile_id` / `set_active_compatible_profile` /
     `clear_active_compatible_profile`
     (`crates/jcode-base/src/provider/registry.rs:26-69`). Note it is a
     `HashMap`, not the `BTreeMap` sketched above, so iteration order is not
     stable.

4. **Active profile selection** — **landed, with the env layer still live.**
   - Replace implicit environment mutation as the only active-profile state with explicit profile IDs.
   - Use env application only as a compatibility/bootstrap layer.
   - As built: `active_openai_compatible_profile: RwLock<Option<String>>`
     (`crates/jcode-base/src/provider/mod.rs:363`) is the authoritative
     selection and is cleared explicitly on switches away
     (`mod.rs:978`, `:1052`, `:1254`). The env layer is still written on
     profile application (`JCODE_OPENROUTER_API_BASE`,
     `..._API_KEY_NAME`, `..._ENV_FILE`, `..._CACHE_NAMESPACE`,
     `..._PROVIDER_FEATURES`, ...:
     `crates/jcode-base/src/provider_catalog.rs:615-630`), which is the
     intended compatibility/bootstrap role but has not been narrowed.

5. **Picker and server snapshots** — **partially landed.**
   - Emit profile-scoped routes and available-model snapshots.
   - Include profile ID/api method in debug output so mislabeling is testable.
   - As built: `RuntimeKey::OpenAiCompatible { profile_id }`
     (`crates/jcode-provider-core/src/lib.rs:701-704`) carries the profile
     identity, and route selection recovers it by parsing the
     `openai-compatible:<profile_id>` prefix out of `ModelRoute::api_method`
     (`crates/jcode-base/src/provider/mod.rs:888-895`). `ModelRoute` itself
     (`crates/jcode-provider-core/src/lib.rs:675-683`) has no `profile_id`
     field, so the profile travels as a string inside `api_method` — the
     stringly-typed shape this slice was meant to remove.

6. **Rename cleanup** — **not started.**
   - Rename generic internals from OpenRouter to OpenAI-compatible where accurate.
   - Keep public commands and config stable.
   - The generic runtime, its crate, and its env vars are all still named
     `openrouter` (`crates/jcode-provider-openrouter-runtime`,
     `JCODE_OPENROUTER_*`).

## Validation matrix

For each configured profile pair, verify:

- Active profile A, inactive profile B: `/model` shows both A and B routes.
- Selecting a B route switches to B and keeps A visible.
- Models with slash IDs are not automatically treated as standard OpenRouter unless the route/profile says so.
- OpenRouter provider-pinning remains available only for the standard OpenRouter profile.
- Direct-profile static and live catalogs remain namespace-scoped.

Key regression scenarios:

- `openrouter/owl-alpha` -> `nvidia-nim:nvidia/llama-...` -> OpenRouter still selectable.
- Cerebras active with Groq configured -> no relabeling of Cerebras models as Groq.
- Chutes active with stale legacy OpenRouter cache -> no stale OpenRouter models under Chutes.
