//! The single source of truth for every slash command.
//!
//! One entry declares a command's canonical name, its aliases, whether it is
//! offered in autocomplete, and its one-line summary (shown in the slash
//! palette).
//! Adding a command means adding one entry here plus its handler: nothing else
//! needs to learn the name, and aliases never reach a dispatcher because
//! [`canonical_input`] rewrites them first.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Visibility {
    /// Offered in the slash palette.
    Public,
    /// Public, but only meaningful when attached to a server.
    Remote,
    /// Dispatched when typed, never advertised.
    Hidden,
}

pub(crate) struct CommandSpec {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) visibility: Visibility,
    pub(crate) summary: &'static str,
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/model",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "List or switch models",
    },
    CommandSpec {
        name: "/provider-test-coverage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show live-test evidence for the current provider/model",
    },
    CommandSpec {
        name: "/refresh-model-list",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Refresh provider model catalogs",
    },
    CommandSpec {
        name: "/agents",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Configure models for agent roles",
    },
    CommandSpec {
        name: "/swarm-prompt",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the active swarm routing prompt in your editor",
    },
    CommandSpec {
        name: "/subagent",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a subagent manually",
    },
    CommandSpec {
        name: "/observe",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the latest tool context in the side panel",
    },
    CommandSpec {
        name: "/todos",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the session todo list as a card in the chat",
    },
    CommandSpec {
        name: "/splitview",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Mirror the current chat in the side panel",
    },
    CommandSpec {
        name: "/btw",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Ask a side question in the side panel",
    },
    CommandSpec {
        name: "/ssh",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Connect to a remote machine using system SSH",
    },
    CommandSpec {
        name: "/git",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show git status for the session working directory",
    },
    CommandSpec {
        name: "/colors",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "List, configure, and score every TUI color",
    },
    CommandSpec {
        name: "/terminal-setup",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Fix Shift+Enter newlines",
    },
    CommandSpec {
        name: "/commit",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Make logical commits from current changes",
    },
    CommandSpec {
        name: "/commit-push",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Make logical commits from current changes, then push",
    },
    CommandSpec {
        name: "/triage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Triage new GitHub issues and autonomously fix the safe ones",
    },
    CommandSpec {
        name: "/transcript",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the current session transcript file",
    },
    CommandSpec {
        name: "/subagent-model",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change subagent model policy",
    },
    CommandSpec {
        name: "/autoreview",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle automatic end-of-turn review",
    },
    CommandSpec {
        name: "/autojudge",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle automatic end-of-turn judging",
    },
    CommandSpec {
        name: "/review",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a one-shot headed review session",
    },
    CommandSpec {
        name: "/judge",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a one-shot headed judge session",
    },
    CommandSpec {
        name: "/effort",
        aliases: &[],
        visibility: Visibility::Public,
        summary: crate::tui::keybind::EFFORT_HELP,
    },
    CommandSpec {
        name: "/transport",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change connection transport",
    },
    CommandSpec {
        name: "/alignment",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change default text alignment",
    },
    CommandSpec {
        name: "/compact-notifications",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle single-line swarm/file-activity notifications",
    },
    CommandSpec {
        name: "/show-agentgrep-output",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle full agentgrep search output inline in chat",
    },
    CommandSpec {
        name: "/tool-call-details",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle dimmed technical details on tool rows with an intent",
    },
    CommandSpec {
        name: "/thinking-display",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/hide the model's thinking text (off/full/current)",
    },
    CommandSpec {
        name: "/cancel",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Cancel the current prompt or operation",
    },
    CommandSpec {
        name: "/clear",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Clear conversation history",
    },
    CommandSpec {
        name: "/cls",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Clear the view only, keeping context",
    },
    CommandSpec {
        name: "/rewind",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Rewind conversation to previous message",
    },
    CommandSpec {
        name: "/poke",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Poke model to resume with incomplete todos",
    },
    CommandSpec {
        name: "/plan",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Create a plan-only response as a plan card",
    },
    CommandSpec {
        name: "/improve",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Autonomously improve the repository",
    },
    CommandSpec {
        name: "/refactor",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Run a safe refactor loop",
    },
    CommandSpec {
        name: "/compact",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Compact context",
    },
    CommandSpec {
        name: "/fix",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Recover when the model cannot continue",
    },
    CommandSpec {
        name: "/memory",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle memory feature",
    },
    CommandSpec {
        name: "/test",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Verify a claim/current changes with layered tests",
    },
    CommandSpec {
        name: "/initiatives",
        aliases: &["/goals", "/goal", "/mission"],
        visibility: Visibility::Public,
        summary: "Open initiatives overview / resume tracked initiatives",
    },
    CommandSpec {
        name: "/swarm",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle swarm feature",
    },
    CommandSpec {
        name: "/overnight",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Run a supervised overnight coordinator",
    },
    CommandSpec {
        name: "/context",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the full session context snapshot",
    },
    CommandSpec {
        name: "/skills",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show loaded skills and jcode-endorsed recommendations",
    },
    CommandSpec {
        name: "/version",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show current version",
    },
    CommandSpec {
        name: "/info",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show session info and tokens",
    },
    CommandSpec {
        name: "/usage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show connected provider usage limits",
    },
    CommandSpec {
        name: "/productivity",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Generate a shareable usage report + dashboard image",
    },
    CommandSpec {
        name: "/config",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show or edit configuration",
    },
    CommandSpec {
        name: "/log",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Mark the current location in the jcode logs",
    },
    CommandSpec {
        name: "/keys",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show keybinding conflicts with your terminal and OS (/keys refresh to rescan)",
    },
    CommandSpec {
        name: "/diff",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Cycle or set diff display mode (off/inline/full/pinned/file)",
    },
    CommandSpec {
        name: "/reload",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Reload into newest available binary",
    },
    CommandSpec {
        name: "/restart",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Restart with current binary",
    },
    CommandSpec {
        name: "/rebuild",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Background rebuild and auto reload",
    },
    CommandSpec {
        name: "/resume",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open session picker",
    },
    CommandSpec {
        name: "/active",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Manage live sessions (working vs ready)",
    },
    CommandSpec {
        name: "/catchup",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open Catch Up picker",
    },
    CommandSpec {
        name: "/back",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Return to the previous Catch Up session",
    },
    CommandSpec {
        name: "/save",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Bookmark session for easy access",
    },
    CommandSpec {
        name: "/unsave",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Remove bookmark from session",
    },
    CommandSpec {
        name: "/rename",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Rename current session",
    },
    CommandSpec {
        name: "/fork",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Fork session into a new window (optional prompt)",
    },
    CommandSpec {
        name: "/transfer",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Compact context into a fresh handoff session",
    },
    CommandSpec {
        name: "/workspace",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Niri-style session workspace",
    },
    CommandSpec {
        name: "/quit",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Exit jcode",
    },
    CommandSpec {
        name: "/auth",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show authentication status",
    },
    CommandSpec {
        name: "/login",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Login to a provider",
    },
    CommandSpec {
        name: "/logout",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Log out of a provider",
    },
    CommandSpec {
        name: "/account",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the combined account picker",
    },
    CommandSpec {
        name: "/cache",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show cache stats or set cache TTL",
    },
    CommandSpec {
        name: "/debug-visual",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle visual debug overlay",
    },
    CommandSpec {
        name: "/client-reload",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Force reload client binary",
    },
    CommandSpec {
        name: "/server-reload",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Force reload server binary",
    },
    CommandSpec {
        name: "/continue",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Continue every interrupted live session that would auto-resume",
    },
];

