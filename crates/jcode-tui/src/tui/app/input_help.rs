use super::*;

impl App {
    /// `/help <command>` detail page, rendered from the command table.
    ///
    /// The text lives in [`super::command_spec::COMMANDS`] so a command cannot
    /// be registered with a detail page that nothing shows, or shown with a page
    /// nothing registered. Only the runtime substitutions happen here.
    pub(super) fn command_help(&self, topic: &str) -> Option<String> {
        let topic = topic.trim().trim_start_matches('/').to_lowercase();
        let spec = super::command_spec::spec_for(&format!("/{topic}"))?;
        let help = spec.detail?;
        if spec.detail_requires_remote && !self.is_remote {
            return None;
        }

        let help = help.replace(
            "{effort_keys}",
            &crate::tui::keybind::effort_switch_keys_label(),
        );
        let resume_shortcut = match crate::tui::keybind::load_open_resume_key().label {
            Some(label) => format!(" You can also press {label} to open it directly."),
            None => String::new(),
        };
        let help = help.replace("{resume_shortcut}", &resume_shortcut);
        // Mac keyboards have no "Alt" key; show the ⌥ keycap instead.
        let help = help.replace(
            "Alt+",
            &format!("{}+", jcode_tui_core::keybind::alt_label()),
        );
        Some(help)
    }
}
