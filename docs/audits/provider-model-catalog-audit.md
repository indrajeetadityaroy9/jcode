> **Staleness note (added 2026-09-05).** This is a **point-in-time snapshot**
> of provider documentation as it read when the audit was taken; the findings
> below are preserved as written and have not been re-verified. Two things have
> moved since:
>
> - **Scope path.** The `OpenAiCompatibleProfile` *struct* is still in
>   `crates/jcode-provider-metadata/src/lib.rs:116-125`, but the profile
>   constants and the array live in
>   `crates/jcode-provider-metadata/src/catalog.rs` —
>   `OPENAI_COMPAT_PROFILES: [OpenAiCompatibleProfile; 37]` at `catalog.rs:435`.
>   The sibling login catalog is
>   `LOGIN_PROVIDERS: [LoginProviderDescriptor; 46]` at `catalog.rs:1089`.
> - **Coverage.** The table below has 29 rows: 28 named vendor profiles plus
>   the generic user-supplied endpoint (`OPENAI_COMPAT_PROFILE`, which is
>   itself one of the array entries). That is 29 of the 37 profiles now in
>   `OPENAI_COMPAT_PROFILES`. The 8 never audited are OpenRouter, Anthropic
>   API, OpenAI API, and Gemini API (the OpenAI-compatible shims for
>   first-party providers), plus NVIDIA NIM, Xiaomi MiMo, Meta Model API, and
>   Celeris.
>
> No row here covers a purged provider: xAI/Grok, AWS Bedrock, Gmail/Google
> login, and the ChatGPT-web route were never OpenAI-compatible profiles and
> are gone from this fork, so nothing in this audit needs retracting on that
> account. Display names have drifted slightly (`Nebius` is now "Nebius Token
> Factory", `Comtegra` is "Comtegra GPU Cloud", `DeepInfra` is "Deep Infra");
> the rows still refer to the same profiles.

# OpenAI-compatible provider `/models` audit

Scope: built-in `OpenAiCompatibleProfile` entries in `crates/jcode-provider-metadata/src/lib.rs`.

Legend:
- `verified data[]`: docs show or explicitly reference OpenAI-compatible `GET /models` shape `{ object: "list", data: [...] }`, or the provider says it implements the OpenAI Models API.
- `verified top-level array`: docs show a top-level model array.
- `supported endpoint, shape not shown`: docs say `/models` exists or OpenAI compatibility includes it, but do not show the response body.
- `catalog/static only`: docs point to a static catalog/models page, not a live `/models` endpoint.
- `unknown`: could not find provider docs proving `/models`.

## Audited providers

| Provider | Evidence | Expected parser support | Notes |
|---|---|---:|---|
| OpenCode Zen | OpenCode docs and models.dev catalog | static/bootstrap, endpoint unverified | OpenCode itself uses models.dev. `/models` on Zen not independently verified. |
| OpenCode Go | OpenCode docs and models.dev catalog | static/bootstrap, endpoint unverified | Same as Zen. |
| Z.AI | Search did not find `/models`; provider docs URL 404 from current metadata path | unknown | Need direct current docs URL or live key-backed test. |
| Kimi Code | Kimi third-party-agent docs found, OpenAI Compatible config only | unknown | No `/models` response shape found. |
| 302.AI | Official docs contain `Models（列出模型）GET` page | likely OpenAI-compatible data[] | Dedicated list-models page exists. Response body in fetched text was truncated before example. |
| Baseten | Official docs say public OpenAI-compatible endpoint `https://inference.baseten.co/v1` | supported endpoint, shape not shown | No dedicated `/models` response found. |
| Cortecs | Official docs overview only plus OpenCode provider entry | catalog/static only | No `/models` endpoint docs found. |
| DeepSeek | Official `GET /models` docs show `{ object, data[] }` | verified data[] | Covered by parser. |
| Comtegra | Official docs list supported `/v1/models` linking OpenAI Models API | supported endpoint, shape OpenAI | Covered by parser. |
| FPT AI Marketplace | Official docs show chat/completions through LiteLLM/OpenAI, no models endpoint | unknown/no evidence | Live `/models` may fail. |
| Firmware/FrogBot | OpenCode provider docs only | catalog/static only | No direct provider API docs found. |
| Hugging Face | General Inference Providers docs, OpenAI-compatible API | supported endpoint, shape not shown | No dedicated `/models` page verified. |
| Moonshot AI | Search/current URL did not expose `/models` docs | unknown | Kimi API search hints model list endpoint, but no official Moonshot page fetched. |
| Nebius | Quickstart docs OpenAI-compatible endpoint | supported endpoint, shape not shown | Dedicated `/models` page not verified. |
| Scaleway | Official “Using Models API” docs found | supported endpoint, shape likely OpenAI | Covered by parser if OpenAI shape. |
| STACKIT | Official integration docs say OpenAI-compatible API and model picker fetches `/models` | supported endpoint, shape not shown | Covered if OpenAI shape. |
| Groq | API reference has Models/List models | verified data[] | Covered. |
| Mistral | API reference has Models/List Available Models | verified data[] style | Covered. |
| Perplexity | API docs fetch/search did not find list-models endpoint | unknown | May not support `/models`; static docs list models. |
| Together AI | Official `GET /models` docs show top-level array | verified top-level array | Parser was fixed for this. |
| DeepInfra | Official OpenAI-compatible docs point to static model catalog, no `/models` page found | catalog/static only | Live `/models` unverified. |
| Fireworks | Official list-models docs found for account model API `{ models: [...] }`; OpenAI compat endpoint also exists | verified models[] variant for account API | Parser supports `models[]` and `name`. Need live base endpoint shape still unverified. |
| MiniMax | Official text generation docs show OpenAI-compatible base and static supported-models table | catalog/static only | No `/models` endpoint found. |
| LM Studio | Official OpenAI compatibility docs list `GET /v1/models` | supported endpoint, shape not shown | OpenAI local server expected data[]. |
| Ollama | Official OpenAI compatibility blog/docs cover chat; `/v1/models` docs not found in fetched page | unknown | Need raw docs/source or live local test. |
| Chutes | Live user response showed `{ object:"list", data:[...] }` with numeric pricing | verified data[] plus numeric pricing | Parser fixed and stale default removed. |
| Cerebras | Official `GET /v1/models` docs show `{ object, data[] }` | verified data[] | Covered. |
| Alibaba Coding Plan | Official docs show OpenAI-compatible base URL but warn Coding Plan is for coding tools only; no `/models` docs | unknown/no evidence | Static default likely needed; live `/models` may fail. |
| Generic openai-compatible | User-supplied endpoint | parser contract | We support `{data[]}`, top-level array, `{models[]}`, id/name identifiers. |

## Parser coverage after `f291f0e`

Supported response forms:
- `{ "data": [{ "id": "..." }] }`
- top-level `[{ "id": "..." }]`
- `{ "models": [{ "id" or "name": "..." }] }`
- numeric or string pricing fields
- context fields: `context_length`, `contextLength`, `max_context_length`, `maxModelLength`, `max_model_len`, `trainingContextLength`

## Gaps identified

No additional parser shape is proven necessary yet. The remaining issue is provider capability/profile accuracy:
- Some providers are OpenAI-compatible for chat but do not document live `GET /models`.
- For those, live catalog refresh should remain best-effort and must gracefully fall back to static catalog.
- Long-term, `OpenAiCompatibleProfile` should probably carry a `model_catalog` capability/strategy so providers known not to support `/models` do not emit noisy refresh failures.