/// Canonical spec for a command token, accepting either the canonical name or
/// any declared alias.
pub(crate) fn spec_for(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name || spec.aliases.iter().any(|alias| *alias == name))
}

/// Split `trimmed` into its command spec and the remainder (leading whitespace
/// included), resolving aliases.
///
/// The token must end at whitespace or end-of-input, so `/modelx` is not a
/// match for `/model`.
pub(crate) fn resolve(trimmed: &str) -> Option<(&'static CommandSpec, &str)> {
    if !trimmed.starts_with('/') {
        return None;
    }
    let token_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (token, rest) = trimmed.split_at(token_end);
    spec_for(token).map(|spec| (spec, rest))
}

/// Rewrite `trimmed` so its command token is the canonical name.
///
/// Returns `None` when the input is not a known command *or* already uses the
/// canonical name, so callers only pay for an allocation when an alias was
/// actually typed. This is what keeps aliases out of every dispatcher: they are
/// normalized once at the entry point instead of being matched again per site.
pub(crate) fn canonical_input(trimmed: &str) -> Option<String> {
    let (spec, rest) = resolve(trimmed)?;
    if trimmed.len() == spec.name.len() + rest.len() && trimmed.starts_with(spec.name) {
        return None;
    }
    Some(format!("{}{}", spec.name, rest))
}

/// Entries offered in the slash palette, in registration order.
pub(crate) fn advertised() -> impl Iterator<Item = &'static CommandSpec> {
    COMMANDS
        .iter()
        .filter(|spec| spec.visibility != Visibility::Hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_and_alias_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for spec in COMMANDS {
            assert!(
                spec.name.starts_with('/'),
                "{} must start with a slash",
                spec.name
            );
            assert!(seen.insert(spec.name), "duplicate command {}", spec.name);
            for alias in spec.aliases {
                assert!(seen.insert(alias), "duplicate alias {alias}");
            }
        }
    }

    #[test]
    fn every_advertised_command_has_a_summary() {
        for spec in advertised() {
            assert!(
                !spec.summary.trim().is_empty(),
                "{} has no summary, so autocomplete would show a blank row",
                spec.name
            );
        }
    }

    #[test]
    fn aliases_resolve_to_their_owner() {
        for spec in COMMANDS {
            for alias in spec.aliases {
                let resolved = spec_for(alias).expect("alias must resolve");
                assert_eq!(resolved.name, spec.name, "{alias} resolved elsewhere");
            }
        }
    }

    #[test]
    fn canonical_input_rewrites_aliases_and_keeps_arguments() {
        assert_eq!(canonical_input("/goal").as_deref(), Some("/initiatives"));
        assert_eq!(
            canonical_input("/mission show abc").as_deref(),
            Some("/initiatives show abc")
        );
        // `/goals` must not be read as `/goal` with a trailing "s".
        assert_eq!(canonical_input("/goals").as_deref(), Some("/initiatives"));
        // Canonical input needs no rewrite, and unknown input is left alone.
        assert_eq!(canonical_input("/initiatives"), None);
        assert_eq!(canonical_input("/nope"), None);
    }

    #[test]
    fn resolve_requires_a_token_boundary() {
        assert!(resolve("/model").is_some());
        assert!(resolve("/model gpt-5").is_some());
        assert!(resolve("/modelx").is_none());
        assert!(resolve("hello /model").is_none());
    }
}
