//! `forms.plan.plan_hash` with the tenant binding Python adds on top
//! (`tenant.bind_plan`): fsv-forms stays tenant-free and independent of
//! fsv-core, so form plans hash through this wrapper.

use fsv_forms::plan::PlanFiles;
use serde_json::Value;

/// sha256 over the canonical payload and each file's name, size and sha256,
/// bound to the configured tenant. Plan hashes nested in `payload` are
/// replaced by their tenant-free form first, so composed hashes bind the
/// tenant exactly once.
pub fn plan_hash(payload: &Value, files: Option<&PlanFiles>) -> std::io::Result<String> {
    #[cfg(test)]
    crate::service::testing::hermetic();
    let raw = fsv_forms::plan_hash(&fsv_core::tenant::unbind(payload), files)?;
    Ok(fsv_core::tenant::bind_plan(&raw))
}
