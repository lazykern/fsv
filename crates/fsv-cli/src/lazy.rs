//! An [`Api`] that opens the session client on first use, like Python
//! modules calling `get_client()` only when a lookup or send needs it.

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};

use fsv_api::service::Api;
use fsv_core::client::{Body, Response};
use fsv_core::{Client, Result};

#[derive(Default)]
pub struct LazyApi(OnceLock<std::result::Result<Arc<Client>, String>>);

impl LazyApi {
    pub fn get(&self) -> Result<Arc<Client>> {
        let r = self
            .0
            .get_or_init(|| crate::util::shared_client().map_err(|e| e.to_string()));
        match r {
            Ok(c) => Ok(c.clone()),
            Err(_) => crate::util::shared_client(),
        }
    }
}

impl Api for LazyApi {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        self.get()?.int_get(path, params)
    }
    fn int_post(&self, path: &str, body: &Value) -> Result<Value> {
        self.get()?.int_post(path, body)
    }
    fn int_put(&self, path: &str, body: &Value) -> Result<Value> {
        self.get()?.int_put(path, body)
    }
    fn int_patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.get()?.int_patch(path, body)
    }
    fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        self.get()?.int_delete(path, body, params)
    }
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        self.get()?.autocomplete(kind, query, params)
    }
    fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
        self.get()?.lookup_choices(link, query)
    }
    fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> Result<Value> {
        self.get()?.fulltext_search(entity, term, page, sort)
    }
    fn get_csrf(&self) -> Result<String> {
        self.get()?.get_csrf()
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        body: Body,
        params: Option<Map<String, Value>>,
        csrf: &str,
    ) -> Result<Value> {
        Api::send_once(self.get()?.as_ref(), method, path, body, params, csrf)
    }
    fn fetch(&self, url: &str) -> Result<Response> {
        Api::fetch(self.get()?.as_ref(), url)
    }
    fn domain(&self) -> String {
        fsv_core::config::domain()
    }
    fn workspace_cookies(&self) -> Vec<String> {
        self.get()
            .map(|c| Api::workspace_cookies(c.as_ref()))
            .unwrap_or_default()
    }
}
