// Spawn route inheritance: which provider_key/route a spawned worker carries.
// Included from `comm_session_tests.rs`, which owns the `use` list.

#[test]
fn resolve_swarm_spawn_model_does_not_inherit_a_route_that_cannot_serve_the_model() {
    // Observed live: a coordinator running claude-opus-5 over Claude OAuth held
    // a stale `provider_key=gemini-api` (stamped before provider selection
    // settled). Inheriting that pair sent the worker's first request to
    // generativelanguage.googleapis.com and it died immediately, so the
    // incoherent key must be replaced by one derived from the model.
    let selection = resolve_swarm_spawn_selection(
        None,
        None,
        &coordinator_identity(
            Some("claude-opus-5"),
            Some("gemini-api"),
            Some("openai-compatible:gemini-api"),
        ),
    );

    assert_eq!(selection.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(selection.provider_key.as_deref(), Some("claude"));
    // The route belonged to the rejected provider, so it must not survive.
    assert_eq!(selection.route_api_method, None);
}

#[test]
fn resolve_swarm_spawn_model_keeps_a_custom_profile_key_for_an_unknown_model_id() {
    // The guard must only fire on a confident first-party disagreement: an
    // OpenAI-compatible profile legitimately serves model ids the built-in
    // tables know nothing about.
    let selection = resolve_swarm_spawn_selection(
        None,
        None,
        &coordinator_identity(
            Some("nvidia/llama-3.3-nemotron-super-49b-v1"),
            Some("nvidia"),
            Some("openai-compatible:nvidia-nim"),
        ),
    );

    assert_eq!(selection.provider_key.as_deref(), Some("nvidia"));
    assert_eq!(
        selection.route_api_method.as_deref(),
        Some("openai-compatible:nvidia-nim")
    );
}
