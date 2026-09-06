use super::*;

fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    ENV_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_temp_jcode_home<T>(f: impl FnOnce() -> T) -> T {
    let _guard = test_env_lock();
    let temp_home = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    jcode_core::env::set_var("JCODE_HOME", temp_home.path());
    let result = f();
    if let Some(prev_home) = prev_home {
        jcode_core::env::set_var("JCODE_HOME", prev_home);
    } else {
        jcode_core::env::remove_var("JCODE_HOME");
    }
    result
}

fn create_git_repo_fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git dir");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"jcode\"\nversion = \"0.0.0\"\n",
    )
    .expect("write Cargo.toml");
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp.path())
        .output()
        .expect("git config email");
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp.path())
        .output()
        .expect("git config name");
    std::process::Command::new("git")
        .args(["add", "Cargo.toml"])
        .current_dir(temp.path())
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(temp.path())
        .output()
        .expect("git commit");
    temp
}

fn source_state_fixture(short_hash: &str, fingerprint: &str) -> SourceState {
    SourceState {
        repo_scope: "repo-scope".to_string(),
        worktree_scope: "worktree-scope".to_string(),
        short_hash: short_hash.to_string(),
        full_hash: format!("{short_hash}-full"),
        dirty: true,
        fingerprint: fingerprint.to_string(),
        version_label: format!("{short_hash}-dirty-{}", &fingerprint[..12]),
        changed_paths: 1,
    }
}

#[test]
fn test_build_manifest_default() {
    let manifest = BuildManifest::default();
    assert!(manifest.stable.is_none());
    assert!(manifest.canary.is_none());
    assert!(manifest.history.is_empty());
}

#[test]
fn test_binary_version_hash_mismatch_rejects_publish_candidate() {
    let source = source_state_fixture("newhash", "123456789abcffff");
    let report = BinaryVersionReport {
        version: Some("v0.0.0-dev (oldhash, dirty)".to_string()),
        git_hash: Some("oldhash".to_string()),
    };

    let error = validate_binary_version_matches_source_report(&report, Path::new("jcode"), &source)
        .expect_err("mismatched git hash should be rejected");

    assert!(
        error
            .to_string()
            .contains("binary was built from git hash oldhash")
    );
}

#[test]
fn test_dev_binary_source_metadata_mismatch_rejects_publish_candidate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let binary = temp.path().join(binary_name());
    std::fs::write(&binary, b"fake").expect("write fake binary");
    let source = source_state_fixture("abc1234", "1111111111112222");
    let stale_source = source_state_fixture("abc1234", "999999999999aaaa");
    write_dev_binary_source_metadata(&binary, &stale_source).expect("write metadata");

    let error = validate_dev_binary_source_metadata(&binary, &source)
        .expect_err("mismatched source metadata should be rejected");

    assert!(error.to_string().contains("source metadata"));
    assert!(error.to_string().contains("999999999999aaaa"));
}

#[test]
fn test_smoke_test_server_protocol_uses_fresh_connection_after_ping() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let temp = tempfile::tempdir().expect("tempdir");
    let socket_path = temp.path().join("smoke.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind unix listener");

    let server = std::thread::spawn(move || {
        let (first, _) = listener.accept().expect("accept ping client");
        let mut first = BufReader::new(first);
        let mut line = String::new();
        first.read_line(&mut line).expect("read ping request");
        assert!(line.contains("\"type\":\"ping\""));
        first
            .get_mut()
            .write_all(b"{\"type\":\"pong\",\"id\":1}\n")
            .expect("write pong");

        let (second, _) = listener.accept().expect("accept subscribe client");
        let mut second = BufReader::new(second);
        line.clear();
        second.read_line(&mut line).expect("read subscribe request");
        assert!(line.contains("\"type\":\"subscribe\""));
        second
            .get_mut()
            .write_all(b"{\"type\":\"ack\",\"id\":2}\n")
            .expect("write subscribe ack");
    });

    smoke_test_server_protocol(&socket_path, "/tmp").expect("smoke test protocol succeeds");
    server.join().expect("server thread join");
}

#[test]
fn test_binary_choice_for_canary_session() {
    let manifest = BuildManifest {
        canary: Some("abc123".to_string()),
        canary_session: Some("session_test".to_string()),
        ..Default::default()
    };

    // Canary session should get canary binary
    match manifest.binary_for_session("session_test") {
        BinaryChoice::Canary(hash) => assert_eq!(hash, "abc123"),
        _ => panic!("Expected canary binary"),
    }

    // Other sessions should get stable (or current if no stable)
    match manifest.binary_for_session("other_session") {
        BinaryChoice::Current => {}
        _ => panic!("Expected current binary"),
    }
}

