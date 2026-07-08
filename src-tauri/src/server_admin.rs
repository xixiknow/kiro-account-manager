use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    commands::app_settings_cmd::{self, AppSettings},
    commands::kiro_settings_cmd,
    core::account::{AccountStore, GroupTagData, GroupTagStore},
    gateway::{self, log_store, GatewayConfig, GatewayRequestLogEntry, GatewayStatus},
    services::session_storage::SessionStorage,
};

const ADMIN_TOKEN_ENV: &str = "KAM_ADMIN_TOKEN";
const ADMIN_COOKIE_NAME: &str = "kam_admin_token";

#[derive(Clone)]
pub struct AdminState {
    admin_token: Arc<String>,
    public_base_url: Option<String>,
    started_at: Instant,
    runtime_config: GatewayConfig,
    request_count: Arc<AtomicU64>,
    last_error: Arc<AsyncMutex<Option<String>>>,
    log_store: Arc<log_store::LogStore>,
    accounts: Arc<Mutex<AccountStore>>,
    group_tags: Arc<Mutex<GroupTagStore>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    version: &'static str,
    public_base_url: Option<String>,
    uptime_seconds: u64,
    gateway: GatewayStatus,
    stats: log_store::LogStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResponse {
    imported: usize,
    total: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptCachePayload {
    prompt_cache_target_percent: u16,
}

pub fn require_admin_token_configured() -> Result<(), String> {
    let token = read_admin_token();
    if token.is_empty() {
        return Err(format!(
            "{ADMIN_TOKEN_ENV} is required before starting kam-server"
        ));
    }
    Ok(())
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/admin/api/auth/login", post(login))
        .route("/admin/api/auth/logout", post(logout))
        .route("/admin/api/status", get(status))
        .route("/admin/api/gateway/config", get(get_gateway_config))
        .route("/admin/api/gateway/config", put(save_gateway_config))
        .route("/admin/api/gateway/start", post(start_gateway))
        .route("/admin/api/gateway/stop", post(stop_gateway))
        .route("/admin/api/accounts", get(list_accounts))
        .route("/admin/api/accounts/import", post(import_accounts))
        .route("/admin/api/groups-tags", get(groups_tags))
        .route("/admin/api/app/settings", get(get_app_settings))
        .route("/admin/api/app/settings", put(save_app_settings))
        .route("/admin/api/kiro/settings", get(get_kiro_settings))
        .route("/admin/api/sessions/workspaces", get(session_workspaces))
        .route("/admin/api/sessions", get(sessions))
        .route("/admin/api/about", get(about))
        .route("/admin/api/logs", get(logs))
        .route("/admin/api/prompt-cache", get(get_prompt_cache))
        .route("/admin/api/prompt-cache", put(save_prompt_cache))
        .with_state(state)
}

impl AdminState {
    pub fn from_runtime(
        runtime_config: GatewayConfig,
        request_count: Arc<AtomicU64>,
        last_error: Arc<AsyncMutex<Option<String>>>,
        log_store: Arc<log_store::LogStore>,
    ) -> Result<Self, String> {
        let admin_token = read_admin_token();
        if admin_token.is_empty() {
            return Err(format!("{ADMIN_TOKEN_ENV} is required"));
        }

        let public_base_url = std::env::var("KAM_PUBLIC_BASE_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());

        Ok(Self {
            admin_token: Arc::new(admin_token),
            public_base_url,
            started_at: Instant::now(),
            runtime_config,
            request_count,
            last_error,
            log_store,
            accounts: Arc::new(Mutex::new(AccountStore::new())),
            group_tags: Arc::new(Mutex::new(GroupTagStore::new())),
        })
    }
}

fn read_admin_token() -> String {
    std::env::var(ADMIN_TOKEN_ENV)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{ADMIN_COOKIE_NAME}=")))
        .map(str::to_string)
}

fn is_authorized(headers: &HeaderMap, state: &AdminState) -> bool {
    bearer_token(headers)
        .map(|token| token == state.admin_token.as_str())
        .unwrap_or(false)
        || cookie_token(headers)
            .map(|token| token == state.admin_token.as_str())
            .unwrap_or(false)
}

fn require_auth(headers: &HeaderMap, state: &AdminState) -> Result<(), Response> {
    if is_authorized(headers, state) {
        Ok(())
    } else {
        Err(json_error(StatusCode::UNAUTHORIZED, "unauthorized"))
    }
}

fn auth_cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{ADMIN_COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/"
    ))
    .expect("admin cookie should be a valid header")
}

fn clear_auth_cookie() -> HeaderValue {
    HeaderValue::from_static("kam_admin_token=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

async fn index() -> Html<&'static str> {
    Html(crate::server_web::ADMIN_HTML)
}

async fn healthz() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "kiro-account-manager",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn login(State(state): State<AdminState>, Json(payload): Json<LoginRequest>) -> Response {
    if payload.token.trim() != state.admin_token.as_str() {
        return json_error(StatusCode::UNAUTHORIZED, "invalid admin token");
    }

    let mut response = Json(json!({ "ok": true })).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth_cookie(state.admin_token.as_str()));
    response
}

async fn logout() -> Response {
    let mut response = Json(json!({ "ok": true })).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_auth_cookie());
    response
}

