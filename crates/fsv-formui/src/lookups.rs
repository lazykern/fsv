//! Lookups the web form runs while you type.
//!
//! * Requesters: `GET /search/autocomplete/requesters?q=<text>&qf=details`
//!   (`results[{id, value, details: "Name <email>", email, ...}]`), the same
//!   call the web's requester box makes.
//! * Groups and agents: the web filters `GET /api/_/bootstrap/agents_groups`
//!   locally; so does fsv (fetched once per session). When the role may not
//!   read it (403), agents fall back to the agent autocomplete.
//! * Other lookup fields: the engine resolver (`lookup_choices` links).
//!
//! A 403/404 becomes [`LookupUnavailable`]; the form shows a dim note
//! instead of failing.

use std::fmt;
use std::sync::{Arc, Mutex};

use fsv_forms::{Field, LookupValue, fuzzy_filter};
use serde_json::{Map, Value as Json, json};

/// A failed Freshservice call, as the form needs it: the HTTP status (when
/// there was a response) and the message to show.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiFailure {
    pub status: Option<u16>,
    pub message: String,
}

impl ApiFailure {
    pub fn new(status: Option<u16>, message: impl Into<String>) -> Self {
        ApiFailure {
            status,
            message: message.into(),
        }
    }

    fn denied(&self) -> bool {
        matches!(self.status, Some(403 | 404))
    }
}

impl fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiFailure {}

impl From<fsv_core::Error> for ApiFailure {
    fn from(e: fsv_core::Error) -> Self {
        ApiFailure::new(e.status(), e.to_string())
    }
}

/// The client calls the form makes while it runs (worker threads call
/// them, hence `Send + Sync`).
pub trait FormApi: Send + Sync {
    /// `GET /search/autocomplete/<kind>?q=<query>&...`: the `results` rows.
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Map<String, Json>,
    ) -> Result<Vec<Json>, ApiFailure>;
    /// `GET /api/_/<path>`.
    fn int_get(&self, path: &str) -> Result<Json, ApiFailure>;
    /// The signed-in agent (`check_session()`), for the profile time zone.
    fn check_session(&self) -> Result<Json, ApiFailure>;
}

impl FormApi for fsv_core::Client {
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Map<String, Json>,
    ) -> Result<Vec<Json>, ApiFailure> {
        Ok(fsv_core::Client::autocomplete(
            self,
            kind,
            query,
            Some(params),
        )?)
    }

    fn int_get(&self, path: &str) -> Result<Json, ApiFailure> {
        Ok(fsv_core::Client::int_get(self, path, None)?)
    }

    fn check_session(&self) -> Result<Json, ApiFailure> {
        Ok(fsv_core::Client::check_session(self)?)
    }
}

/// People / group / other lookups by text (the engine's resolver, shared
/// with worker threads).
pub trait Resolver: Send + Sync {
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        field: &Field,
    ) -> Result<Vec<LookupValue>, ApiFailure>;
}

/// Adapts a [`Resolver`] to the engine's [`fsv_forms::LookupResolver`].
pub struct EngineResolver(pub Arc<dyn Resolver>);

impl fsv_forms::LookupResolver for EngineResolver {
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        field: &Field,
    ) -> Result<Vec<LookupValue>, fsv_forms::state::ResolverError> {
        self.0
            .lookup(entity, query, field)
            .map_err(|e| Box::new(e) as fsv_forms::state::ResolverError)
    }
}

/// The tenant or the agent's role does not offer this lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct LookupUnavailable {
    pub what: String,
    pub status: Option<u16>,
}

impl LookupUnavailable {
    pub fn new(what: impl Into<String>, status: Option<u16>) -> Self {
        LookupUnavailable {
            what: what.into(),
            status,
        }
    }
}

impl fmt::Display for LookupUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(s) => write!(f, "{} is not available for your role (HTTP {s})", self.what),
            None => write!(
                f,
                "{} is not available for your role (not offered)",
                self.what
            ),
        }
    }
}

/// Why a lookup gave no rows.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupError {
    Unavailable(LookupUnavailable),
    Api(ApiFailure),
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LookupError::Unavailable(u) => u.fmt(f),
            LookupError::Api(a) => a.fmt(f),
        }
    }
}

fn unavailable_or(e: ApiFailure, what: impl Into<String>) -> LookupError {
    if e.denied() {
        LookupError::Unavailable(LookupUnavailable::new(what, e.status))
    } else {
        LookupError::Api(e)
    }
}