#[test]
fn test_find_repo_in_ancestors_walks_upward() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("jcode-repo");
    let nested = repo.join("a").join("b").join("c");

    std::fs::create_dir_all(repo.join(".git")).expect("create .git");
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"jcode\"\nversion = \"0.0.0\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(&nested).expect("create nested dirs");

    let found = find_repo_in_ancestors(&nested).expect("repo should be found");
    assert_eq!(found, repo);
}

#[test]
fn test_client_update_candidate_prefers_published_current_channel() {
    let _guard = test_env_lock();
    let temp_home = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    jcode_core::env::set_var("JCODE_HOME", temp_home.path());

    let version = "test-current";
    let version_binary =
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), version)
            .expect("install test version");
    update_current_symlink(version).expect("update current symlink");

    let candidate = client_update_candidate().expect("expected current-channel candidate");
    assert_eq!(candidate.1, "current");
    assert_eq!(
        std::fs::canonicalize(candidate.0).expect("canonical candidate"),
        std::fs::canonicalize(version_binary).expect("canonical version binary")
    );

    if let Some(prev_home) = prev_home {
        jcode_core::env::set_var("JCODE_HOME", prev_home);
    } else {
        jcode_core::env::remove_var("JCODE_HOME");
    }
}

#[test]
fn launcher_dir_uses_sandbox_bin_when_jcode_home_is_set() {
    with_temp_jcode_home(|| {
        let launcher_dir = launcher_dir().expect("launcher dir");
        let expected = storage::jcode_dir().expect("jcode dir").join("bin");
        assert_eq!(launcher_dir, expected);
    });
}

#[test]
fn update_launcher_symlink_stays_inside_sandbox_home() {
    with_temp_jcode_home(|| {
        let version = "sandbox-current";
        let version_binary =
            install_binary_at_version(std::env::current_exe().as_ref().unwrap(), version)
                .expect("install test version");
        update_current_symlink(version).expect("update current symlink");

        let launcher = update_launcher_symlink_to_current().expect("update launcher");
        let expected_launcher = storage::jcode_dir()
            .expect("jcode dir")
            .join("bin")
            .join(binary_name());
        assert_eq!(launcher, expected_launcher);
        assert_eq!(
            std::fs::canonicalize(&launcher).expect("canonical launcher"),
            std::fs::canonicalize(version_binary).expect("canonical version binary")
        );
    });
}

#[test]
fn test_canary_status_serialization() {
    assert_eq!(
        serde_json::to_string(&CanaryStatus::Testing).unwrap(),
        "\"testing\""
    );
    assert_eq!(
        serde_json::to_string(&CanaryStatus::Passed).unwrap(),
        "\"passed\""
    );
}

#[test]
fn dirty_source_state_uses_fingerprint_in_version_label() {
    let repo = create_git_repo_fixture();
    std::fs::write(repo.path().join("notes.txt"), "dirty change\n").expect("write dirty file");

    let state = current_source_state(repo.path()).expect("source state");
    assert!(state.dirty);
    assert!(
        state
            .version_label
            .starts_with(&format!("{}-dirty-", state.short_hash))
    );
    assert!(state.version_label.len() > state.short_hash.len() + 7);
}