async fn gateway_status(state: &AdminState) -> GatewayStatus {
    let last_error = state.last_error.lock().await.clone();
    GatewayStatus {
        running: true,
        host: state.runtime_config.host.clone(),
        port: state.runtime_config.port,
        request_count: state.request_count.load(Ordering::Relaxed),
        last_error,
        runtime_config: Some(state.runtime_config.clone()),
    }
}

async fn status(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let stats = state.log_store.get_stats().await;
    let body = StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        public_base_url: state.public_base_url.clone(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        gateway: gateway_status(&state).await,
        stats,
    };
    Json(body).into_response()
}

async fn get_gateway_config(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match gateway::get_gateway_config() {
        Ok(config) => Json(config).into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn save_gateway_config(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(config): Json<GatewayConfig>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match gateway::save_gateway_config(&config) {
        Ok(()) => Json(json!({
            "ok": true,
            "restartRequired": true,
            "message": "Configuration saved. Restart the kam-server container to apply listener/runtime changes."
        }))
        .into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

async fn start_gateway(
    headers: HeaderMap,
    State(state): State<AdminState>,
    payload: Option<Json<GatewayConfig>>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let restart_required = payload.is_some();
    if let Some(Json(config)) = payload {
        if let Err(error) = gateway::save_gateway_config(&config) {
            return json_error(StatusCode::BAD_REQUEST, error);
        }
    }

    Json(json!({
        "ok": true,
        "gateway": gateway_status(&state).await,
        "restartRequired": restart_required,
        "message": "kam-server keeps the Gateway listener online; restart the container after changing runtime config."
    }))
    .into_response()
}

async fn stop_gateway(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    Json(json!({
        "ok": true,
        "gateway": gateway_status(&state).await,
        "message": "In server mode the listener is owned by the container process. Stop the Docker container to stop external access."
    }))
    .into_response()
}

async fn list_accounts(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let store = match state.accounts.lock() {
        Ok(store) => store,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "account store lock failed",
            )
        }
    };

    Json(store.get_all()).into_response()
}

async fn groups_tags(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let store = match state.group_tags.lock() {
        Ok(store) => store,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "group/tag store lock failed",
            )
        }
    };

    Json(GroupTagData {
        groups: store.get_groups(),
        tags: store.get_tags(),
    })
    .into_response()
}

async fn get_app_settings(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match app_settings_cmd::get_app_settings_inner() {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn save_app_settings(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(settings): Json<AppSettings>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match app_settings_cmd::save_settings_to_file(&settings) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

async fn get_kiro_settings(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match kiro_settings_cmd::get_kiro_settings().await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => Json(json!({ "unavailable": true, "message": error })).into_response(),
    }
}

async fn session_workspaces(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match SessionStorage::new().and_then(|storage| storage.list_workspaces()) {
        Ok(workspaces) => Json(workspaces).into_response(),
        Err(error) => {
            Json(json!({ "unavailable": true, "message": error.to_string() })).into_response()
        }
    }
}

async fn sessions(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let workspace_hash = query
        .get("workspaceHash")
        .map(String::as_str)
        .unwrap_or_default();
    if workspace_hash.is_empty() {
        return Json(Vec::<Value>::new()).into_response();
    }

    match SessionStorage::new().and_then(|storage| storage.list_sessions(workspace_hash)) {
        Ok(sessions) => Json(sessions).into_response(),
        Err(error) => {
            Json(json!({ "unavailable": true, "message": error.to_string() })).into_response()
        }
    }
}

async fn about(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    Json(json!({
        "name": "Kiro Account Manager",
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "server",
        "image": "ghcr.io/xixiknow/kiro-account-manager",
        "dataDir": std::env::var("KAM_DATA_DIR").unwrap_or_else(|_| "(default user data dir)".to_string())
    }))
    .into_response()
}

async fn import_accounts(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(payload): Json<Value>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let json_text = if let Some(text) = payload.get("json").and_then(Value::as_str) {
        text.to_string()
    } else if let Some(accounts) = payload.get("accounts") {
        accounts.to_string()
    } else if payload.is_array() {
        payload.to_string()
    } else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "expected an account array, { json }, or { accounts }",
        );
    };

    let mut store = match state.accounts.lock() {
        Ok(store) => store,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "account store lock failed",
            )
        }
    };

    match store.import_from_json(&json_text) {
        Ok(imported) => {
            let total = store.accounts.len();
            Json(ImportResponse { imported, total }).into_response()
        }
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

async fn logs(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(120)
        .clamp(1, 500);
    let mut entries: Vec<GatewayRequestLogEntry> = state.log_store.get_last(limit).await;
    entries.reverse();
    Json(entries).into_response()
}

async fn get_prompt_cache(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match gateway::get_gateway_config() {
        Ok(config) => Json(json!({
            "promptCacheTargetPercent": config.prompt_cache_target_percent
        }))
        .into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn save_prompt_cache(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(payload): Json<PromptCachePayload>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }
    if payload.prompt_cache_target_percent > 100 {
        return json_error(
            StatusCode::BAD_REQUEST,
            "promptCacheTargetPercent must be from 0 to 100",
        );
    }

    let mut config = match gateway::get_gateway_config() {
        Ok(config) => config,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    config.prompt_cache_target_percent = payload.prompt_cache_target_percent;

    match gateway::save_gateway_config(&config) {
        Ok(()) => Json(json!({
            "ok": true,
            "promptCacheTargetPercent": config.prompt_cache_target_percent,
            "restartRequired": true
        }))
        .into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}
