//! Mapping from parsed CLI arguments to an initial process title.
//!
//! This logic depends on the clap `Args`/`Command` types defined in `cli`, so
//! it lives in the CLI layer. The low-level title-setting primitives it uses
//! (`compact_process_title`, `session_name`, `set_title`) live in the
//! `process_title` core module.

use crate::cli::args::{AmbientCommand, Args, Command};
use crate::process_title::{compact_process_title, session_name, set_title};

pub(crate) fn initial_title(args: &Args) -> String {
    match &args.command {
        Some(Command::Serve { .. }) => "jcode:server".to_string(),
        Some(Command::Acp) => "jcode acp".to_string(),
        Some(Command::Server { .. }) => "jcode server".to_string(),
        Some(Command::Connect) => "jcode:client".to_string(),
        #[cfg(unix)]
        Some(Command::ApiBridge { .. }) => "jcode api-bridge".to_string(),
        Some(Command::Run { .. }) => "jcode run".to_string(),
        Some(Command::Login { .. }) => "jcode login".to_string(),
        Some(Command::Repl) => "jcode repl".to_string(),
        Some(Command::Version { .. }) => "jcode version".to_string(),
        Some(Command::Usage { .. }) => "jcode usage".to_string(),
        Some(Command::Debug { .. }) => "jcode debug".to_string(),
        Some(Command::Auth(_)) => "jcode auth".to_string(),
        Some(Command::Provider(_)) => "jcode provider".to_string(),
        Some(Command::Memory(_)) => "jcode memory".to_string(),
        Some(Command::Session(_)) => "jcode session".to_string(),
        Some(Command::Ambient(AmbientCommand::RunVisible)) => "jcode ambient visible".to_string(),
        Some(Command::Transcript { .. }) => "jcode transcript".to_string(),
        Some(Command::Replay { .. }) => "jcode replay".to_string(),
        Some(Command::Model(_)) => "jcode model".to_string(),
        Some(Command::ProviderTestCoverage { .. }) => "jcode provider-test-coverage".to_string(),
        Some(Command::ProviderDoctor { .. }) => "jcode provider-doctor".to_string(),
        Some(Command::AuthTest { .. }) => "jcode auth-test".to_string(),
        Some(Command::Restart { .. }) => "jcode restart".to_string(),
        Some(Command::Menubar { .. }) => "jcode menubar".to_string(),
        None => {
            if let Some(resume) = args.resume.as_deref().filter(|resume| !resume.is_empty()) {
                compact_process_title("jcode:c:", Some(&session_name(resume)))
            } else {
                "jcode:client".to_string()
            }
        }
    }
}

pub(crate) fn set_initial_title(args: &Args) {
    set_title(initial_title(args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::lock_test_env;
    use clap::Parser;

    fn with_env_lock<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_test_env();
        f()
    }

    #[test]
    fn initial_title_labels_server() {
        with_env_lock(|| {
            let args = Args::parse_from(["jcode", "serve"]);
            assert_eq!(initial_title(&args), "jcode:server");
        });
    }

    #[test]
    fn initial_title_labels_resume_client_with_short_name() {
        with_env_lock(|| {
            let args = Args::parse_from(["jcode", "--resume", "session_fox_123"]);
            assert_eq!(initial_title(&args), "jcode:c:fox");
        });
    }
}
