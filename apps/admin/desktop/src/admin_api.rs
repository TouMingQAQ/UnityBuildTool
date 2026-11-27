//! In-process adapter used by the native administration window.
use crate::backend::Backend;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct AdminError {
    pub status: u16,
    pub message: String,
}

impl AdminError {
    fn from_fault((status, message): (u16, String)) -> Self {
        Self { status, message }
    }
}

#[derive(Clone)]
pub struct AdminApi {
    backend: Arc<Backend>,
}

impl AdminApi {
    pub fn new(backend: Arc<Backend>) -> Self {
        Self { backend }
    }

    pub fn call(
        &self,
        project_id: Option<&str>,
        method: &str,
        path: &str,
        body: Value,
    ) -> Result<Value, AdminError> {
        let result = match project_id {
            Some(id) => self.backend.handle_admin_project(method, path, body, id),
            None => self.backend.handle(method, path, body),
        };
        result.map_err(AdminError::from_fault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routes_project_and_root_config_through_in_process_api() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Backend::new(temp.path().join("config.json"), temp.path().into());
        let api = AdminApi::new(backend);
        api.call(
            None,
            "POST",
            "/api/config",
            json!({"schemaVersion":2,"projects":[{"id":"alpha","slug":"alpha","name":"Alpha","enabled":true}]}),
        )
        .unwrap();
        api.call(
            Some("alpha"),
            "POST",
            "/api/config",
            json!({"projectPath":"game"}),
        )
        .unwrap();
        let status = api
            .call(Some("alpha"), "GET", "/api/status", json!({}))
            .unwrap();
        assert_eq!(status["config"]["projectPath"], "game");
        let error = api
            .call(Some("missing"), "GET", "/api/status", json!({}))
            .unwrap_err();
        assert_eq!(error.status, 404);
    }

    #[test]
    fn preserves_unknown_config_fields_and_project_identity() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Backend::new(temp.path().join("config.json"), temp.path().into());
        let api = AdminApi::new(backend);
        api.call(
            None,
            "POST",
            "/api/config",
            json!({
                "schemaVersion":2,
                "futureRoot":{"enabled":true},
                "server":{"host":"127.0.0.1","port":50345},
                "projects":[{
                    "id":"alpha","slug":"alpha","name":"Alpha","enabled":true,
                    "lastVersionCode":7,"projectPath":"old","futureProject":{"keep":"yes"},
                    "buildNodes":[{"id":"node","futureNodeField":42}]
                }]
            }),
        )
        .unwrap();

        api.call(
            Some("alpha"),
            "POST",
            "/api/config",
            json!({
                "projectPath":"new",
                "id":"overridden","slug":"overridden","name":"Overridden",
                "enabled":false,"lastVersionCode":999
            }),
        )
        .unwrap();
        let status = api
            .call(Some("alpha"), "GET", "/api/status", json!({}))
            .unwrap();
        let config = &status["config"];
        assert_eq!(config["projectPath"], "new");
        assert_eq!(config["id"], "alpha");
        assert_eq!(config["slug"], "alpha");
        assert_eq!(config["name"], "Alpha");
        assert_eq!(config["enabled"], true);
        assert_eq!(config["lastVersionCode"], 7);
        assert_eq!(config["futureProject"]["keep"], "yes");
        assert_eq!(config["buildNodes"][0]["futureNodeField"], 42);

        let root = api.call(None, "GET", "/api/config", json!({})).unwrap();
        assert_eq!(root["config"]["futureRoot"]["enabled"], true);
    }

    #[test]
    fn maps_invalid_inputs_and_missing_routes_to_admin_errors() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Backend::new(temp.path().join("config.json"), temp.path().into());
        let api = AdminApi::new(backend);

        let invalid = api
            .call(None, "POST", "/api/config", json!([]))
            .unwrap_err();
        assert_eq!(invalid.status, 400);
        let missing = api
            .call(None, "GET", "/api/unknown", json!({}))
            .unwrap_err();
        assert_eq!(missing.status, 404);
        let unknown = api
            .call(Some("missing"), "GET", "/api/status", json!({}))
            .unwrap_err();
        assert_eq!(unknown.status, 404);
    }
}
