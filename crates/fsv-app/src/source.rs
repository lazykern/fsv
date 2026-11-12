//! [`Api`] over a shared HTTP client, for the `fsv_views` page reads
//! (`views.optional_get`'s `c.int_get` / `c.v2_get`).

use std::sync::Arc;

use serde_json::{Map, Value};

use fsv_api::service::Api;
use fsv_core::client::{Body, Response};
use fsv_core::{Client, Result};

/// The page reads of `fsv_views` over a live [`Client`].
#[derive(Clone)]
pub struct ClientSource {
    client: Arc<Client>,
}

impl ClientSource {
    pub fn new(client: Arc<Client>) -> ClientSource {
        ClientSource { client }
    }
}

impl Api for ClientSource {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        Api::int_get(&*self.client, path, params)
    }
    fn int_post(&self, path: &str, body: &Value) -> Result<Value> {
        Api::int_post(&*self.client, path, body)
    }
    fn int_put(&self, path: &str, body: &Value) -> Result<Value> {
        Api::int_put(&*self.client, path, body)
    }
    fn int_patch(&self, path: &str, body: &Value) -> Result<Value> {
        Api::int_patch(&*self.client, path, body)
    }
    fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        Api::int_delete(&*self.client, path, body, params)
    }
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        Api::autocomplete(&*self.client, kind, query, params)
    }
    fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
        Api::lookup_choices(&*self.client, link, query)
    }
    fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> Result<Value> {
        Api::fulltext_search(&*self.client, entity, term, page, sort)
    }
    fn get_csrf(&self) -> Result<String> {
        Api::get_csrf(&*self.client)
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        body: Body,
        params: Option<Map<String, Value>>,
        csrf: &str,
    ) -> Result<Value> {
        Api::send_once(&*self.client, method, path, body, params, csrf)
    }
    fn fetch(&self, url: &str) -> Result<Response> {
        Api::fetch(&*self.client, url)
    }
    fn page(&self, path: &str) -> Result<Response> {
        Api::page(&*self.client, path)
    }
    fn domain(&self) -> String {
        Api::domain(&*self.client)
    }
    fn workspace_cookies(&self) -> Vec<String> {
        Api::workspace_cookies(&*self.client)
    }
    fn me(&self) -> Result<Value> {
        Api::me(&*self.client)
    }
    fn v2_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        Api::v2_get(&*self.client, path, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{ok, pairs, status, tenant};
    use fsv_core::Error;
    use fsv_views::FetchError;
    use serde_json::json;

    fn classified(got: Result<Value>) -> std::result::Result<Value, FetchError> {
        got.map_err(FetchError::from)
    }

    #[test]
    fn int_get_sends_path_and_params() {
        let t = tenant(|c| {
            (c.path == "/api/_/tickets/5/notes")
                .then(|| ok(json!({"notes": [1]})))
                .flatten()
        });
        let src = ClientSource::new(t.client.clone());
        let params = crate::testkit::obj(json!({"include": "user", "page": 1, "per_page": 30}));
        assert_eq!(
            src.int_get("tickets/5/notes", Some(params)).unwrap(),
            json!({"notes": [1]})
        );
        let calls = t.calls();
        assert_eq!(calls[0].method, "GET");
        assert_eq!(
            calls[0].query,
            pairs(&[("include", "user"), ("page", "1"), ("per_page", "30")])
        );
    }

    #[test]
    fn v2_get_uses_the_public_api() {
        let t = tenant(|c| {
            (c.path == "/api/v2/agents/me")
                .then(|| ok(json!({"agent": {"id": 1}})))
                .flatten()
        });
        let src = ClientSource::new(t.client.clone());
        assert_eq!(
            src.v2_get("agents/me", None).unwrap(),
            json!({"agent": {"id": 1}})
        );
        assert_eq!(t.lines(), ["GET /api/v2/agents/me"]);
    }

    #[test]
    fn http_errors_become_statuses_and_sessions_stay_sessions() {
        let t = tenant(|c| match c.path.as_str() {
            "/api/_/x" => status(403, json!({})),
            "/api/_/y" => status(401, json!({})),
            _ => None,
        });
        let src = ClientSource::new(t.client.clone());
        assert_eq!(
            classified(src.int_get("missing", None)),
            Err(FetchError::Status(Some(404)))
        );
        assert!(matches!(src.int_get("y", None), Err(Error::Session(_))));
        let miss = fsv_views::source::optional_get(&src, "missing", None, "tabs")
            .unwrap()
            .unwrap_err();
        assert_eq!(miss.note(), "tabs: not available on this account (404)");
        assert!(fsv_views::source::optional_get(&src, "y", None, "").is_err());
        // A 403 with a live session (the dashboard probe answers 200) is an API error.
        assert_eq!(
            classified(src.int_get("x", None)),
            Err(FetchError::Status(Some(403)))
        );
    }

    #[test]
    fn workspace_get_falls_back_to_the_plain_path() {
        let t = tenant(|c| {
            (c.path == "/api/_/change_management/4")
                .then(|| ok(json!({"ok": 1})))
                .flatten()
        });
        let src = ClientSource::new(t.client.clone());
        let got = fsv_views::source::workspace_get(
            &src,
            "change_management/4",
            None,
            Some(&json!(2)),
            "lifecycle",
        )
        .unwrap();
        assert_eq!(got.unwrap(), json!({"ok": 1}));
        assert_eq!(
            t.lines(),
            [
                "GET /api/_/ws/2/change_management/4",
                "GET /api/_/change_management/4"
            ]
        );
    }

    #[test]
    fn delegates_the_rest_of_the_client() {
        let t = tenant(|_| None);
        let src = ClientSource::new(t.client.clone());
        assert_eq!(Api::domain(&src), Api::domain(&*t.client));
        assert_eq!(
            Api::workspace_cookies(&src),
            Api::workspace_cookies(&*t.client)
        );
    }
}
