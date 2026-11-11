//! [`fsv_views::Source`] over the HTTP client (`views.optional_get`'s
//! `c.int_get` / `c.v2_get`).

use std::sync::Arc;

use serde_json::{Map, Value};

use fsv_core::{Client, Error};
use fsv_views::{FetchError, Source};

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

/// `optional_get`'s `except` ladder: a `SessionError` stays a session error
/// (exit 3), an `APIError` keeps its status, anything else has none.
pub fn fetch_error(e: Error) -> FetchError {
    match e {
        Error::Session(msg) => FetchError::Session(msg),
        Error::Api(api) => FetchError::Status(Some(i64::from(api.status))),
        _ => FetchError::Status(None),
    }
}

impl Source for ClientSource {
    fn int_get(
        &mut self,
        path: &str,
        params: Option<&Map<String, Value>>,
    ) -> Result<Value, FetchError> {
        self.client
            .int_get(path, params.cloned())
            .map_err(fetch_error)
    }

    fn v2_get(&mut self, path: &str) -> Result<Value, FetchError> {
        self.client.v2_get(path, None).map_err(fetch_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{ok, pairs, status, tenant};
    use fsv_core::ApiError;
    use serde_json::json;

    #[test]
    fn int_get_sends_path_and_params() {
        let t = tenant(|c| {
            (c.path == "/api/_/tickets/5/notes")
                .then(|| ok(json!({"notes": [1]})))
                .flatten()
        });
        let mut src = ClientSource::new(t.client.clone());
        let params = crate::testkit::obj(json!({"include": "user", "page": 1, "per_page": 30}));
        assert_eq!(
            src.int_get("tickets/5/notes", Some(&params)).unwrap(),
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
        let mut src = ClientSource::new(t.client.clone());
        assert_eq!(
            src.v2_get("agents/me").unwrap(),
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
        let mut src = ClientSource::new(t.client.clone());
        assert_eq!(
            src.int_get("missing", None),
            Err(FetchError::Status(Some(404)))
        );
        assert!(matches!(
            src.int_get("y", None),
            Err(FetchError::Session(_))
        ));
        let miss = fsv_views::source::optional_get(&mut src, "missing", None, "tabs")
            .unwrap()
            .unwrap_err();
        assert_eq!(miss.note(), "tabs: not available on this account (404)");
        assert!(fsv_views::source::optional_get(&mut src, "y", None, "").is_err());
        // A 403 with a live session (the dashboard probe answers 200) is an API error.
        assert_eq!(src.int_get("x", None), Err(FetchError::Status(Some(403))));
    }

    #[test]
    fn workspace_get_falls_back_to_the_plain_path() {
        let t = tenant(|c| {
            (c.path == "/api/_/change_management/4")
                .then(|| ok(json!({"ok": 1})))
                .flatten()
        });
        let mut src = ClientSource::new(t.client.clone());
        let got = fsv_views::source::workspace_get(
            &mut src,
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
    fn fetch_error_mapping() {
        assert_eq!(
            fetch_error(Error::session("gone")),
            FetchError::Session("gone".into())
        );
        assert_eq!(
            fetch_error(ApiError::new(0, "x").into()),
            FetchError::Status(Some(0))
        );
        assert_eq!(fetch_error(Error::other("boom")), FetchError::Status(None));
    }
}
