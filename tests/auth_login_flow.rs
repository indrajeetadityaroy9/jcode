use anyhow::Result;
use jcode::auth::AuthStatus;
use jcode::provider_catalog::login_providers;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TRACKED_ENV_VARS: [&str; 4] = ["HOME", "APPDATA", "XDG_CONFIG_HOME", "JCODE_HOME"];

struct TestEnv {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
    _temp: tempfile::TempDir,
}

impl TestEnv {
    fn new() -> Result<Self> {
        let lock = lock_env();
        let temp = tempfile::Builder::new()
            .prefix("jcode-auth-flow-")
            .tempdir()?;
        let saved = TRACKED_ENV_VARS
            .into_iter()
            .map(|key| (key, std::env::var(key).ok()))
            .collect::<Vec<_>>();

        jcode::env::set_var("HOME", temp.path());
        jcode::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
        jcode::env::set_var("APPDATA", temp.path().join("AppData").join("Roaming"));
        jcode::env::set_var("JCODE_HOME", temp.path().join("jcode-home"));
        AuthStatus::invalidate_cache();

        Ok(Self {
            _lock: lock,
            saved,
            _temp: temp,
        })
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        AuthStatus::invalidate_cache();
        for (key, value) in &self.saved {
            if let Some(value) = value {
                jcode::env::set_var(key, value);
            } else {
                jcode::env::remove_var(key);
            }
        }
        AuthStatus::invalidate_cache();
    }
}

#[test]
fn every_login_provider_has_auth_status_and_tui_copy_metadata() -> Result<()> {
    let _env = TestEnv::new()?;
    let status = AuthStatus::default();

    for provider in login_providers() {
        assert!(!provider.id.trim().is_empty(), "provider id must be set");
        assert!(
            !provider.display_name.trim().is_empty(),
            "{} display name must be set",
            provider.id
        );
        assert!(
            !provider.menu_detail.trim().is_empty(),
            "{} setup copy must be set",
            provider.id
        );
        assert!(
            !provider.auth_kind.label().trim().is_empty(),
            "{} auth kind label must be set",
            provider.id
        );
        assert!(
            !status
                .method_detail_for_provider(*provider)
                .trim()
                .is_empty(),
            "{} detected setup copy must be renderable",
            provider.id
        );
    }

    Ok(())
}
