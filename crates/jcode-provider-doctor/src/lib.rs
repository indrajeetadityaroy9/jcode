//! Provider-doctor diagnostics for jcode.
//!
//! Sits downstream of `jcode-base` so edits to the doctor cluster do not
//! rebuild the base -> app-core -> tui spine:
//! - [`provider_e2e`]: the strict end-to-end runner behind `jcode
//!   provider-doctor` (offline/catalog/full tiers, native-runtime drivers).
//! - [`live_provider_probes`]: the native-runtime probes the doctor drives
//!   (chat, streaming, tool-call, reasoning smokes).

pub mod live_provider_probes;
pub mod provider_e2e;

pub use provider_e2e::{
    DoctorCheck, DoctorReport, DoctorSpend, DoctorTier, NativeProviderKind,
    native_doctor_supports_provider, run_antigravity_native_e2e, run_claude_native_e2e,
    run_generic_native_e2e,
};
