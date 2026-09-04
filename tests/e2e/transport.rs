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
    // The scenario sends a message between the two snapshots, so the message
    // count legitimately grows (0 -> 2) and the request id differs. What must
    // hold is that resume replays the *same session metadata* and includes the
    // completed exchange.
    let strip_volatile = |summary: &str| -> String {
        summary
            .split(':')
            .filter(|field| !field.starts_with("messages=") && !field.parse::<u64>().is_ok())
            .collect::<Vec<_>>()
            .join(":")
    };
    assert_eq!(
        strip_volatile(&unix_history),
        strip_volatile(&unix_resume),
        "resume should replay the same session metadata"
    );
    assert!(
        unix_history.contains(":messages=0:"),
        "history before the exchange should be empty: {unix_history}"
    );
    assert!(
        unix_resume.contains(":messages=2:"),
        "resume should replay the completed exchange: {unix_resume}"
    );

    Ok(())
}
