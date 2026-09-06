use super::*;

impl App {
    fn client_maintenance_busy_message(
        current: crate::bus::ClientMaintenanceAction,
        requested: crate::bus::ClientMaintenanceAction,
    ) -> String {
        if current == requested {
            format!("{} already running in the background.", current.title())
        } else {
            format!(
                "{} already running in the background. Wait for it to finish before starting {}.",
                current.title(),
                requested.noun()
            )
        }
    }

    fn client_maintenance_card_title(action: crate::bus::ClientMaintenanceAction) -> String {
        action.title().to_string()
    }

    fn client_maintenance_card_message(
        action: crate::bus::ClientMaintenanceAction,
        status: impl Into<String>,
        note: impl Into<String>,
    ) -> String {
        let note = note.into();
        let mut content = format!("Status: {}", status.into());
        if !note.is_empty() {
            content.push_str("\n\n");
            content.push_str(&note);
        }
        if action == crate::bus::ClientMaintenanceAction::Rebuild {
            content.push_str(
                "\n\nPipeline: git pull --ff-only → cargo build --release → cargo test --release -- --test-threads=1",
            );
        }
        content
    }

    fn set_client_maintenance_message(
        &mut self,
        action: crate::bus::ClientMaintenanceAction,
        content: String,
    ) {
        let title = Self::client_maintenance_card_title(action);
        if let Some(idx) = self
            .display_messages
            .iter()
            .rposition(|message| Self::is_client_maintenance_message(message, &title))
        {
            let message = &mut self.display_messages[idx];
            let title_changed = message.title.as_deref() != Some(title.as_str());
            if title_changed {
                message.title = Some(title);
            }
            if message.content != content || title_changed {
                message.content = content;
                self.bump_display_messages_version();
            }
        } else {
            self.push_display_message(DisplayMessage::system(content).with_title(title));
        }
    }

    fn remove_client_maintenance_message(
        &mut self,
        action: crate::bus::ClientMaintenanceAction,
    ) -> bool {
        let title = Self::client_maintenance_card_title(action);
        let Some(idx) = self
            .display_messages
            .iter()
            .rposition(|message| Self::is_client_maintenance_message(message, &title))
        else {
            return false;
        };
        self.display_messages.remove(idx);
        self.bump_display_messages_version();
        true
    }

    pub(super) fn start_background_client_rebuild(&mut self, session_id: String) {
        self.start_background_client_maintenance(
            crate::bus::ClientMaintenanceAction::Rebuild,
            session_id,
        );
    }

    fn start_background_client_maintenance(
        &mut self,
        action: crate::bus::ClientMaintenanceAction,
        session_id: String,
    ) {
        if let Some(current) = self.background_client_action {
            let message = Self::client_maintenance_busy_message(current, action);
            self.set_status_notice(&message);
            self.set_client_maintenance_message(
                current,
                Self::client_maintenance_card_message(current, "already running", message),
            );
            return;
        }

        self.background_client_action = Some(action);
        self.pending_background_client_reload = None;

        self.set_status_notice("Starting background rebuild...");
        self.set_client_maintenance_message(
            action,
            Self::client_maintenance_card_message(
                action,
                "starting background rebuild",
                "Running in the background. jcode will reload automatically after the rebuild succeeds.",
            ),
        );
        crate::session_rebuild::spawn_background_session_rebuild(session_id);
    }

    pub(super) fn maybe_finish_background_client_reload(&mut self) -> bool {
        if self.is_processing {
            return false;
        }

        // The rebuild runs entirely in the background. Once the replacement
        // binary is ready, do not take the terminal away while the user is
        // typing. The tick loop retries as soon as this small quiet window ends.
        const RELOAD_TYPING_QUIET_WINDOW: std::time::Duration =
            std::time::Duration::from_millis(1200);
        if self
            .last_user_interaction
            .is_some_and(|activity| activity.elapsed() < RELOAD_TYPING_QUIET_WINDOW)
        {
            return false;
        }

        let Some((session_id, action)) = self.pending_background_client_reload.take() else {
            return false;
        };

        self.set_client_maintenance_message(
            action,
            Self::client_maintenance_card_message(
                action,
                "reloading client",
                "The new binary is ready, so jcode is switching over now.",
            ),
        );
        self.save_input_for_reload(&session_id);
        #[cfg(not(test))]
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            // Survives exec and lets the replacement measure the complete period
            // from the old client's final interactive state to its first frame.
            crate::env::set_var("JCODE_RELOAD_GAP_STARTED_MS", now.as_millis().to_string());
        }
        self.reload_requested = Some(session_id);
        self.should_quit = true;
        true
    }

    pub(super) fn handle_session_update_status(&mut self, status: crate::bus::SessionUpdateStatus) {
        use crate::bus::SessionUpdateStatus;

        let Some(active_session_id) = self.active_client_session_id().map(str::to_string) else {
            return;
        };

        match status {
            SessionUpdateStatus::Status {
                session_id,
                action,
                message,
            } => {
                if session_id != active_session_id {
                    return;
                }
                self.background_client_action = Some(action);
                self.set_status_notice(message.clone());
                self.set_client_maintenance_message(
                    action,
                    Self::client_maintenance_card_message(
                        action,
                        message,
                        "Still running in the background. jcode will reload automatically when ready.",
                    ),
                );
            }
            SessionUpdateStatus::ReadyToReload {
                session_id,
                action,
                version,
            } => {
                if session_id != active_session_id {
                    return;
                }
                self.background_client_action = None;
                let ready_message = format!("✅ Rebuild finished ({}).", version);
                if self.is_processing {
                    self.pending_background_client_reload = Some((session_id, action));
                    self.set_status_notice(format!(
                        "{} ready - will reload after the current turn",
                        action.title()
                    ));
                    self.set_client_maintenance_message(
                        action,
                        Self::client_maintenance_card_message(
                            action,
                            ready_message,
                            "Waiting for the current turn to finish before reloading.",
                        ),
                    );
                    return;
                }

                self.set_client_maintenance_message(
                    action,
                    Self::client_maintenance_card_message(action, ready_message, "Reloading now."),
                );
                self.pending_background_client_reload = Some((session_id, action));
                if !self.maybe_finish_background_client_reload() {
                    self.set_status_notice(format!("↑ {} ready · reloads when idle", version));
                    self.remove_client_maintenance_message(action);
                }
            }
            SessionUpdateStatus::Error {
                session_id,
                action,
                message,
            } => {
                if session_id != active_session_id {
                    return;
                }
                self.background_client_action = None;
                self.pending_background_client_reload = None;
                // One line only: the full error is already in the log.
                let reason = crate::session_rebuild::summarize_rebuild_error(&message);
                self.set_status_notice(format!("{} failed: {reason}", action.title()));
                self.set_client_maintenance_message(
                    action,
                    Self::client_maintenance_card_message(action, format!("failed ({reason})"), ""),
                );
            }
        }
    }
}
