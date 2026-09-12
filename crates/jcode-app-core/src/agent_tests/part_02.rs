// Stop-reason classification and provider guardrail / empty-response recovery.
#[test]
fn empty_post_tool_response_gets_more_than_one_retry() {
    // Regression guard for the Claude Opus 5 benchmark incident. A provider can
    // return an empty response immediately after tool results; that is a
    // transient hiccup, not a finished task. With only one retry allowed, a
    // single empty response (observed once in 43 turns) ended a 20-hour agent
    // run with the work half-done and the submission unoptimized.
    assert!(
        Agent::MAX_EMPTY_POST_TOOL_CONTINUATION_ATTEMPTS > 1,
        "a single retry lets one transient empty response end a long run"
    );
    // Bounded, so a genuinely finished agent still exits instead of looping.
    assert!(Agent::MAX_EMPTY_POST_TOOL_CONTINUATION_ATTEMPTS <= 10);
}

#[test]
fn output_budget_truncation_requests_a_continuation() {
    // Regression guard for the Claude Opus 5 benchmark incident. A turn cut off
    // by the output budget reports stop_reason=max_tokens and can contain zero
    // tool calls, which otherwise looks exactly like a finished turn. The agent
    // must treat it as incomplete and continue rather than ending the run.
    assert!(Agent::should_continue_after_stop_reason("max_tokens"));
    assert!(Agent::should_continue_after_stop_reason("MAX_TOKENS"));
    assert!(Agent::should_continue_after_stop_reason(" max_tokens "));
    assert!(Agent::should_continue_after_stop_reason(
        "max_output_tokens"
    ));
    assert!(Agent::should_continue_after_stop_reason("length"));
    assert!(Agent::should_continue_after_stop_reason("truncated"));
    assert!(Agent::should_continue_after_stop_reason("incomplete"));

    // Normal completions must not trigger a continuation loop.
    assert!(!Agent::should_continue_after_stop_reason("end_turn"));
    assert!(!Agent::should_continue_after_stop_reason("tool_use"));
    assert!(!Agent::should_continue_after_stop_reason("stop"));
    // An absent reason is the pre-fix wire behaviour: it cannot be recovered
    // from, which is precisely why MessageEnd must forward the real reason.
    assert!(!Agent::should_continue_after_stop_reason(""));
}

#[test]
fn stranded_tool_use_stop_is_detected() {
    // Second half of the Opus 5 DeepSWE incident: the provider reported
    // stop_reason="tool_use" while the parsed tool-call list was empty, so the
    // turn loop had nothing to execute and broke out mid-task, discarding every
    // uncommitted edit. `tool_use` is a normal completion reason, so
    // `should_continue_after_stop_reason` must keep rejecting it; the stranded
    // case is only recoverable when it is paired with zero tool calls, which is
    // exactly what this predicate is for.
    assert!(Agent::is_stranded_tool_use_stop(Some("tool_use")));
    assert!(Agent::is_stranded_tool_use_stop(Some("TOOL_USE")));
    assert!(Agent::is_stranded_tool_use_stop(Some(" tool_use ")));

    assert!(!Agent::is_stranded_tool_use_stop(Some("end_turn")));
    assert!(!Agent::is_stranded_tool_use_stop(Some("max_tokens")));
    assert!(!Agent::is_stranded_tool_use_stop(Some("")));
    assert!(!Agent::is_stranded_tool_use_stop(None));
    // Must stay disjoint from the truncation path so a turn never takes both
    // continuation branches for one stop reason.
    assert!(!Agent::should_continue_after_stop_reason("tool_use"));
}