fn email_of(row: &Map<String, Json>) -> Option<String> {
    for k in ["email", "primary_email"] {
        if let Some(Json::String(v)) = row.get(k)
            && v.contains('@')
        {
            return Some(v.clone());
        }
    }
    if let Some(Json::String(d)) = row.get("details") {
        if d.contains('<') && d.trim_end().ends_with('>') {
            let tail = d.rsplit_once('<').map_or("", |(_, t)| t);
            let email = tail.trim_end_matches(['>', ' ']).trim();
            return (!email.is_empty()).then(|| email.to_string());
        }
        if d.contains('@') {
            return Some(d.trim().to_string());
        }
    }
    None
}

/// A person row (`value`/`name`/`details`) as a lookup value.
pub fn person(row: &Map<String, Json>, id_key: &str) -> LookupValue {
    let id = row.get(id_key).cloned().unwrap_or(Json::Null);
    let mut name = row
        .get("value")
        .filter(|v| fsv_py::truthy(v))
        .or_else(|| row.get("name").filter(|v| fsv_py::truthy(v)))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    if name.is_empty()
        && let Some(Json::String(d)) = row.get("details")
    {
        name = d.split('<').next().unwrap_or("").trim().to_string();
    }
    if name.is_empty() && fsv_py::truthy(&id) {
        name = fsv_py::py_str(&id);
    }
    let email = email_of(row);
    LookupValue::new(id, name, email.as_deref())
}

/// People matching `query` (name or email), as the web's requester box.
pub fn search_requesters(
    api: &dyn FormApi,
    query: &str,
    limit: usize,
) -> Result<Vec<LookupValue>, LookupError> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let mut params = Map::new();
    params.insert("qf".into(), json!("details"));
    let rows = api
        .autocomplete("requesters", q, params)
        .map_err(|e| unavailable_or(e, "requester search"))?;
    Ok(rows
        .iter()
        .filter_map(Json::as_object)
        .filter(|r| r.get("id").is_some_and(|v| !v.is_null()))
        .filter(|r| !r.get("deleted").is_some_and(fsv_py::truthy))
        .map(|r| person(r, "id"))
        .take(limit)
        .collect())
}

fn list_of<'a>(m: &'a Map<String, Json>, key: &str) -> &'a [Json] {
    match m.get(key) {
        Some(Json::Array(a)) => a,
        _ => &[],
    }
}

enum Boot {
    Unloaded,
    Loaded(Map<String, Json>),
    Denied(LookupUnavailable),
}

/// The `bootstrap/agents_groups` lists, fetched once, filtered locally.
/// Shared by worker threads; a 403/404 is remembered and not retried.
pub struct AgentsGroups {
    api: Arc<dyn FormApi>,
    data: Mutex<Boot>,
}

impl AgentsGroups {
    pub fn new(api: Arc<dyn FormApi>) -> Self {
        AgentsGroups {
            api,
            data: Mutex::new(Boot::Unloaded),
        }
    }

    fn load(&self) -> Result<Map<String, Json>, LookupError> {
        let mut guard = self.data.lock().unwrap_or_else(|p| p.into_inner());
        match &*guard {
            Boot::Loaded(m) => return Ok(m.clone()),
            Boot::Denied(u) => return Err(LookupError::Unavailable(u.clone())),
            Boot::Unloaded => {}
        }
        match self.api.int_get("bootstrap/agents_groups") {
            Ok(data) => {
                let m = data.as_object().cloned().unwrap_or_default();
                *guard = Boot::Loaded(m.clone());
                Ok(m)
            }
            Err(e) if e.denied() => {
                let u = LookupUnavailable::new("the agents and groups list", e.status);
                *guard = Boot::Denied(u.clone());
                Err(LookupError::Unavailable(u))
            }
            Err(e) => Err(LookupError::Api(e)),
        }
    }

    /// Whether the bootstrap answer carries a `key` list at all.
    pub fn has(&self, key: &str) -> Result<bool, LookupError> {
        Ok(matches!(self.load()?.get(key), Some(Json::Array(_))))
    }

