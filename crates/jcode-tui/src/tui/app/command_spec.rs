//! The single source of truth for every slash command.
//!
//! One entry declares a command's canonical name, its aliases, whether it is
//! offered in autocomplete, its one-line summary (used by the suggestion palette
//! and the `/help` overlay) and its optional `/help <command>` detail page.
//! Adding a command means adding one entry here plus its handler: nothing else
//! needs to learn the name, and aliases never reach a dispatcher because
//! [`canonical_input`] rewrites them first.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Visibility {
    /// Offered in autocomplete and listed by `/help`.
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
    /// `/help <command>` page. `None` means the summary is the whole story.
    pub(crate) detail: Option<&'static str>,
    /// Detail page only applies while attached to a server.
    pub(crate) detail_requires_remote: bool,
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show help and keyboard shortcuts",
        detail: Some(
            "/help\nShow general command list and keyboard shortcuts.\n\n/help <command>\nShow detailed help for one command.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/model",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "List or switch models",
        detail: Some(
            "/model\nOpen model picker.\n\n/model <name>\nSwitch model.\n\n/model <name>@<provider>\nPin OpenRouter routing (@auto clears pin).",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/provider-test-coverage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show live-test evidence for the current provider/model",
        detail: Some(
            "/provider-test-coverage\nShow jcode live verification evidence for the current provider/model.\n\n/provider-test-coverage <provider> <model>\nLook up a specific provider/model pair in the live-test coverage ledger.\n\nThe report shows last-tested time, jcode build, passed/missing checkpoints, readiness gaps, and a caveat that missing evidence is not a provider failure.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/refresh-model-list",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Refresh provider model catalogs",
        detail: Some(
            "/refresh-model-list\nForce-refresh provider model catalogs, update /model, and persist the refreshed cache.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/agents",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Configure models for agent roles",
        detail: Some(
            "/agents\nOpen the agent-model config picker.\n\n/agents <swarm|review|judge|memory|ambient>\nJump straight to that agent role's saved model override.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/swarm-prompt",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the active swarm routing prompt in your editor",
        detail: Some(
            "/swarm-prompt\nOpen the active swarm routing prompt in $VISUAL or $EDITOR.\n\nJcode uses a nonblank project override at ./.jcode/swarm-prompt.md when present, then ~/.jcode/swarm-prompt.md, then the built-in default. If no editable override exists, this command creates the global file from the built-in default. Restart or reload Jcode after editing because running agent tool registries cache the prompt.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/subagent",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a subagent manually",
        detail: Some(
            "/subagent <prompt>\nLaunch a subagent immediately.\n\nOptional flags:\n  --type <kind>         sets the subagent type (default general)\n  --model <name>        overrides the subagent model for this run\n  --continue <id>       resumes an existing subagent session",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/observe",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the latest tool context in the side panel",
        detail: Some(
            "/observe\nToggle transient observe mode for the side panel.\n\n/observe on\nEnable observe mode and focus the observe page.\n\n/observe off\nDisable observe mode.\n\n/observe status\nShow whether observe mode is enabled.\n\nObserve mode shows only the latest tool call or tool result added to context, and it is not persisted to disk.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/todos",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the session todo list as a card in the chat",
        detail: Some(
            "/todos\nShow the current session's todo list as an inline card in the chat (press again, or the todo-card hotkey, to dismiss the trailing card). The card live-updates as the todo list changes.\n\n/todos panel\nToggle the legacy dedicated todo screen in the side panel.\n\n/todos pin\nToggle pinning the full todo list to the top of the chat transcript while it scrolls (saved to config as display.pin_todos, off by default).\n\n/todos on\nEnable the side-panel todo screen and focus it.\n\n/todos off\nDisable the side-panel todo screen.\n\n/todos status\nShow todo card/panel/pin status.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/splitview",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Mirror the current chat in the side panel",
        detail: Some(
            "/splitview\nToggle a transient split view that mirrors the current chat in the side panel.\n\n/splitview on\nEnable split view and focus the mirrored chat page.\n\n/splitview off\nDisable split view.\n\n/splitview status\nShow whether split view is enabled.\n\nThis gives the side panel its own scroll position for the same conversation so you can read older context while keeping the main composer active.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/btw",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Ask a side question in the side panel",
        detail: Some(
            "/btw <question>\nAsk a side question without derailing the current session.\n\nForks (splits) the session into a new window with the full conversation cloned, and the forked session starts by answering the question. The original session keeps working uninterrupted.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/ssh",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Connect to a remote machine using system SSH",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/git",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show git status for the session working directory",
        detail: Some(
            "/git\nShow git status --short --branch for the current session working directory.\n\n/git status\nAlias for /git.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/colors",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "List, configure, and score every TUI color",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/hotkeys",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "List hotkeys with your personal usage",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/terminal-setup",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Fix Shift+Enter newlines",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/commit",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Make logical commits from current changes",
        detail: Some(
            "/commit\nAsk the agent to inspect current uncommitted changes and create interactive, logical commits.\n\nThe agent should group related files or hunks, preserve unrelated work, validate as appropriate, and report the commits created plus anything left uncommitted.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/commit-push",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Make logical commits from current changes, then push",
        detail: Some(
            "/commit-push\nSame as /commit, then push the new commits to the remote tracking branch.\n\nThe agent groups related changes into logical commits, preserves unrelated work, then runs git push (using git push -u if the branch has no upstream). It will not force-push or rewrite already-pushed history, and reports the commits created plus the push result.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/triage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Triage new GitHub issues and autonomously fix the safe ones",
        detail: Some(
            "/triage [focus]\nTriage open GitHub issues for the current repo, then autonomously fix the safe ones.\n\nThe agent lists untriaged issues with gh, classifies each (auto-fix, needs-info, needs-human, duplicate, question), applies existing labels, fixes and verifies the clear-cut bugs, and reports back with a summary table. Every public comment is clearly signed as the Jcode agent, and issues are never closed as wontfix/invalid without your confirmation.\n\nOptional focus text narrows the triage, for example /triage only crash reports.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/transcript",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the current session transcript file",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/subagent-model",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change subagent model policy",
        detail: Some(
            "/subagent-model\nShow the current subagent model policy for this session.\n\n/subagent-model <name>\nPin a fixed model for future subagents in this session.\n\n/subagent-model inherit\nReset to using the current active model.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/autoreview",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle automatic end-of-turn review",
        detail: Some(
            "/autoreview\nShow autoreview status for this session.\n\n/autoreview on\nEnable end-of-turn autoreview for this session.\n\n/autoreview off\nDisable autoreview for this session.\n\n/autoreview now\nLaunch a headed reviewer immediately in a new window.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/autojudge",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle automatic end-of-turn judging",
        detail: Some(
            "/autojudge\nShow autojudge status for this session.\n\n/autojudge on\nEnable end-of-turn autojudge for this session. The autojudge acts like a completion manager: it tells the parent agent either to continue with specific next steps or that it is fine to stop.\n\n/autojudge off\nDisable autojudge for this session.\n\n/autojudge now\nLaunch a headed autojudge immediately in a new window.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/review",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a one-shot headed review session",
        detail: Some(
            "/review\nLaunch a one-shot headed review session immediately.\n\nThe reviewer will DM this session when done. If OpenAI ChatGPT OAuth is available, it prefers gpt-5.5.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/judge",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Launch a one-shot headed judge session",
        detail: Some(
            "/judge\nLaunch a one-shot headed judge session immediately.\n\nThe judge will DM this session when done. If OpenAI ChatGPT OAuth is available, it prefers gpt-5.5.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/effort",
        aliases: &[],
        visibility: Visibility::Public,
        summary: crate::tui::keybind::EFFORT_HELP,
        detail: Some(
            "/effort\nShow current effort.\n\n/effort <level>\nSet effort (none|minimal|low|medium|high|xhigh|max|swarm|swarm-deep). Which levels apply depends on the model. The swarm rungs run at max reasoning and turn on swarm orchestration (light fan-out or the deep task graph).\n\nAlso: {effort_keys} to cycle.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/fast",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle fast mode",
        detail: Some(
            "/fast\nShow whether fast mode is enabled, plus the saved default.\n\n/fast on\nEnable fast mode (service_tier = priority) for the current session.\n\n/fast off\nDisable fast mode for the current session.\n\n/fast status\nShow current fast-mode status.\n\n/fast default on\nSave fast mode as the default on startup.\n\n/fast default off\nSave fast mode as the default off on startup.\n\n/fast default status\nShow the saved fast-mode default.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/transport",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change connection transport",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/alignment",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/change default text alignment",
        detail: Some(
            "/alignment\nShow the current alignment and the saved default.\n\n/alignment centered\nSave centered alignment as the default and apply it immediately.\n\n/alignment left\nSave left-aligned mode as the default and apply it immediately.\n\nPress Alt+C anytime to toggle alignment just for the current session.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/compact-notifications",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle single-line swarm/file-activity notifications",
        detail: Some(
            "/compact-notifications\nShow whether swarm/file-activity notifications are compact.\n\n/compact-notifications on\nCollapse file-activity notifications to a single line (path · summary), dropping the intent and diff preview.\n\n/compact-notifications off\nRestore the full multi-line notification cards.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/show-agentgrep-output",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle full agentgrep search output inline in chat",
        detail: Some(
            "/show-agentgrep-output\nShow whether full agentgrep search output renders inline in the transcript.\n\n/show-agentgrep-output on\nRender the full agentgrep search results inline beneath each agentgrep call instead of just the one-line summary.\n\n/show-agentgrep-output off\nShow only the compact one-line agentgrep summary.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/tool-call-details",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/toggle dimmed technical details on tool rows with an intent",
        detail: Some(
            "/tool-call-details\nShow whether the dimmed technical detail (command, path, args) renders next to the model-provided intent on tool rows.\n\n/tool-call-details on\nShow the technical detail after the intent, e.g. `bash · Run tests · $ cargo test`.\n\n/tool-call-details off\nShow only the intent on tool rows that have one. Rows without an intent still show the technical detail, and error summaries always render.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/thinking-display",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show/hide the model's thinking text (off/full/current)",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/cancel",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Cancel the current prompt or operation",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/clear",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Clear conversation history",
        detail: Some(
            "/clear\nClear current conversation, queue, and display; starts a fresh session.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/cls",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Clear the view only, keeping context",
        detail: Some(
            "/cls\nClear the rendered view only. The model keeps its full context; nothing is sent or forgotten. (Ctrl+L clears the screen but keeps history in scrollback.)",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/rewind",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Rewind conversation to previous message",
        detail: Some(
            "/rewind\nShow numbered conversation history.\n\n/rewind N\nRewind to message N (drops everything after it and resets provider session).\n\n/rewind undo\nUndo the most recent rewind and restore the removed messages.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/poke",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Poke model to resume with incomplete todos",
        detail: Some(
            "/poke [on|off|status]\nPoke the model to resume when it has stopped with incomplete todos.\n\n\
                Auto-poke now starts enabled by default, and Ctrl+P toggles it on/off.\n\
                Set auto_poke = false under [features] in ~/.jcode/config.toml to start with it disabled.\n\
                /poke or /poke on arms auto-poke and immediately pokes if work remains.\n\
                /poke off disarms auto-poke and clears any queued poke follow-ups.\n\
                /poke status shows whether auto-poke is currently armed.\n\
                If a turn is currently running, the poke is queued and sent right after that turn finishes.\n\
                Injects a reminder with the number of incomplete todos and prompts the model to either\n\
                finish the work, update the todo list to reflect what is done, or ask for user input if genuinely blocked.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/plan",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Create a plan-only response as a plan card",
        detail: Some(
            "/plan [goal]\nDraft a plan without implementing anything. The model inspects the repo, then presents a structured plan (Goal, Scope, Approach, Validation, Open questions) as a dedicated plan card in the conversation.\n\nNothing is edited: it stops after presenting the plan. Once you approve, it converts the plan into a todo list and starts the work.\n\n/plan with no goal plans the task currently in focus.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/improve",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Autonomously improve the repository",
        detail: Some(
            "/improve [focus]\nStart an autonomous repo-improvement loop. The model inspects the project, writes a ranked todo list, implements the highest-leverage safe improvements, validates them, then keeps going until further work has diminishing returns.\n\n/improve plan [focus]\nGenerate a ranked improve todo list only, without editing files.\n\n/improve resume\nResume the last saved improve mode for this session using the current improve todos.\n\n/improve status\nShow the inferred status of the current improve run and todo batch.\n\n/improve stop\nAsk the model to stop after the next safe point, update todos, and summarize remaining work.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/refactor",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Run a safe refactor loop",
        detail: Some(
            "/refactor [focus]\nStart a refactor loop aimed at moving the repo toward a practical 10/10. The main agent inspects the project, writes a ranked refactor todo list, implements the best safe refactors itself, validates each batch, and asks one independent read-only subagent to review each meaningful batch before continuing.\n\n/refactor plan [focus]\nGenerate a ranked refactor todo list only, without editing files.\n\n/refactor resume\nResume the last saved refactor mode for this session using the current refactor todos.\n\n/refactor status\nShow the inferred status of the current refactor run and todo batch.\n\n/refactor stop\nAsk the model to stop after the next safe point, update todos, and summarize remaining work.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/compact",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Compact context",
        detail: Some(
            "/compact\nForce context compaction now.\nStarts background summarization and applies it automatically when ready.\n\n/compact mode\nShow current compaction mode for this session.\n\n/compact mode <reactive|proactive|semantic>\nChange compaction mode for this session.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/fix",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Recover when the model cannot continue",
        detail: Some(
            "/fix\nRun recovery actions when the model cannot continue.\nRepairs missing tool outputs, resets provider session state, and starts compaction when possible.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/memory",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle memory feature",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/test",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Verify a claim/current changes with layered tests",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/initiatives",
        aliases: &["/goals", "/goal", "/mission"],
        visibility: Visibility::Public,
        summary: "Open initiatives overview / resume tracked initiatives",
        detail: Some(
            "/goals\nOpen the goals overview in the side panel.\n\n/goals resume\nResume the most relevant active goal for this session/project.\n\n/goals show <id>\nOpen a specific goal in the side panel.\n\nAliases: /goals, /goal, /mission.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/swarm",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle swarm feature",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/overnight",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Run a supervised overnight coordinator",
        detail: Some(
            "/overnight <hours>[h|m] [mission]\nStart one overnight coordinator with a target wake/report time. The coordinator prioritizes verifiable, low-risk work, maintains structured logs, updates review notes, and generates a review HTML page.\n\n/overnight status\nShow the latest overnight run status.\n\n/overnight log\nShow recent overnight events.\n\n/overnight review\nOpen the generated review page.\n\n/overnight cancel\nRequest cancellation after the current coordinator turn reaches a safe boundary.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/context",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show the full session context snapshot",
        detail: Some(
            "/context\nShow the full session context snapshot: prompt/context composition, compaction state, model/provider/runtime details, queued work, todos, and side-panel state.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/skills",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show loaded skills and jcode-endorsed recommendations",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/version",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show current version",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/info",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show session info and tokens",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/usage",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show connected provider usage limits",
        detail: Some(
            "/usage\nFetch and display usage limits for connected providers. This command only reports real connected-provider usage windows and reset times.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/productivity",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Generate a shareable usage report + dashboard image",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/config",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show or edit configuration",
        detail: Some(
            "/config\nShow active configuration.\n\n/config init\nCreate default config file.\n\n/config edit\nOpen config in $EDITOR.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/log",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Mark the current location in the jcode logs",
        detail: Some(
            "/log mark [note]\nWrite a distinctive JCODE_LOG_MARK line to ~/.jcode/logs/jcode-YYYY-MM-DD.log with the current session, provider, model, working directory, and optional note. Use this to mark a spot for agents to inspect later.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/keys",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show keybinding conflicts with your terminal and OS (/keys refresh to rescan)",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/diff",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Cycle or set diff display mode (off/inline/full/pinned/file)",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/reload",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Reload into newest available binary",
        detail: Some(
            "/reload\nReload into the newest available binary if one is ready. This is fast and does not rebuild.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/restart",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Restart with current binary",
        detail: Some(
            "/restart\nRestart jcode with the current binary. Session is preserved.\nUseful after config changes, MCP server updates, or env var changes.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/rebuild",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Background rebuild and auto reload",
        detail: Some(
            "/rebuild\nRun git pull --ff-only, cargo build --release, and release tests in the background. jcode stays usable and reloads automatically when the build is ready.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/resume",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open session picker",
        detail: Some(
            "/resume\nOpen the interactive session picker. Browse and search all sessions, preview conversation history, and resume the highlighted session. By default, Enter resumes in the current terminal and Ctrl+Enter opens a new terminal; keybindings.session_picker_enter can swap those actions.{resume_shortcut}\n\nPress Esc to return to your current session.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/active",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Manage live sessions (working vs ready)",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/catchup",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open Catch Up picker",
        detail: Some(
            "/catchup\nOpen the Catch Up picker for finished sessions that need attention.\n\n/catchup next\nTeleport to the next session needing attention and open a Catch Up brief in the side panel.\n\n/catchup list\nAlias for opening the picker.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/back",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Return to the previous Catch Up session",
        detail: Some(
            "/back\nReturn to the previous session you came from via Catch Up.\n\nWorks after a /catchup next jump or after selecting a session from the Catch Up picker.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/save",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Bookmark session for easy access",
        detail: Some(
            "/save\nBookmark the current session so it appears at the top of /resume.\n\n/save <label>\nBookmark with a custom label for easy identification.\n\nSaved sessions are shown in a dedicated \"Saved\" section in the session picker.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/unsave",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Remove bookmark from session",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/rename",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Rename current session",
        detail: Some(
            "/rename <session name>\nSet a custom display title for the current session. This updates the window title and /resume display.\n\n/rename --clear\nClear the custom name and return to the generated session title.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/fork",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Fork session into a new window (optional prompt)",
        detail: Some(
            "/fork\nFork the current session into a new terminal pane or window. Clones the full conversation history so both sessions continue from the same point. Inside tmux, jcode automatically opens a right-side pane.\n\n/fork <prompt>\nFork the session and start the new pane/window by answering the prompt. The original session keeps working uninterrupted.\n\n/split\nAlias for /fork.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/transfer",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Compact context into a fresh handoff session",
        detail: Some(
            "/transfer\nCompact the current session into a summary-only handoff, copy the current todo list to a fresh session, and open that transferred session in a new window.\n\nIf a turn is currently running, jcode first soft-pauses the current session at the next safe point, then performs the transfer.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/workspace",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Niri-style session workspace",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/quit",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Exit jcode",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/auth",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show authentication status",
        detail: Some(
            "/auth\nShow authentication status for all providers.\n\n/login\nInteractive provider selection - pick a provider to log into.\n\n/login <provider>\nStart login flow directly for any provider shown by /login or the /login completions.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/login",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Login to a provider",
        detail: Some(
            "/auth\nShow authentication status for all providers.\n\n/login\nInteractive provider selection - pick a provider to log into.\n\n/login <provider>\nStart login flow directly for any provider shown by /login or the /login completions.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/logout",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Log out of a provider",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/account",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Open the combined account picker",
        detail: Some(
            "/account\nOpen the inline account picker showing both Claude and OpenAI accounts together. It lists saved accounts plus new/replace actions for each provider.\n\n/account claude  or  /account openai\nOpen the inline picker filtered to that provider.\n\n/account <provider> settings\nShow provider-specific account/settings details.\n\n/account <provider> login\nStart or refresh credentials for a provider.\n\n/account claude add  or  /account openai add\nCreate the next numbered OAuth account directly.\n\n/account <provider> switch <label>\nSwitch the active account for multi-account providers.\n\n/account <provider> remove <label>\nRemove a saved account.\n\n/account default-provider <provider|auto>\nSet the preferred default provider for future sessions.\n\n/account default-model <model|clear>\nSet the preferred default model for future sessions.\n\nOpenAI-specific settings:\n  /account openai transport ...\n  /account openai effort ...\n  /account openai fast on|off\n\nCustom provider settings:\n  /account openai-compatible api-base ...\n  /account openai-compatible api-key-name ...\n  /account openai-compatible env-file ...\n  /account openai-compatible default-model ...",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/cache",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Show cache stats or set cache TTL",
        detail: Some(
            "/cache stats\nShow KV cache stats for this session: cache read/write totals, hit ratios, current baseline, and recent miss attributions.\n\n/cache\nToggle Anthropic cache TTL between 5 minutes and 1 hour.\n\n/cache 1h  or  /cache 5m\nSet Anthropic cache TTL explicitly.",
        ),
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/debug-visual",
        aliases: &[],
        visibility: Visibility::Public,
        summary: "Toggle visual debug overlay",
        detail: None,
        detail_requires_remote: false,
    },
    CommandSpec {
        name: "/client-reload",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Force reload client binary",
        detail: Some("/client-reload\nForce client binary reload in remote mode."),
        detail_requires_remote: true,
    },
    CommandSpec {
        name: "/server-reload",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Force reload server binary",
        detail: Some("/server-reload\nForce server binary reload in remote mode."),
        detail_requires_remote: true,
    },
    CommandSpec {
        name: "/continue",
        aliases: &[],
        visibility: Visibility::Remote,
        summary: "Continue every interrupted live session that would auto-resume",
        detail: Some(
            "/continue\nContinue every interrupted live session that would auto-resume on a reload.\n\nThe server walks all currently-live sessions and, for each idle one that still owes the model a reply (a turn that errored or was interrupted mid-generation), injects the standard \"continue where you left off\" reminder so it picks back up. Sessions that are busy, fresh, or already complete are left untouched.\n\nAlias: /resumeall.",
        ),
        detail_requires_remote: true,
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

/// Entries offered in autocomplete and listed by `/help`, in registration order.
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
