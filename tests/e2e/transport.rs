use crate::test_support::*;

#[tokio::test]
async fn test_unix_socket_transport_subscribe_history_message_and_resume() -> Result<()> {
    let _env = setup_test_env()?;
    let unix = run_unix_transport_scenario().await?;

    assert!(
        unix.subscribe_events
            .iter()
            .any(|event| matches!(event, ServerEvent::Ack { id } if *id == 1))
    );
    assert!(
        unix.subscribe_events
            .iter()
            .any(|event| matches!(event, ServerEvent::Done { id } if *id == 1))
    );

    let unix_history = unix
        .history_events
        .iter()
        .find_map(summarize_history_invariant)
        .ok_or_else(|| anyhow::anyhow!("missing unix history event"))?;
    assert!(
        !unix_history.is_empty(),
        "history payload should summarize to a non-empty invariant"
    );

    let unix_resume = unix
        .resume_events
        .iter()
        .find_map(summarize_history_invariant)
        .ok_or_else(|| anyhow::anyhow!("missing unix resume history event"))?;
    assert_eq!(
        unix_history, unix_resume,
        "resume should replay the same history payload"
    );

    Ok(())
}