    pub fn groups(&self) -> Result<Vec<LookupValue>, LookupError> {
        let data = self.load()?;
        Ok(list_of(&data, "groups")
            .iter()
            .filter_map(Json::as_object)
            .filter(|g| g.get("id").is_some_and(|v| !v.is_null()))
            .map(|g| {
                let id = g.get("id").cloned().unwrap_or(Json::Null);
                let name = g
                    .get("name")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| g.get("value").filter(|v| fsv_py::truthy(v)))
                    .unwrap_or(&id);
                LookupValue::new(id.clone(), fsv_py::py_str(name), None)
            })
            .collect())
    }

    pub fn agents(&self) -> Result<Vec<LookupValue>, LookupError> {
        let data = self.load()?;
        let mut out = vec![];
        for a in list_of(&data, "agents").iter().filter_map(Json::as_object) {
            let shown = a
                .get("value")
                .filter(|v| fsv_py::truthy(v))
                .or_else(|| a.get("name").filter(|v| fsv_py::truthy(v)))
                .map(fsv_py::py_str)
                .unwrap_or_default();
            if shown.contains("(Deactivated)") {
                continue;
            }
            let key = if a.get("user_id").is_some_and(|v| !v.is_null()) {
                "user_id"
            } else {
                "id"
            };
            if a.get(key).is_some_and(|v| !v.is_null()) {
                out.push(person(a, key));
            }
        }
        Ok(out)
    }

    pub fn filter(
        &self,
        entity: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LookupValue>, LookupError> {
        let pool = if entity == "group" {
            self.groups()?
        } else {
            self.agents()?
        };
        let items: Vec<(String, String)> = pool
            .iter()
            .map(|p| (p.display.clone(), p.email.clone().unwrap_or_default()))
            .collect();
        Ok(fuzzy_filter(query, &items)
            .into_iter()
            .take(limit)
            .map(|i| pool[i].clone())
            .collect())
    }
}