#[test]
fn pending_activation_can_complete_and_roll_back() {
    with_temp_jcode_home(|| {
        let current_version = "stable-prev";
        let shared_version = "shared-prev";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), current_version)
            .expect("install previous version");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), shared_version)
            .expect("install previous shared version");
        update_current_symlink(current_version).expect("publish previous current");
        update_shared_server_symlink(shared_version).expect("publish previous shared");

        let mut manifest = BuildManifest::default();
        manifest
            .set_pending_activation(PendingActivation {
                session_id: "session-a".to_string(),
                new_version: "canary-next".to_string(),
                previous_current_version: Some(current_version.to_string()),
                previous_shared_server_version: Some(shared_version.to_string()),
                source_fingerprint: Some("fingerprint-a".to_string()),
                requested_at: Utc::now(),
            })
            .expect("set pending activation");

        let completed = complete_pending_activation_for_session("session-a")
            .expect("complete activation")
            .expect("completed version");
        assert_eq!(completed, "canary-next");
        let manifest = BuildManifest::load().expect("load manifest");
        assert!(manifest.pending_activation.is_none());
        assert_eq!(manifest.canary.as_deref(), Some("canary-next"));
        assert_eq!(manifest.canary_status, Some(CanaryStatus::Passed));

        let mut manifest = BuildManifest::load().expect("reload manifest");
        manifest
            .set_pending_activation(PendingActivation {
                session_id: "session-b".to_string(),
                new_version: "canary-bad".to_string(),
                previous_current_version: Some(current_version.to_string()),
                previous_shared_server_version: Some(shared_version.to_string()),
                source_fingerprint: Some("fingerprint-b".to_string()),
                requested_at: Utc::now(),
            })
            .expect("set second pending activation");

        let rolled_back = rollback_pending_activation_for_session("session-b")
            .expect("rollback activation")
            .expect("rolled back version");
        assert_eq!(rolled_back, "canary-bad");
        let restored = read_current_version()
            .expect("read current version")
            .expect("restored current version");
        assert_eq!(restored, current_version);
        let restored_shared = read_shared_server_version()
            .expect("read shared server version")
            .expect("restored shared server version");
        assert_eq!(restored_shared, shared_version);
    });
}

#[test]
fn explicit_promotion_pins_installed_version_without_moving_client_channels() {
    with_temp_jcode_home(|| {
        let stable = "0.22.0";
        let current = "abc1234-dirty-deadbeef";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), stable)
            .expect("install stable");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), current)
            .expect("install current");
        update_stable_symlink(stable).expect("publish stable");
        update_current_symlink(current).expect("publish current");
        update_shared_server_symlink(stable).expect("shared server initially tracks stable");

        let previous = promote_version_to_shared_server(current).expect("promote current");

        assert_eq!(previous.as_deref(), Some(stable));
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(current)
        );
        assert_eq!(read_current_version().unwrap().as_deref(), Some(current));
        assert_eq!(read_stable_version().unwrap().as_deref(), Some(stable));
        assert!(!shared_server_tracks_stable().expect("explicit promotion should pin the channel"));

        let previous = promote_version_to_shared_server(current).expect("repeat promotion");
        assert_eq!(
            previous.as_deref(),
            Some(current),
            "repeating the same promotion is idempotent"
        );
    });
}

#[test]
fn explicit_promotion_rejects_missing_and_path_like_versions() {
    with_temp_jcode_home(|| {
        let missing = promote_version_to_shared_server("not-installed")
            .expect_err("missing version must be rejected");
        assert!(missing.to_string().contains("Version binary not found"));

        for invalid in ["", "..", "../stable", r"..\stable", "C:\\stable"] {
            let error = promote_version_to_shared_server(invalid)
                .expect_err("path-like version must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("Invalid installed version label"),
                "unexpected error for {invalid:?}: {error:#}"
            );
        }
    });
}

#[test]
fn shared_server_candidate_repairs_stale_shared_channel_to_stable() {
    with_temp_jcode_home(|| {
        let stale_version = "0.14.2";
        let installed_version = "0.17.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), stale_version)
            .expect("install stale shared version");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), installed_version)
            .expect("install installed version");
        update_shared_server_symlink(stale_version).expect("update shared server");
        update_stable_symlink(installed_version).expect("update stable");
        update_current_symlink(installed_version).expect("update current");

        let candidate =
            shared_server_update_candidate().expect("expected stable shared-server candidate");
        assert_eq!(candidate.1, "stable");
        let selected = std::fs::canonicalize(candidate.0).expect("canonical selected");
        let installed = std::fs::canonicalize(version_binary_path(installed_version).unwrap())
            .expect("canonical installed");
        assert_eq!(selected, installed);
    });
}

#[test]
fn shared_server_candidate_allows_shared_channel_matching_stable() {
    with_temp_jcode_home(|| {
        let installed_version = "0.17.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), installed_version)
            .expect("install installed version");
        update_shared_server_symlink(installed_version).expect("update shared server");
        update_stable_symlink(installed_version).expect("update stable");

        let candidate =
            shared_server_update_candidate().expect("expected matching shared-server candidate");
        assert_eq!(candidate.1, "shared-server");
    });
}

