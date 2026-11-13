//! [`MetaSource`] over `fsv_api::formmeta::load_meta` (fetch + cache of the
//! form fields and business rules).

use std::sync::Arc;

use serde_json::Value;

use fsv_api::formmeta::{self, DiskStore, MetaStore};
use fsv_api::formmeta::{FormMeta, MetaSource};
use fsv_api::service::Api;
use fsv_core::Result;

/// Form metadata from the server, cached in a [`MetaStore`].
#[derive(Clone)]
pub struct LiveMeta {
    pub api: Arc<dyn Api + Send + Sync>,
    pub store: Arc<dyn MetaStore + Send + Sync>,
}

impl LiveMeta {
    pub fn new(
        api: Arc<dyn Api + Send + Sync>,
        store: Arc<dyn MetaStore + Send + Sync>,
    ) -> LiveMeta {
        LiveMeta { api, store }
    }

    /// The current profile's cache directory and config.
    pub fn disk(api: Arc<dyn Api + Send + Sync>) -> LiveMeta {
        LiveMeta::new(api, Arc::new(DiskStore::current()))
    }
}

impl MetaSource for LiveMeta {
    fn load_meta(
        &self,
        module: &str,
        mode: &str,
        fresh: bool,
        module_id: &Value,
    ) -> Result<FormMeta> {
        self.load_meta_in(module, mode, fresh, module_id, &Value::Null)
    }

    fn load_meta_in(
        &self,
        module: &str,
        mode: &str,
        fresh: bool,
        module_id: &Value,
        record_workspace: &Value,
    ) -> Result<FormMeta> {
        formmeta::load_meta_in(
            self.api.as_ref(),
            self.store.as_ref(),
            module,
            mode,
            fresh,
            module_id,
            record_workspace,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{ok, pairs, tenant};
    use fsv_api::formmeta::MemStore;
    use serde_json::json;

    fn fields() -> Value {
        json!({"change_fields": [{"id": 1, "name": "subject", "label": "Subject", "field_type": "default_subject",
            "required": true, "position": 1, "default_field": true, "choices": [], "nested_fields": [], "workspace_id": 2}]})
    }

    fn rules_query(form_type: &str, module_id: &str) -> Vec<(String, String)> {
        pairs(&[
            ("ember_api", "true"),
            ("module_type", "change"),
            ("item_id", "null"),
            ("requester", ""),
            ("requested_for", ""),
            ("include_workspace", "false"),
            ("filter_by_default_form_action", "true"),
            ("form_type", form_type),
            ("user_type", "null"),
            ("module_id", module_id),
            ("cf_name_prefix", "customFields."),
        ])
    }

    /// Requests and result from `formmeta.load_meta(c, "changes", "new", fresh=True)` in Python.
    #[test]
    fn load_meta_fetches_fields_then_workspace_rules() {
        let t = tenant(|c| match c.path.as_str() {
            "/api/_/change_form_fields" => ok(fields()),
            "/api/_/ws/2/business_rules" => {
                ok(json!({"business_rules": [], "user": {"time_zone": "Tokyo"}}))
            }
            _ => None,
        });
        let live = LiveMeta::new(t.client.clone(), Arc::new(MemStore::default()));
        let m = live
            .load_meta("changes", "new", true, &Value::Null)
            .unwrap();
        let calls = t.calls();
        assert_eq!(
            calls.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["/api/_/change_form_fields", "/api/_/ws/2/business_rules"]
        );
        assert_eq!(calls[1].query, rules_query("new_form", ""));
        assert_eq!(
            (m.form_type.as_str(), m.source.as_str()),
            ("new_form", "server")
        );
        assert_eq!(m.workspace_id, json!(2));
        assert_eq!(
            m.rules,
            json!({"business_rules": [], "user": {"time_zone": "Tokyo"}, "workspace": 2})
        );
        assert_eq!(m.spec().get("subject").unwrap().label, "Subject");
    }

    #[test]
    fn edit_meta_sends_the_module_id_and_caches() {
        let t = tenant(|c| match c.path.as_str() {
            "/api/_/change_form_fields" => ok(fields()),
            "/api/_/ws/2/business_rules" => ok(json!({"business_rules": []})),
            _ => None,
        });
        let live = LiveMeta::new(t.client.clone(), Arc::new(MemStore::default()));
        let m = live.load_meta("change", "edit", false, &json!(77)).unwrap();
        assert_eq!(
            (m.form_type.as_str(), m.module_id.clone()),
            ("edit_form", json!(77))
        );
        assert_eq!(t.calls()[1].query, rules_query("edit_form", "77"));
        // Fields come from the cache; empty rules were never cached.
        live.load_meta("change", "edit", false, &json!(77)).unwrap();
        assert_eq!(
            t.calls()
                .iter()
                .skip(2)
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>(),
            ["/api/_/ws/2/business_rules"]
        );
    }

    #[test]
    fn disk_store_uses_the_profile() {
        let t = tenant(|_| None);
        let live = LiveMeta::disk(t.client.clone());
        assert!(
            live.store
                .cached("no-such-entry-for-fsv-app-tests")
                .is_none()
        );
    }
}