/// Suggestions for a lookup field.
pub fn search_field(
    api: &dyn FormApi,
    resolver: &dyn Resolver,
    boot: &AgentsGroups,
    field: &Field,
    entity: &str,
    query: &str,
) -> Result<Vec<LookupValue>, LookupError> {
    if entity == "user" {
        return search_requesters(api, query, 20);
    }
    if entity == "group" || entity == "agent" {
        let local = if entity == "group" {
            Some(boot.filter(entity, query, 50))
        } else {
            match boot.has("agents") {
                Ok(true) => Some(boot.filter(entity, query, 50)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            }
        };
        match local {
            Some(Ok(rows)) => return Ok(rows),
            Some(Err(LookupError::Unavailable(u))) if entity == "group" => {
                return Err(LookupError::Unavailable(u));
            }
            Some(Err(LookupError::Api(e))) => return Err(LookupError::Api(e)),
            _ => {}
        }
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        return resolver
            .lookup("agent", query, field)
            .map_err(|e| unavailable_or(e, "agent search"));
    }
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    resolver
        .lookup(entity, query, field)
        .map_err(|e| unavailable_or(e, format!("{} search", field.label)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Api {
        boot: Option<Result<Json, ApiFailure>>,
        people: Vec<Json>,
        denied: bool,
        boot_calls: AtomicUsize,
    }

    impl FormApi for Api {
        fn autocomplete(
            &self,
            _kind: &str,
            query: &str,
            params: Map<String, Json>,
        ) -> Result<Vec<Json>, ApiFailure> {
            assert_eq!(params.get("qf"), Some(&json!("details")));
            if self.denied {
                return Err(ApiFailure::new(Some(403), "forbidden"));
            }
            let q = query.to_lowercase();
            Ok(self
                .people
                .iter()
                .filter(|p| {
                    p["value"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
                })
                .cloned()
                .collect())
        }
        fn int_get(&self, path: &str) -> Result<Json, ApiFailure> {
            assert_eq!(path, "bootstrap/agents_groups");
            self.boot_calls.fetch_add(1, Ordering::SeqCst);
            self.boot.clone().unwrap_or(Ok(json!({})))
        }
        fn check_session(&self) -> Result<Json, ApiFailure> {
            Ok(json!({}))
        }
    }

    struct Res(Result<Vec<LookupValue>, ApiFailure>);
    impl Resolver for Res {
        fn lookup(&self, _: &str, _: &str, _: &Field) -> Result<Vec<LookupValue>, ApiFailure> {
            self.0.clone()
        }
    }

    fn row(v: Json) -> Map<String, Json> {
        v.as_object().cloned().expect("object")
    }

    #[test]
    fn person_reads_name_and_email_shapes() {
        let p = person(
            &row(json!({"id": 1, "details": "Jane Doe <jane@example.com>"})),
            "id",
        );
        assert_eq!(
            (p.display.as_str(), p.email.as_deref()),
            ("Jane Doe", Some("jane@example.com"))
        );
        let p = person(
            &row(json!({"id": 2, "value": "Bo", "details": "bo@example.com"})),
            "id",
        );
        assert_eq!(p.email.as_deref(), Some("bo@example.com"));
        let p = person(
            &row(json!({"id": 3, "name": "Cy", "primary_email": "cy@example.com"})),
            "id",
        );
        assert_eq!(
            (p.display.as_str(), p.email.as_deref()),
            ("Cy", Some("cy@example.com"))
        );
        assert_eq!(person(&row(json!({"id": 4})), "id").display, "4");
    }

    #[test]
    fn requesters_skip_deleted_and_blank_queries() {
        let api = Api {
            people: vec![
                json!({"id": 1, "value": "Jane Doe", "email": "jane@example.com"}),
                json!({"id": 2, "value": "Jane Gone", "email": "gone@example.com", "deleted": true}),
                json!({"value": "Jane No Id"}),
            ],
            ..Default::default()
        };
        let got = search_requesters(&api, "jane", 20).expect("rows");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].display, "Jane Doe");
        assert!(search_requesters(&api, "  ", 20).expect("rows").is_empty());
        let denied = Api {
            denied: true,
            ..Default::default()
        };
        let e = search_requesters(&denied, "x", 20).expect_err("denied");
        assert_eq!(
            e.to_string(),
            "requester search is not available for your role (HTTP 403)"
        );
    }

    #[test]
    fn bootstrap_is_fetched_once_and_filtered_locally() {
        let api = Arc::new(Api {
            boot: Some(Ok(json!({
                "groups": [{"id": 9, "name": "Platform Ops"}, {"id": 10, "name": "Billing"}],
                "agents": [{"id": 1, "user_id": 71, "value": "Alex Kim", "email": "alex@example.com"},
                           {"id": 2, "value": "Old One (Deactivated)"}],
            }))),
            ..Default::default()
        });
        let boot = AgentsGroups::new(api.clone());
        let res = Res(Ok(vec![]));
        let f = Field::new("group", "Group", fsv_forms::Kind::LookupGroup);
        let g = search_field(api.as_ref(), &res, &boot, &f, "group", "plat").expect("rows");
        assert_eq!(g[0].display, "Platform Ops");
        let a = search_field(api.as_ref(), &res, &boot, &f, "agent", "").expect("rows");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, json!(71));
        assert_eq!(api.boot_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn denied_bootstrap_falls_back_for_agents_only() {
        let api = Arc::new(Api {
            boot: Some(Err(ApiFailure::new(Some(403), "forbidden"))),
            ..Default::default()
        });
        let boot = AgentsGroups::new(api.clone());
        let f = Field::new("agent", "Agent", fsv_forms::Kind::LookupAgent);
        let ok = Res(Ok(vec![LookupValue::new(5, "Alex", None)]));
        let e = search_field(api.as_ref(), &ok, &boot, &f, "group", "x").expect_err("denied");
        assert_eq!(
            e.to_string(),
            "the agents and groups list is not available for your role (HTTP 403)"
        );
        let rows = search_field(api.as_ref(), &ok, &boot, &f, "agent", "al").expect("fallback");
        assert_eq!(rows[0].display, "Alex");
        assert!(
            search_field(api.as_ref(), &ok, &boot, &f, "agent", " ")
                .expect("blank")
                .is_empty()
        );
        let denied = Res(Err(ApiFailure::new(Some(404), "nope")));
        let e = search_field(api.as_ref(), &denied, &boot, &f, "agent", "al").expect_err("denied");
        assert_eq!(
            e.to_string(),
            "agent search is not available for your role (HTTP 404)"
        );
        assert_eq!(api.boot_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_lookups_use_the_resolver() {
        let api = Api::default();
        let boot = AgentsGroups::new(Arc::new(Api::default()));
        let f = Field::new("reviewer", "Reviewer", fsv_forms::Kind::Lookup);
        let denied = Res(Err(ApiFailure::new(Some(403), "x")));
        let e = search_field(&api, &denied, &boot, &f, "lookup", "jo").expect_err("denied");
        assert_eq!(
            e.to_string(),
            "Reviewer search is not available for your role (HTTP 403)"
        );
        let broken = Res(Err(ApiFailure::new(Some(500), "server error")));
        assert_eq!(
            search_field(&api, &broken, &boot, &f, "lookup", "jo").expect_err("500"),
            LookupError::Api(ApiFailure::new(Some(500), "server error"))
        );
    }
}