#[test]
fn shared_server_candidate_ignores_shared_channel_with_missing_marker() {
    with_temp_jcode_home(|| {
        let shared_version = "0.14.2";
        let installed_version = "0.17.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), shared_version)
            .expect("install shared version");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), installed_version)
            .expect("install installed version");
        update_shared_server_symlink(shared_version).expect("update shared server");
        std::fs::remove_file(shared_server_version_file().unwrap()).expect("remove marker");
        update_stable_symlink(installed_version).expect("update stable");

        let candidate = shared_server_update_candidate()
            .expect("expected stable candidate when shared marker is missing");
        assert_eq!(candidate.1, "stable");
    });
}

#[test]
fn shared_server_candidate_ignores_shared_channel_with_corrupt_marker() {
    with_temp_jcode_home(|| {
        let shared_version = "0.14.2";
        let installed_version = "0.17.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), shared_version)
            .expect("install shared version");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), installed_version)
            .expect("install installed version");
        update_shared_server_symlink(shared_version).expect("update shared server");
        std::fs::write(
            shared_server_version_file().unwrap(),
            "not-the-installed-version",
        )
        .expect("write corrupt marker");
        update_stable_symlink(installed_version).expect("update stable");

        let candidate = shared_server_update_candidate()
            .expect("expected stable candidate when shared marker is corrupt");
        assert_eq!(candidate.1, "stable");
    });
}

#[test]
fn version_match_detects_installed_channel_by_semver_or_git_hash() {
    with_temp_jcode_home(|| {
        std::fs::create_dir_all(builds_dir().unwrap()).expect("create builds dir");
        std::fs::write(stable_version_file().unwrap(), "0.17.0").expect("write stable marker");
        assert!(version_matches_installed_channel(
            "v0.17.0 (abc1234)",
            "different"
        ));
        assert!(!version_matches_installed_channel("v0.14.2", "different"));

        std::fs::write(stable_version_file().unwrap(), "abc1234-dirty-build")
            .expect("write git marker");
        assert!(version_matches_installed_channel(
            "v0.14.2-dev (abc1234)",
            "abc1234"
        ));
    });
}

#[test]
fn shared_server_tracks_stable_when_marker_missing() {
    with_temp_jcode_home(|| {
        std::fs::create_dir_all(builds_dir().unwrap()).expect("create builds dir");
        // No shared-server marker at all: nothing deliberate to protect.
        assert!(shared_server_tracks_stable().expect("tracks stable"));
    });
}

#[test]
fn shared_server_tracks_stable_when_equal_to_stable() {
    with_temp_jcode_home(|| {
        std::fs::create_dir_all(builds_dir().unwrap()).expect("create builds dir");
        std::fs::write(stable_version_file().unwrap(), "0.17.0").expect("write stable");
        std::fs::write(shared_server_version_file().unwrap(), "0.17.0").expect("write shared");
        assert!(shared_server_tracks_stable().expect("tracks stable"));
    });
}

#[test]
fn shared_server_does_not_track_stable_when_pinned_to_local_build() {
    with_temp_jcode_home(|| {
        std::fs::create_dir_all(builds_dir().unwrap()).expect("create builds dir");
        std::fs::write(stable_version_file().unwrap(), "0.17.0").expect("write stable");
        std::fs::write(
            shared_server_version_file().unwrap(),
            "56f43c3d-dirty-deadbeef",
        )
        .expect("write shared");
        assert!(!shared_server_tracks_stable().expect("does not track stable"));
    });
}

#[test]
fn advance_shared_server_carries_forward_when_tracking_stable() {
    with_temp_jcode_home(|| {
        let old = "0.17.0";
        let new = "0.18.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), old)
            .expect("install old");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), new)
            .expect("install new");
        update_stable_symlink(old).expect("stable old");
        update_shared_server_symlink(old).expect("shared old");

        let advanced = advance_shared_server_if_tracking_stable(new).expect("advance");
        assert!(advanced);
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(new),
            "shared-server should follow the update"
        );
    });
}

#[test]
fn advance_shared_server_preserves_pinned_local_build() {
    with_temp_jcode_home(|| {
        let stable_old = "0.17.0";
        let pinned = "56f43c3d-dirty-deadbeef";
        let update = "0.18.0";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), stable_old)
            .expect("install stable");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), pinned)
            .expect("install pinned build");
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), update)
            .expect("install update");
        update_stable_symlink(stable_old).expect("stable");
        update_shared_server_symlink(pinned).expect("shared pinned");

        let advanced = advance_shared_server_if_tracking_stable(update).expect("advance");
        assert!(!advanced, "must not advance a deliberately-promoted build");
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(pinned),
            "a pinned shared-server build must be preserved across update"
        );
    });
}