#[test]
fn guardrail_stop_reason_detection() {
    assert!(Agent::is_guardrail_stop_reason(Some("refusal")));
    assert!(Agent::is_guardrail_stop_reason(Some("REFUSAL")));
    assert!(Agent::is_guardrail_stop_reason(Some(" content_filter ")));
    assert!(Agent::is_guardrail_stop_reason(Some("safety")));
    assert!(Agent::is_guardrail_stop_reason(Some("model_guardrail")));
    assert!(Agent::is_guardrail_stop_reason(Some("policy_violation_x")));
    assert!(!Agent::is_guardrail_stop_reason(Some("end_turn")));
    assert!(!Agent::is_guardrail_stop_reason(Some("max_tokens")));
    assert!(!Agent::is_guardrail_stop_reason(Some("tool_use")));
    assert!(!Agent::is_guardrail_stop_reason(Some("stop")));
    assert!(!Agent::is_guardrail_stop_reason(None));
}

#[test]
fn fable_guardrail_reconsideration_is_narrow_and_bounded() {
    assert!(Agent::should_reconsider_fable_guardrail(
        "claude-fable-5",
        Some("refusal"),
        0,
        1,
    ));
    assert!(Agent::should_reconsider_fable_guardrail(
        "CLAUDE-FABLE-5-20260801",
        Some("content_filter"),
        0,
        1,
    ));
    assert!(Agent::should_reconsider_fable_guardrail(
        "claude-fable-5",
        Some("refusal"),
        1,
        3,
    ));
    assert!(Agent::should_reconsider_fable_guardrail(
        "claude-fable-5",
        Some("refusal"),
        2,
        3,
    ));
    assert!(!Agent::should_reconsider_fable_guardrail(
        "claude-fable-5",
        Some("refusal"),
        3,
        3,
    ));
    assert!(!Agent::should_reconsider_fable_guardrail(
        "claude-fable-5",
        Some("end_turn"),
        0,
        1,
    ));
    assert!(!Agent::should_reconsider_fable_guardrail(
        "claude-opus-5",
        Some("refusal"),
        0,
        1,
    ));
}

#[test]
fn fable_guardrail_prompt_suite_is_distinct_and_safety_preserving() {
    let prompts = Agent::FABLE_GUARDRAIL_RECONSIDERATION_PROMPTS;
    assert_eq!(prompts.len(), 3);
    assert_ne!(prompts[0], prompts[1]);
    assert_ne!(prompts[1], prompts[2]);
    assert!(prompts[0].contains("full context"));
    assert!(prompts[1].contains("safe portions"));
    assert!(prompts[2].contains("Do not weaken a refusal"));
}

#[test]
fn guardrail_notice_for_refusal_stop() {
    let notice = Agent::provider_guardrail_notice(Some("refusal"), true, true)
        .expect("refusal with empty text must produce a notice");
    assert!(
        notice.contains("refusal"),
        "notice should name the stop reason: {notice}"
    );
    assert!(notice.to_lowercase().contains("guardrail"));
    // Guardrail stop with visible text still surfaces (partial output then refusal).
    assert!(Agent::provider_guardrail_notice(Some("refusal"), false, false).is_some());
}

#[test]
fn guardrail_notice_for_silent_empty_turn() {
    // end_turn with zero visible output and reasoning-only content: surface it.
    let notice = Agent::provider_guardrail_notice(Some("end_turn"), true, true)
        .expect("empty visible output must produce a notice");
    assert!(notice.contains("internal reasoning"), "{notice}");
    assert!(notice.contains("end_turn"), "{notice}");
    // Unknown stop reason, empty output, no reasoning.
    let notice = Agent::provider_guardrail_notice(None, true, false)
        .expect("empty visible output must produce a notice");
    assert!(notice.contains("unknown"), "{notice}");
    assert!(!notice.contains("internal reasoning"), "{notice}");
}

#[test]
fn guardrail_notice_absent_for_normal_turns() {
    // Normal turn with visible text: no notice.
    assert!(Agent::provider_guardrail_notice(Some("end_turn"), false, false).is_none());
    assert!(Agent::provider_guardrail_notice(None, false, true).is_none());
}

#[test]
fn empty_turn_log_event_separates_guardrails_from_transient_empties() {
    assert_eq!(
        Agent::empty_turn_log_event(Some("refusal")),
        "PROVIDER_GUARDRAIL"
    );
    assert_eq!(
        Agent::empty_turn_log_event(Some("content_filter")),
        "PROVIDER_GUARDRAIL"
    );
    assert_eq!(
        Agent::empty_turn_log_event(Some("stop")),
        "PROVIDER_EMPTY_RESPONSE"
    );
    assert_eq!(Agent::empty_turn_log_event(None), "PROVIDER_EMPTY_RESPONSE");
}

