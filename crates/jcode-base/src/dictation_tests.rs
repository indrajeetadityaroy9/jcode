use super::{last_focused_session, remember_last_focused_session, run_command};

#[tokio::test]
async fn run_command_trims_trailing_newlines() {
    let text = run_command("printf 'hello from test\\n'", 5)
        .await
        .expect("dictation command should succeed");
    assert_eq!(text, "hello from test");
}

#[test]
fn remember_and_read_last_focused_session() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("JCODE_HOME");
    let temp = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", temp.path());

    let active_dir = temp.path().join("active_pids");
    std::fs::create_dir_all(&active_dir).expect("create active_pids");
    std::fs::write(active_dir.join("session_whale_123"), "99999").expect("write active pid");

    remember_last_focused_session("session_whale_123").expect("remember session");
    assert_eq!(
        last_focused_session().expect("read session"),
        Some("session_whale_123".to_string())
    );

    if let Some(prev) = prev {
        crate::env::set_var("JCODE_HOME", prev);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
}