/// Simulate the channel mutations performed by `/update`'s stable install path
/// (`download_and_install_blocking_with_progress`), without doing any network
/// I/O. This is the exact sequence: advance shared-server if tracking stable,
/// then move stable/current/launcher to the freshly installed version.
fn simulate_stable_update_channel_swap(new_version: &str) {
    install_binary_at_version(std::env::current_exe().as_ref().unwrap(), new_version)
        .expect("install update version");
    // /update tries to carry the daemon's reload target forward, but only when
    // shared-server is tracking stable.
    advance_shared_server_if_tracking_stable(new_version).expect("advance shared-server");
    update_stable_symlink(new_version).expect("update stable");
    update_current_symlink(new_version).expect("update current");
    update_launcher_symlink_to_current().expect("update launcher");
}

/// Resolve the binary the long-lived daemon would actually reload into. This
/// mirrors `reload_exec_target` / `server_update_candidate` in the server, which
/// both go through `shared_server_update_candidate`.
fn daemon_reload_target_version() -> Option<String> {
    let (candidate, _label) = shared_server_update_candidate()?;
    let canonical = std::fs::canonicalize(&candidate).unwrap_or(candidate);
    // versions/<version>/jcode -> <version>
    canonical
        .parent()
        .and_then(|p| p.file_name())
        .map(|name| name.to_string_lossy().into_owned())
}

/// Reproduces the user-reported "/update gives the new client but a stale
/// server" bug.
///
/// Repro setup matches a real machine state observed in the field: the
/// `shared-server` channel is pinned to a local build that differs from
/// `stable`. When the user runs `/update`, the client channels advance to the
/// new release, but `advance_shared_server_if_tracking_stable` refuses to move
/// the pinned shared-server channel, so the daemon's reload target stays on the
/// old pinned binary forever.
///
/// EXPECTED (post-fix): after `/update`, the daemon's reload target resolves to
/// the freshly installed release version, so a reconnecting client can upgrade
/// the server too.
#[test]
fn update_leaves_daemon_reload_target_stale_when_shared_server_pinned() {
    with_temp_jcode_home(|| {
        // Field state: client + server both on an old local build.
        let old_pinned = "3f160da1-dirty-e756d52efca9";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), old_pinned)
            .expect("install old pinned build");
        // `stable` lags behind (a previously released version).
        let old_stable = "0.14.3";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), old_stable)
            .expect("install old stable");
        update_stable_symlink(old_stable).expect("stable");
        update_current_symlink(old_pinned).expect("current pinned");
        update_shared_server_symlink(old_pinned).expect("shared-server pinned");

        // User runs `/update`: a newer release ships and the client installs it.
        let new_release = "0.15.0";
        simulate_stable_update_channel_swap(new_release);

        // Client side is upgraded: current + stable now point at the release.
        assert_eq!(
            read_current_version().unwrap().as_deref(),
            Some(new_release),
            "client `current` channel should advance on /update"
        );
        assert_eq!(
            read_stable_version().unwrap().as_deref(),
            Some(new_release),
            "client `stable` channel should advance on /update"
        );

        // Server side: what would the daemon reload into? This is the bug.
        let target = daemon_reload_target_version();
        assert_eq!(
            target.as_deref(),
            Some(new_release),
            "BUG: after /update the daemon's reload target is still stale \
             (shared-server pinned to {old_pinned}); the user gets a new client \
             but the long-lived server never upgrades. shared-server-version={:?}",
            read_shared_server_version().unwrap()
        );
    });
}

/// Control case: when `shared-server` is tracking `stable` (the normal
/// install), `/update` correctly advances the daemon's reload
/// target. This guards against a fix that over-corrects and breaks the healthy
/// path.
#[test]
fn update_advances_daemon_reload_target_when_shared_server_tracks_stable() {
    with_temp_jcode_home(|| {
        let old_release = "0.14.3";
        install_binary_at_version(std::env::current_exe().as_ref().unwrap(), old_release)
            .expect("install old release");
        update_stable_symlink(old_release).expect("stable");
        update_current_symlink(old_release).expect("current");
        update_shared_server_symlink(old_release).expect("shared-server tracks stable");

        let new_release = "0.15.0";
        simulate_stable_update_channel_swap(new_release);

        assert_eq!(
            daemon_reload_target_version().as_deref(),
            Some(new_release),
            "daemon reload target should advance with /update when tracking stable"
        );
    });
}