#[test]
fn guardrail_notice_for_transient_empty_does_not_blame_content_filter() {
    let notice = Agent::provider_guardrail_notice(Some("stop"), true, false)
        .expect("empty visible output must produce a notice");
    assert!(
        !notice.contains("usually a provider-side guardrail"),
        "transient empty responses must not be blamed on a guardrail: {notice}"
    );
    assert!(notice.contains("empty response"), "{notice}");
}

#[tokio::test]
async fn empty_post_tool_response_is_retried_in_shared_helper() {
    let _guard = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);

    let mut attempts = 0u32;
    // Empty response right after tool results: inject continuation.
    let retried = agent
        .maybe_continue_empty_post_tool_response(true, true, Some("stop"), &mut attempts)
        .expect("helper must not error");
    assert!(retried);
    assert_eq!(attempts, 1);
    let recovery = agent
        .session
        .messages
        .last()
        .expect("recovery instruction must be persisted");
    assert_eq!(recovery.role, Role::User);
    assert!(
        recovery
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .is_some_and(|text| text.starts_with("<system-reminder>")),
        "synthetic recovery instruction must be hidden from the transcript"
    );

    // A guardrail refusal is deliberate and must not be retried.
    let retried = agent
        .maybe_continue_empty_post_tool_response(true, true, Some("refusal"), &mut attempts)
        .expect("helper must not error");
    assert!(!retried);

    // Visible output or no recent tool result: no retry.
    assert!(
        !agent
            .maybe_continue_empty_post_tool_response(false, true, Some("stop"), &mut attempts)
            .unwrap()
    );
    assert!(
        !agent
            .maybe_continue_empty_post_tool_response(true, false, Some("stop"), &mut attempts)
            .unwrap()
    );

    // Retry budget is bounded.
    attempts = Agent::MAX_EMPTY_POST_TOOL_CONTINUATION_ATTEMPTS;
    assert!(
        !agent
            .maybe_continue_empty_post_tool_response(true, true, Some("stop"), &mut attempts)
            .unwrap()
    );
}

/// Accepts only bare model ids, the way a provider behaves when a session
/// persisted a route it can no longer serve (`openai:claude-opus-5`).
#[derive(Clone)]
struct RouteRejectingProvider {
    model: Arc<std::sync::Mutex<String>>,
}

#[async_trait]
impl Provider for RouteRejectingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        unreachable!("RouteRejectingProvider does not complete requests")
    }

    fn name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> String {
        self.model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_model(&self, request: &str) -> Result<()> {
        if request.contains(':') {
            anyhow::bail!("Unsupported OpenAI model '{request}'");
        }
        *self
            .model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = request.to_string();
        Ok(())
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

#[tokio::test]
async fn session_model_restore_drops_a_route_the_provider_cannot_serve() {
    let provider: Arc<dyn Provider> = Arc::new(RouteRejectingProvider {
        model: Arc::new(std::sync::Mutex::new("gpt-5.6-sol".to_string())),
    });
    let registry = Registry::new(provider.clone()).await;
    let _guard = crate::storage::lock_test_env();
    let mut agent = Agent::new(provider.clone(), registry);

    // The incoherent triple a spawn leaves behind when its model override
    // fails: an Anthropic model beside an OpenAI provider key, no route. The
    // route-qualified request `openai:claude-opus-5` cannot resolve, and
    // continuing silently would run the session on the provider's own model.
    agent.session.model = Some("claude-opus-5".to_string());
    agent.session.provider_key = Some("openai".to_string());
    agent.session.route_api_method = None;

    agent.restore_model_from_session();

    assert_eq!(
        provider.model(),
        "claude-opus-5",
        "restore must keep the session's model when only its persisted route is unusable"
    );
}
