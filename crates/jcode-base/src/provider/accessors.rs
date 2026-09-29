use super::*;

impl MultiProvider {
    pub(super) fn claude_provider(&self) -> Option<Arc<dyn Provider>> {
        self.claude
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn anthropic_provider(&self) -> Option<Arc<dyn Provider>> {
        self.anthropic
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn openai_provider(&self) -> Option<Arc<dyn Provider>> {
        self.openai
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn antigravity_provider(&self) -> Option<Arc<dyn Provider>> {
        self.antigravity
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn gemini_provider(&self) -> Option<Arc<dyn Provider>> {
        self.gemini
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn has_claude_runtime(&self) -> bool {
        self.anthropic_provider().is_some() || self.claude_provider().is_some()
    }

    pub(super) fn provider_slot_available(&self, provider: ActiveProvider) -> bool {
        match provider {
            ActiveProvider::Claude => self.has_claude_runtime(),
            ActiveProvider::OpenAI => self.openai_provider().is_some(),
            ActiveProvider::Antigravity => self.antigravity_provider().is_some(),
            ActiveProvider::Gemini => self.gemini_provider().is_some(),
        }
    }

    pub(super) fn reconcile_auth_if_provider_missing(&self, provider: ActiveProvider) -> bool {
        if self.provider_slot_available(provider) {
            return true;
        }

        crate::logging::info(&format!(
            "Provider {} missing at use site; reconciling auth from disk",
            Self::provider_label(provider)
        ));
        Provider::on_auth_changed(self);
        self.provider_slot_available(provider)
    }
}