/// Write a distinct, real binary into `versions/<version>/jcode` with an
/// explicit mtime so channel-repair mtime comparisons are deterministic
/// (install_binary_at_version hard-links and would share an mtime).
fn write_versioned_binary(version: &str, mtime: std::time::SystemTime) -> PathBuf {
    let dir = builds_dir().unwrap().join("versions").join(version);
    std::fs::create_dir_all(&dir).expect("create version dir");
    let path = dir.join(binary_name());
    std::fs::write(&path, format!("bin {version}")).expect("write binary");
    std::fs::File::open(&path)
        .expect("open binary")
        .set_modified(mtime)
        .expect("set mtime");
    path
}

#[test]
fn repair_repoints_stale_shared_server_to_newer_stable() {
    use std::time::{Duration, SystemTime};
    with_temp_jcode_home(|| {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let old = "0.14.6";
        let new = "0.22.0";
        // shared-server pinned to the OLD build; stable advanced to the NEW
        // release (the "current client, no-op /update, stale server" state).
        write_versioned_binary(old, base);
        write_versioned_binary(new, base + Duration::from_secs(60));
        update_shared_server_symlink(old).expect("pin shared-server old");
        update_stable_symlink(new).expect("stable new");

        let outcome = repair_stale_shared_server_channel().expect("repair");
        assert_eq!(
            outcome,
            SharedServerRepair::Repaired {
                previous: Some(old.to_string()),
                repaired_to: new.to_string(),
            },
        );
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(new),
            "shared-server should be dragged forward to stable"
        );
    });
}

#[test]
fn repair_is_noop_when_shared_server_already_matches_stable() {
    use std::time::{Duration, SystemTime};
    with_temp_jcode_home(|| {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let v = "0.22.0";
        write_versioned_binary(v, base);
        update_shared_server_symlink(v).expect("shared");
        update_stable_symlink(v).expect("stable");

        assert_eq!(
            repair_stale_shared_server_channel().expect("repair"),
            SharedServerRepair::AlreadyCurrent,
        );
        assert_eq!(read_shared_server_version().unwrap().as_deref(), Some(v));
    });
}

#[test]
fn repair_preserves_fresher_local_pin() {
    use std::time::{Duration, SystemTime};
    with_temp_jcode_home(|| {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let stable_old = "0.14.3";
        let pinned_new = "56f43c3d-dirty-deadbeef";
        // A deliberately-promoted build that is NEWER than stable must be
        // preserved (the whole point of pinning shared-server).
        write_versioned_binary(stable_old, base);
        write_versioned_binary(pinned_new, base + Duration::from_secs(120));
        update_stable_symlink(stable_old).expect("stable");
        update_shared_server_symlink(pinned_new).expect("pin newer local build");

        assert_eq!(
            repair_stale_shared_server_channel().expect("repair"),
            SharedServerRepair::AlreadyCurrent,
            "must not downgrade a fresher local pin to an older stable"
        );
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(pinned_new),
        );
    });
}

#[test]
fn repair_preserves_older_local_pin() {
    use std::time::{Duration, SystemTime};
    with_temp_jcode_home(|| {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let pinned_old = "56f43c3d-dirty-deadbeef";
        let stable_new = "0.22.0";
        write_versioned_binary(pinned_old, base);
        write_versioned_binary(stable_new, base + Duration::from_secs(120));
        update_shared_server_symlink(pinned_old).expect("pin older local build");
        update_stable_symlink(stable_new).expect("stable new");

        assert_eq!(
            repair_stale_shared_server_channel().expect("repair"),
            SharedServerRepair::AlreadyCurrent,
            "repair must not overwrite a deliberately-pinned local build"
        );
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(pinned_old),
        );
    });
}

#[test]
fn repair_never_downgrades_when_stable_is_older() {
    use std::time::{Duration, SystemTime};
    with_temp_jcode_home(|| {
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let shared_new = "0.22.0";
        let stable_old = "0.14.3";
        write_versioned_binary(stable_old, base);
        write_versioned_binary(shared_new, base + Duration::from_secs(90));
        update_shared_server_symlink(shared_new).expect("shared new");
        update_stable_symlink(stable_old).expect("stable old");

        assert_eq!(
            repair_stale_shared_server_channel().expect("repair"),
            SharedServerRepair::AlreadyCurrent,
            "repair must never move shared-server backward to an older stable"
        );
        assert_eq!(
            read_shared_server_version().unwrap().as_deref(),
            Some(shared_new),
        );
    });
}
