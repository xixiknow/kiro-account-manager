use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;
use tower_http::services::ServeDir;

use crate::{
    auth::{auth_social, providers::SocialTokenResponse},
    clients::{
        aws_sso_client::{
            AWSSSOClient, ClientRegistration, DeviceAuthorizationResponse, TokenResponse,
        },
        kiro_auth_client::KiroAuthServiceClient,
    },
    commands::account_cmd::{
        AddAccountResult, UpdateAccountParams, VerifyAccountParams, VerifyAccountResponse,
    },
    commands::account_models::{fetch_all_available_models, write_available_models_cache},
    commands::app_settings_cmd::{self, AppSettings},
    commands::common::{
        apply_refreshed_account_tokens, calc_expires_at, ensure_account_machine_id,
        extract_user_info, extract_user_info_from_jwt, find_existing_account_idx,
        generate_account_machine_id, get_usage_by_account, get_usage_by_provider_with_machine_id,
        is_auth_error_message, refresh_token_by_provider, resolve_idc_client_id_hash, save_store,
        update_account_status, KIRO_BUILDER_ID_START_URL,
    },
    commands::kiro_settings_cmd,
    core::account::{
        Account, AccountProxyConfig, AccountStore, AccountTagLink, GroupTagData, GroupTagStore,
    },
    gateway::{self, log_store, GatewayConfig, GatewayRequestLogEntry, GatewayStatus},
    services::session_storage::SessionStorage,
    utils::client_id_hash::normalize_start_url,
};

const ADMIN_TOKEN_ENV: &str = "KAM_ADMIN_TOKEN";
const ADMIN_COOKIE_NAME: &str = "kam_admin_token";
const ONLINE_LOGIN_TTL: Duration = Duration::from_secs(15 * 60);

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
    online_logins: Arc<Mutex<HashMap<String, PendingOnlineLogin>>>,
    online_idc_logins: Arc<Mutex<HashMap<String, PendingIdcDeviceLogin>>>,
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
    #[serde(default)]
    prompt_cache_ttl_secs: Option<u64>,
    #[serde(default)]
    prompt_cache_max_entries: Option<usize>,
    #[serde(default)]
    prompt_cache_ignore_client_control: Option<bool>,
}

#[derive(Debug, Clone)]
struct PendingOnlineLogin {
    provider: String,
    code_verifier: String,
    redirect_uri: String,
    machine_id: String,
    created_at: Instant,
}

#[derive(Debug, Clone)]
struct PendingIdcDeviceLogin {
    provider: String,
    region: String,
    start_url: String,
    machine_id: String,
    client_registration: ClientRegistration,
    device_authorization: DeviceAuthorizationResponse,
    created_at: Instant,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeginSocialLoginRequest {
    provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BeginSocialLoginResponse {
    authorize_url: String,
    state: String,
    provider: String,
    redirect_uri: String,
    expires_in_seconds: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingOnlineLoginInfo {
    kind: String,
    state: String,
    provider: String,
    redirect_uri: String,
    user_code: Option<String>,
    verification_uri_complete: Option<String>,
    age_seconds: u64,
    expires_in_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeginIdcDeviceLoginRequest {
    provider: String,
    region: Option<String>,
    start_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BeginIdcDeviceLoginResponse {
    state: String,
    provider: String,
    region: String,
    start_url: String,
    verification_uri: String,
    verification_uri_complete: String,
    user_code: String,
    expires_in_seconds: u64,
    interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct IdcPollQuery {
    state: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdcPollResponse {
    state: String,
    provider: String,
    status: String,
    message: String,
    account_display_id: Option<String>,
    expires_in_seconds: u64,
    interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct SocialCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[allow(dead_code)]
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
    let web_dir = resolve_server_web_dir().unwrap_or_else(|| PathBuf::from("server-web-dist"));

    Router::new()
        .route("/", get(index))
        .nest_service("/assets", ServeDir::new(web_dir.join("assets")))
        .route("/healthz", get(healthz))
        .route("/admin/api/auth/login", post(login))
        .route("/admin/api/auth/logout", post(logout))
        .route("/admin/auth/callback/social", get(social_online_callback))
        .route(
            "/admin/api/online-login/social/begin",
            post(begin_social_online_login),
        )
        .route(
            "/admin/api/online-login/idc/begin",
            post(begin_idc_device_login),
        )
        .route(
            "/admin/api/online-login/idc/poll",
            get(poll_idc_device_login),
        )
        .route("/admin/api/online-login/pending", get(online_login_pending))
        .route("/admin/api/status", get(status))
        .route("/admin/api/gateway/config", get(get_gateway_config))
        .route("/admin/api/gateway/config", put(save_gateway_config))
        .route("/admin/api/gateway/start", post(start_gateway))
        .route("/admin/api/gateway/stop", post(stop_gateway))
        .route("/admin/api/accounts", get(list_accounts))
        .route("/admin/api/accounts/import", post(import_accounts))
        .route("/admin/api/invoke/{command}", post(invoke_command))
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
        .fallback(get(index))
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
            online_logins: Arc::new(Mutex::new(HashMap::new())),
            online_idc_logins: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

fn read_admin_token() -> String {
    std::env::var(ADMIN_TOKEN_ENV)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn resolve_server_web_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var("KAM_WEB_DIR").ok().map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("server-web"))),
        Some(PathBuf::from("/opt/kam/server-web")),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.join("index.html").is_file())
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

fn normalize_social_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "google" => Some("Google"),
        "github" => Some("Github"),
        _ => None,
    }
}

fn normalize_idc_provider(provider: &str) -> Option<&'static str> {
    match provider
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
        .as_str()
    {
        "builderid" | "awsbuilderid" => Some("BuilderId"),
        "iam" | "enterprise" | "iamidentitycenter" | "identitycenter" | "awsiamidentitycenter" => {
            Some("Enterprise")
        }
        _ => None,
    }
}

fn resolve_idc_start_url(provider: &str, start_url: Option<String>) -> Result<String, String> {
    if provider == "BuilderId" {
        return Ok(KIRO_BUILDER_ID_START_URL.to_string());
    }

    start_url
        .map(|url| normalize_start_url(&url))
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| {
            "IAM Identity Center 登录需要填写 Start URL，例如 https://d-1234567890.awsapps.com/start"
                .to_string()
        })
}

fn resolve_idc_region(region: Option<String>) -> String {
    region
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "us-east-1".to_string())
}

fn idc_refresh_token_fallback_identity(refresh_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(refresh_token.as_bytes());
    let digest = hasher.finalize();
    let prefix = digest[..3]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("kiro_{prefix}")
}

fn resolve_idc_account_identity(
    email: Option<String>,
    user_id: Option<String>,
    refresh_token: &str,
) -> String {
    email
        .or(user_id)
        .unwrap_or_else(|| idc_refresh_token_fallback_identity(refresh_token))
}

fn merge_optional_identity(
    primary_email: Option<String>,
    primary_user_id: Option<String>,
    fallback_email: Option<String>,
    fallback_user_id: Option<String>,
) -> (Option<String>, Option<String>) {
    (
        primary_email.or(fallback_email),
        primary_user_id.or(fallback_user_id),
    )
}

fn resolve_stored_idc_user_id(
    provider: &str,
    display_id: &str,
    user_id: Option<String>,
) -> Option<String> {
    user_id.or_else(|| {
        (provider == "Enterprise" && !display_id.trim().is_empty())
            .then(|| display_id.trim().to_string())
    })
}

fn first_forwarded_value(value: &str) -> Option<String> {
    value
        .split(',')
        .next()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(first_forwarded_value)
}

fn resolve_public_base_url(headers: &HeaderMap, state: &AdminState) -> Result<String, String> {
    if let Some(public_base_url) = state.public_base_url.clone() {
        return Ok(public_base_url);
    }

    let host = header_string(headers, "x-forwarded-host")
        .or_else(|| header_string(headers, "host"))
        .ok_or_else(|| {
            "KAM_PUBLIC_BASE_URL is required when the public host cannot be derived".to_string()
        })?;
    let proto = header_string(headers, "x-forwarded-proto").unwrap_or_else(|| "http".to_string());

    Ok(format!("{}://{}", proto.trim_end_matches("://"), host)
        .trim_end_matches('/')
        .to_string())
}

fn cleanup_expired_online_logins(logins: &mut HashMap<String, PendingOnlineLogin>) {
    logins.retain(|_, pending| pending.created_at.elapsed() < ONLINE_LOGIN_TTL);
}

fn cleanup_expired_idc_logins(logins: &mut HashMap<String, PendingIdcDeviceLogin>) {
    logins.retain(|_, pending| pending.created_at.elapsed() < ONLINE_LOGIN_TTL);
}

fn online_login_remaining(created_at: Instant, upstream_expires_in: Option<i64>) -> u64 {
    let ttl = upstream_expires_in
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or(ONLINE_LOGIN_TTL)
        .min(ONLINE_LOGIN_TTL);
    ttl.as_secs().saturating_sub(created_at.elapsed().as_secs())
}

fn idc_poll_interval(device_authorization: &DeviceAuthorizationResponse) -> u64 {
    u64::try_from(device_authorization.interval)
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(5)
}

fn is_idc_poll_pending_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("authorization_pending") || lower.contains("slow_down")
}

fn is_idc_poll_expired_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("expired_token") || lower.contains("expiredtoken") || lower.contains("expired")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn callback_page(status: StatusCode, ok: bool, title: &str, message: &str) -> Response {
    let safe_title = escape_html(title);
    let safe_message = escape_html(message);
    let js_message = serde_json::to_string(message)
        .unwrap_or_else(|_| "\"login callback finished\"".to_string());
    let ok_literal = if ok { "true" } else { "false" };
    let accent = if ok { "#66d48e" } else { "#f07272" };
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{safe_title}</title>
    <style>
      :root {{ color-scheme: dark; font-family: "Aptos", "Segoe UI", "Microsoft YaHei", sans-serif; }}
      body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: #0b0f11; color: #eef5f1; }}
      main {{ width: min(520px, calc(100vw - 32px)); border: 1px solid #314047; border-radius: 10px; background: #151d20; padding: 24px; box-shadow: 0 24px 80px rgba(0, 0, 0, .34); }}
      .mark {{ width: 46px; height: 46px; display: grid; place-items: center; border-radius: 8px; background: {accent}; color: #07100b; font-weight: 900; margin-bottom: 16px; }}
      h1 {{ margin: 0 0 8px; font-size: 24px; letter-spacing: 0; }}
      p {{ margin: 0; color: #9eb3b5; line-height: 1.7; }}
    </style>
  </head>
  <body>
    <main>
      <div class="mark">K</div>
      <h1>{safe_title}</h1>
      <p>{safe_message}</p>
    </main>
    <script>
      try {{
        const message = {{ type: "kam-online-login-complete", ok: {ok_literal}, message: {js_message} }};
        if ("BroadcastChannel" in window) {{
          const channel = new BroadcastChannel("kam-online-login");
          channel.postMessage(message);
          channel.close();
        }}
        if (window.opener) {{
          window.opener.postMessage(message, window.location.origin);
        }}
        window.setTimeout(() => window.close(), 1200);
      }} catch (_error) {{}}
    </script>
  </body>
</html>"#
    );
    (status, Html(body)).into_response()
}

async fn index() -> Html<String> {
    let html = resolve_server_web_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("index.html")).ok())
        .unwrap_or_else(|| crate::server_web::ADMIN_HTML.to_string());
    Html(html)
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

async fn begin_social_online_login(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(payload): Json<BeginSocialLoginRequest>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let provider = match normalize_social_provider(&payload.provider) {
        Some(provider) => provider.to_string(),
        None => return json_error(StatusCode::BAD_REQUEST, "provider must be Google or Github"),
    };

    let public_base_url = match resolve_public_base_url(&headers, &state) {
        Ok(url) => url,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let redirect_uri = format!("{public_base_url}/admin/auth/callback/social");
    let state_id = uuid::Uuid::new_v4().to_string();
    let code_verifier = auth_social::generate_code_verifier_social();
    let code_challenge = auth_social::generate_code_challenge_social(&code_verifier);
    let machine_id = generate_account_machine_id();
    let client = match KiroAuthServiceClient::new(&machine_id) {
        Ok(client) => client,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    let authorize_url =
        client.build_login_url(&provider, &redirect_uri, &code_challenge, &state_id);

    let mut logins = match state.online_logins.lock() {
        Ok(logins) => logins,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "online login store lock failed",
            )
        }
    };
    cleanup_expired_online_logins(&mut logins);
    logins.insert(
        state_id.clone(),
        PendingOnlineLogin {
            provider: provider.clone(),
            code_verifier,
            redirect_uri: redirect_uri.clone(),
            machine_id,
            created_at: Instant::now(),
        },
    );

    Json(BeginSocialLoginResponse {
        authorize_url,
        state: state_id,
        provider,
        redirect_uri,
        expires_in_seconds: ONLINE_LOGIN_TTL.as_secs(),
    })
    .into_response()
}

async fn begin_idc_device_login(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(payload): Json<BeginIdcDeviceLoginRequest>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let provider = match normalize_idc_provider(&payload.provider) {
        Some(provider) => provider.to_string(),
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "provider must be BuilderId, IAM, or Enterprise",
            )
        }
    };
    let region = resolve_idc_region(payload.region);
    let start_url = match resolve_idc_start_url(&provider, payload.start_url) {
        Ok(start_url) => start_url,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };

    let sso_client = AWSSSOClient::new(&region);
    let client_registration = match sso_client
        .register_device_client(&start_url, provider == "Enterprise")
        .await
    {
        Ok(registration) => registration,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let device_authorization = match sso_client
        .start_device_authorization(
            &client_registration.client_id,
            &client_registration.client_secret,
            &start_url,
        )
        .await
    {
        Ok(device_authorization) => device_authorization,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };

    let state_id = uuid::Uuid::new_v4().to_string();
    let machine_id = generate_account_machine_id();
    let verification_uri_complete = device_authorization
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| device_authorization.verification_uri.clone());

    let mut logins = match state.online_idc_logins.lock() {
        Ok(logins) => logins,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "online IdC login store lock failed",
            )
        }
    };
    cleanup_expired_idc_logins(&mut logins);
    let interval_seconds = idc_poll_interval(&device_authorization);
    logins.insert(
        state_id.clone(),
        PendingIdcDeviceLogin {
            provider: provider.clone(),
            region: region.clone(),
            start_url: start_url.clone(),
            machine_id,
            client_registration,
            device_authorization: device_authorization.clone(),
            created_at: Instant::now(),
        },
    );

    Json(BeginIdcDeviceLoginResponse {
        state: state_id,
        provider,
        region,
        start_url,
        verification_uri: device_authorization.verification_uri,
        verification_uri_complete,
        user_code: device_authorization.user_code,
        expires_in_seconds: online_login_remaining(
            Instant::now(),
            Some(device_authorization.expires_in),
        ),
        interval_seconds,
    })
    .into_response()
}

async fn poll_idc_device_login(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(query): Query<IdcPollQuery>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let state_id = query.state.trim().to_string();
    if state_id.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "state is required");
    }

    let pending = {
        let mut logins = match state.online_idc_logins.lock() {
            Ok(logins) => logins,
            Err(_) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "online IdC login store lock failed",
                )
            }
        };
        cleanup_expired_idc_logins(&mut logins);
        logins.get(&state_id).cloned()
    };

    let Some(pending) = pending else {
        return json_error(
            StatusCode::NOT_FOUND,
            "online IdC login not found or expired",
        );
    };

    if online_login_remaining(
        pending.created_at,
        Some(pending.device_authorization.expires_in),
    ) == 0
    {
        if let Ok(mut logins) = state.online_idc_logins.lock() {
            logins.remove(&state_id);
        }
        return Json(IdcPollResponse {
            state: state_id,
            provider: pending.provider,
            status: "error".to_string(),
            message: "AWS 登录验证码已过期，请重新发起登录。".to_string(),
            account_display_id: None,
            expires_in_seconds: 0,
            interval_seconds: idc_poll_interval(&pending.device_authorization),
        })
        .into_response();
    }

    let sso_client = AWSSSOClient::new(&pending.region);
    let token_response = match sso_client
        .create_token_with_device_code(
            &pending.client_registration.client_id,
            &pending.client_registration.client_secret,
            &pending.device_authorization.device_code,
        )
        .await
    {
        Ok(token_response) => token_response,
        Err(error) if is_idc_poll_pending_error(&error) => {
            return Json(IdcPollResponse {
                state: state_id,
                provider: pending.provider,
                status: "pending".to_string(),
                message: "等待 AWS 授权完成。".to_string(),
                account_display_id: None,
                expires_in_seconds: online_login_remaining(
                    pending.created_at,
                    Some(pending.device_authorization.expires_in),
                ),
                interval_seconds: idc_poll_interval(&pending.device_authorization),
            })
            .into_response()
        }
        Err(error) if is_idc_poll_expired_error(&error) => {
            if let Ok(mut logins) = state.online_idc_logins.lock() {
                logins.remove(&state_id);
            }
            return Json(IdcPollResponse {
                state: state_id,
                provider: pending.provider,
                status: "error".to_string(),
                message: "AWS 登录验证码已过期，请重新发起登录。".to_string(),
                account_display_id: None,
                expires_in_seconds: 0,
                interval_seconds: idc_poll_interval(&pending.device_authorization),
            })
            .into_response();
        }
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, error);
        }
    };

    match finish_idc_device_login(&state, pending.clone(), token_response).await {
        Ok(account) => {
            if let Ok(mut logins) = state.online_idc_logins.lock() {
                logins.remove(&state_id);
            }
            Json(IdcPollResponse {
                state: state_id,
                provider: pending.provider,
                status: "complete".to_string(),
                message: "AWS 在线登录成功。".to_string(),
                account_display_id: Some(account.get_display_id()),
                expires_in_seconds: 0,
                interval_seconds: idc_poll_interval(&pending.device_authorization),
            })
            .into_response()
        }
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

async fn online_login_pending(headers: HeaderMap, State(state): State<AdminState>) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    let mut logins = match state.online_logins.lock() {
        Ok(logins) => logins,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "online login store lock failed",
            )
        }
    };
    cleanup_expired_online_logins(&mut logins);

    let mut pending: Vec<PendingOnlineLoginInfo> = logins
        .iter()
        .map(|(state_id, pending)| {
            let age_seconds = pending.created_at.elapsed().as_secs();
            PendingOnlineLoginInfo {
                kind: "social".to_string(),
                state: state_id.clone(),
                provider: pending.provider.clone(),
                redirect_uri: pending.redirect_uri.clone(),
                user_code: None,
                verification_uri_complete: None,
                age_seconds,
                expires_in_seconds: ONLINE_LOGIN_TTL.as_secs().saturating_sub(age_seconds),
            }
        })
        .collect();

    let mut idc_logins = match state.online_idc_logins.lock() {
        Ok(logins) => logins,
        Err(_) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "online IdC login store lock failed",
            )
        }
    };
    cleanup_expired_idc_logins(&mut idc_logins);
    pending.extend(idc_logins.iter().map(|(state_id, pending)| {
        let age_seconds = pending.created_at.elapsed().as_secs();
        let verification_uri_complete = pending
            .device_authorization
            .verification_uri_complete
            .clone()
            .unwrap_or_else(|| pending.device_authorization.verification_uri.clone());
        PendingOnlineLoginInfo {
            kind: "idc".to_string(),
            state: state_id.clone(),
            provider: pending.provider.clone(),
            redirect_uri: verification_uri_complete.clone(),
            user_code: Some(pending.device_authorization.user_code.clone()),
            verification_uri_complete: Some(verification_uri_complete),
            age_seconds,
            expires_in_seconds: online_login_remaining(
                pending.created_at,
                Some(pending.device_authorization.expires_in),
            ),
        }
    }));
    Json(pending).into_response()
}

async fn social_online_callback(
    State(state): State<AdminState>,
    Query(query): Query<SocialCallbackQuery>,
) -> Response {
    if let Some(error) = query.error {
        let detail = query.error_description.unwrap_or(error);
        return callback_page(
            StatusCode::BAD_REQUEST,
            false,
            "在线登录失败",
            &format!("Kiro 授权返回错误：{detail}"),
        );
    }

    let code = match query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty())
    {
        Some(code) => code.to_string(),
        None => {
            return callback_page(
                StatusCode::BAD_REQUEST,
                false,
                "在线登录失败",
                "授权回调缺少 code 参数。",
            )
        }
    };
    let callback_state = match query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|state| !state.is_empty())
    {
        Some(callback_state) => callback_state.to_string(),
        None => {
            return callback_page(
                StatusCode::BAD_REQUEST,
                false,
                "在线登录失败",
                "授权回调缺少 state 参数。",
            )
        }
    };

    let pending = {
        let mut logins = match state.online_logins.lock() {
            Ok(logins) => logins,
            Err(_) => {
                return callback_page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    false,
                    "在线登录失败",
                    "在线登录状态读取失败。",
                )
            }
        };
        cleanup_expired_online_logins(&mut logins);
        logins.remove(&callback_state)
    };

    let Some(pending) = pending else {
        return callback_page(
            StatusCode::BAD_REQUEST,
            false,
            "在线登录已过期",
            "没有找到匹配的登录请求，请回到管理后台重新发起在线登录。",
        );
    };

    match finish_social_online_login(&state, pending, &code).await {
        Ok(account) => callback_page(
            StatusCode::OK,
            true,
            "在线登录成功",
            &format!(
                "账号 {} 已写入账号池，窗口稍后会自动关闭。",
                account.get_display_id()
            ),
        ),
        Err(error) => callback_page(StatusCode::BAD_REQUEST, false, "在线登录失败", &error),
    }
}

async fn finish_social_online_login(
    state: &AdminState,
    pending: PendingOnlineLogin,
    code: &str,
) -> Result<Account, String> {
    let client = KiroAuthServiceClient::new(&pending.machine_id)?;
    let token_response: SocialTokenResponse = client
        .create_token(code, &pending.code_verifier, &pending.redirect_uri, None)
        .await?;

    let usage_result = get_usage_by_provider_with_machine_id(
        &pending.provider,
        &token_response.access_token,
        &pending.machine_id,
    )
    .await?;

    if usage_result.is_banned {
        return Err("BANNED: 账号已被封禁".to_string());
    }

    let resolved_profile_arn = token_response
        .profile_arn
        .clone()
        .or_else(|| usage_result.profile_arn.clone());
    let (new_email, user_id) = extract_user_info(&usage_result.usage_data);
    let final_email = new_email.clone().or(user_id.clone()).unwrap_or_else(|| {
        format!(
            "{}_{}",
            pending.provider.to_lowercase(),
            token_response
                .refresh_token
                .chars()
                .take(8)
                .collect::<String>()
        )
    });

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let existing_idx = find_existing_account_idx(
        &store.accounts,
        new_email.as_ref(),
        &pending.provider,
        &token_response.refresh_token,
        user_id.as_ref(),
    );

    let account = if let Some(idx) = existing_idx {
        let existing = &mut store.accounts[idx];
        existing.access_token = Some(token_response.access_token.clone());
        existing.refresh_token = Some(token_response.refresh_token.clone());
        existing.expires_at = Some(calc_expires_at(token_response.expires_in));
        existing.provider = Some(pending.provider.clone());
        existing.auth_method = Some("social".to_string());
        if new_email.is_some() {
            existing.email.clone_from(&new_email);
        }
        existing.user_id = user_id;
        existing.id_token = token_response.id_token.clone();
        existing.profile_arn.clone_from(&resolved_profile_arn);
        existing.usage_data = Some(usage_result.usage_data);
        if existing
            .machine_id
            .as_ref()
            .is_none_or(|id| id.trim().is_empty())
        {
            existing.machine_id = Some(pending.machine_id.clone());
        }
        update_account_status(existing, usage_result.is_banned, usage_result.is_auth_error);
        existing.clone()
    } else {
        let mut account = Account::new(final_email, format!("Kiro {} 账号", pending.provider));
        account.access_token = Some(token_response.access_token.clone());
        account.refresh_token = Some(token_response.refresh_token.clone());
        account.expires_at = Some(calc_expires_at(token_response.expires_in));
        account.provider = Some(pending.provider.clone());
        account.auth_method = Some("social".to_string());
        account.user_id = user_id;
        account.id_token = token_response.id_token.clone();
        account.profile_arn = resolved_profile_arn;
        account.usage_data = Some(usage_result.usage_data);
        account.machine_id = Some(pending.machine_id.clone());
        update_account_status(
            &mut account,
            usage_result.is_banned,
            usage_result.is_auth_error,
        );
        store.accounts.insert(0, account.clone());
        account
    };

    save_store(&store)?;
    Ok(account)
}

async fn finish_idc_device_login(
    state: &AdminState,
    pending: PendingIdcDeviceLogin,
    token_response: TokenResponse,
) -> Result<Account, String> {
    let usage_result = get_usage_by_provider_with_machine_id(
        &pending.provider,
        &token_response.access_token,
        &pending.machine_id,
    )
    .await?;

    if usage_result.is_banned {
        return Err("BANNED: 账号已被封禁".to_string());
    }

    let resolved_profile_arn = usage_result.profile_arn.clone();
    let (usage_email, usage_user_id) = extract_user_info(&usage_result.usage_data);
    let jwt_identity = token_response
        .id_token
        .as_deref()
        .map(extract_user_info_from_jwt)
        .filter(|(email, user_id)| email.is_some() || user_id.is_some())
        .or_else(|| {
            let identity = extract_user_info_from_jwt(&token_response.access_token);
            (identity.0.is_some() || identity.1.is_some()).then_some(identity)
        })
        .unwrap_or((None, None));
    let (new_email, user_id) =
        merge_optional_identity(usage_email, usage_user_id, jwt_identity.0, jwt_identity.1);
    let display_id = resolve_idc_account_identity(
        new_email.clone(),
        user_id.clone(),
        &token_response.refresh_token,
    );
    let stored_user_id =
        resolve_stored_idc_user_id(&pending.provider, &display_id, user_id.clone());
    let account_start_url = if pending.provider == "Enterprise" {
        Some(pending.start_url.clone())
    } else {
        None
    };
    let client_id_hash = resolve_idc_client_id_hash(
        &pending.provider,
        None,
        account_start_url
            .as_deref()
            .or(Some(pending.start_url.as_str())),
    )?;

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let existing_idx = find_existing_account_idx(
        &store.accounts,
        new_email.as_ref(),
        &pending.provider,
        &token_response.refresh_token,
        user_id.as_ref(),
    );

    let account = if let Some(idx) = existing_idx {
        let existing = &mut store.accounts[idx];
        existing.access_token = Some(token_response.access_token.clone());
        existing.refresh_token = Some(token_response.refresh_token.clone());
        if pending.provider == "Enterprise" || new_email.is_some() {
            existing.email.clone_from(&new_email);
        }
        existing.user_id.clone_from(&stored_user_id);
        existing.provider = Some(pending.provider.clone());
        existing.auth_method = Some("IdC".to_string());
        existing.expires_at = Some(calc_expires_at(token_response.expires_in));
        existing.client_id = Some(pending.client_registration.client_id.clone());
        existing.client_secret = Some(pending.client_registration.client_secret.clone());
        existing.client_id_hash = Some(client_id_hash.clone());
        existing.region = Some(pending.region.clone());
        existing.start_url = account_start_url.clone();
        existing.sso_session_id = token_response.aws_sso_app_session_id.clone();
        existing.id_token = token_response.id_token.clone();
        existing.profile_arn.clone_from(&resolved_profile_arn);
        existing.usage_data = Some(usage_result.usage_data);
        if existing
            .machine_id
            .as_ref()
            .is_none_or(|id| id.trim().is_empty())
        {
            existing.machine_id = Some(pending.machine_id.clone());
        }
        update_account_status(existing, usage_result.is_banned, usage_result.is_auth_error);
        existing.clone()
    } else {
        let mut account = if pending.provider == "Enterprise" {
            Account::new_enterprise(
                display_id.clone(),
                "Kiro IAM Identity Center 账号".to_string(),
            )
        } else {
            Account::new(display_id.clone(), "Kiro BuilderId 账号".to_string())
        };
        if pending.provider == "Enterprise" || new_email.is_some() {
            account.email = new_email;
        }
        account.access_token = Some(token_response.access_token.clone());
        account.refresh_token = Some(token_response.refresh_token.clone());
        account.provider = Some(pending.provider.clone());
        account.auth_method = Some("IdC".to_string());
        account.user_id = stored_user_id;
        account.expires_at = Some(calc_expires_at(token_response.expires_in));
        account.client_id = Some(pending.client_registration.client_id.clone());
        account.client_secret = Some(pending.client_registration.client_secret.clone());
        account.client_id_hash = Some(client_id_hash);
        account.region = Some(pending.region.clone());
        account.start_url = account_start_url;
        account.sso_session_id = token_response.aws_sso_app_session_id.clone();
        account.id_token = token_response.id_token.clone();
        account.profile_arn = resolved_profile_arn;
        account.usage_data = Some(usage_result.usage_data);
        account.machine_id = Some(pending.machine_id.clone());
        update_account_status(
            &mut account,
            usage_result.is_banned,
            usage_result.is_auth_error,
        );
        store.accounts.insert(0, account.clone());
        account
    };

    save_store(&store)?;
    Ok(account)
}

async fn server_add_account_by_social(
    state: &AdminState,
    refresh_token: String,
    provider: Option<String>,
    machine_id: Option<String>,
    access_token: Option<String>,
) -> Result<AddAccountResult, String> {
    let provider = provider
        .as_deref()
        .and_then(normalize_social_provider)
        .unwrap_or("Google")
        .to_string();
    let machine_id = machine_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_account_machine_id);
    let mut account = Account::new("pending".to_string(), format!("Kiro {provider} 账号"));
    account.provider = Some(provider.clone());
    account.auth_method = Some("social".to_string());
    account.refresh_token = Some(refresh_token.clone());
    account.machine_id = Some(machine_id.clone());

    let refresh = refresh_token_by_provider(&account).await?;
    apply_refreshed_account_tokens(&mut account, &refresh);
    if let Some(access_token) = access_token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        account.access_token = Some(access_token);
    }
    let final_access_token = account
        .access_token
        .clone()
        .ok_or_else(|| "No access token after refresh".to_string())?;
    let final_refresh_token = account
        .refresh_token
        .clone()
        .unwrap_or_else(|| refresh_token.clone());

    let usage_result =
        get_usage_by_provider_with_machine_id(&provider, &final_access_token, &machine_id).await?;
    if usage_result.is_banned {
        return Err("BANNED: 账号已被封禁".to_string());
    }
    if let Some(profile_arn) = usage_result.profile_arn.clone() {
        account.profile_arn = Some(profile_arn);
    }
    let (new_email, user_id) = extract_user_info(&usage_result.usage_data);
    let display_id = new_email.clone().or(user_id.clone()).unwrap_or_else(|| {
        format!(
            "{}_{}",
            provider.to_lowercase(),
            final_refresh_token.chars().take(8).collect::<String>()
        )
    });

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let existing_idx = find_existing_account_idx(
        &store.accounts,
        new_email.as_ref(),
        &provider,
        &final_refresh_token,
        user_id.as_ref(),
    );
    let is_new = existing_idx.is_none();
    let result_account = if let Some(idx) = existing_idx {
        let existing = &mut store.accounts[idx];
        existing.access_token.clone_from(&account.access_token);
        existing.refresh_token = Some(final_refresh_token.clone());
        existing.expires_at.clone_from(&account.expires_at);
        existing.provider = Some(provider.clone());
        existing.auth_method = Some("social".to_string());
        if new_email.is_some() {
            existing.email.clone_from(&new_email);
        }
        existing.user_id = user_id;
        existing.id_token.clone_from(&account.id_token);
        existing.profile_arn.clone_from(&account.profile_arn);
        existing.usage_data = Some(usage_result.usage_data);
        if existing
            .machine_id
            .as_ref()
            .is_none_or(|id| id.trim().is_empty())
        {
            existing.machine_id = Some(machine_id);
        }
        update_account_status(existing, usage_result.is_banned, usage_result.is_auth_error);
        existing.clone()
    } else {
        account.email = Some(display_id);
        account.user_id = user_id;
        account.refresh_token = Some(final_refresh_token);
        account.usage_data = Some(usage_result.usage_data);
        update_account_status(
            &mut account,
            usage_result.is_banned,
            usage_result.is_auth_error,
        );
        store.accounts.insert(0, account.clone());
        account
    };

    save_store(&store)?;
    Ok(AddAccountResult {
        account: result_account,
        is_new,
    })
}

async fn server_add_account_by_idc(
    state: &AdminState,
    payload: &Value,
) -> Result<AddAccountResult, String> {
    let provider = payload
        .get("provider")
        .and_then(Value::as_str)
        .and_then(normalize_idc_provider)
        .unwrap_or("BuilderId")
        .to_string();
    let refresh_token = payload_string(payload, "refreshToken")?;
    let client_id = payload_string(payload, "clientId")?;
    let client_secret = payload_string(payload, "clientSecret")?;
    let region = payload
        .get("region")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("us-east-1")
        .to_string();
    let machine_id = payload
        .get("machineId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(generate_account_machine_id);
    let start_url = if provider == "Enterprise" {
        Some(resolve_idc_start_url(
            &provider,
            payload
                .get("startUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
        )?)
    } else {
        None
    };

    let mut account = if provider == "Enterprise" {
        Account::new_enterprise(
            "pending".to_string(),
            "Kiro IAM Identity Center 账号".to_string(),
        )
    } else {
        Account::new("pending".to_string(), "Kiro BuilderId 账号".to_string())
    };
    account.provider = Some(provider.clone());
    account.auth_method = Some("IdC".to_string());
    account.refresh_token = Some(refresh_token.clone());
    account.client_id = Some(client_id);
    account.client_secret = Some(client_secret);
    account.region = Some(region.clone());
    account.machine_id = Some(machine_id.clone());
    account.start_url.clone_from(&start_url);
    account.password = payload
        .get("password")
        .and_then(Value::as_str)
        .map(str::to_string);
    account.client_id_hash = payload
        .get("clientIdHash")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| resolve_idc_client_id_hash(&provider, None, start_url.as_deref()).ok());

    let refresh = refresh_token_by_provider(&account).await?;
    apply_refreshed_account_tokens(&mut account, &refresh);
    if let Some(access_token) = payload
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        account.access_token = Some(access_token.to_string());
    }
    let access_token = account
        .access_token
        .clone()
        .ok_or_else(|| "No access token after refresh".to_string())?;
    let final_refresh_token = account
        .refresh_token
        .clone()
        .unwrap_or_else(|| refresh_token.clone());
    let usage_result =
        get_usage_by_provider_with_machine_id(&provider, &access_token, &machine_id).await?;
    if usage_result.is_banned {
        return Err("BANNED: 账号已被封禁".to_string());
    }
    if let Some(profile_arn) = usage_result.profile_arn.clone() {
        account.profile_arn = Some(profile_arn);
    }

    let (usage_email, usage_user_id) = extract_user_info(&usage_result.usage_data);
    let jwt_identity = account
        .id_token
        .as_deref()
        .map(extract_user_info_from_jwt)
        .filter(|(email, user_id)| email.is_some() || user_id.is_some())
        .or_else(|| {
            let identity = extract_user_info_from_jwt(&access_token);
            (identity.0.is_some() || identity.1.is_some()).then_some(identity)
        })
        .unwrap_or((None, None));
    let (new_email, user_id) =
        merge_optional_identity(usage_email, usage_user_id, jwt_identity.0, jwt_identity.1);
    let display_id =
        resolve_idc_account_identity(new_email.clone(), user_id.clone(), &final_refresh_token);
    let stored_user_id = resolve_stored_idc_user_id(&provider, &display_id, user_id.clone());

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let existing_idx = find_existing_account_idx(
        &store.accounts,
        new_email.as_ref(),
        &provider,
        &final_refresh_token,
        user_id.as_ref(),
    );
    let is_new = existing_idx.is_none();
    let result_account = if let Some(idx) = existing_idx {
        let existing = &mut store.accounts[idx];
        existing.access_token.clone_from(&account.access_token);
        existing.refresh_token = Some(final_refresh_token.clone());
        existing.expires_at.clone_from(&account.expires_at);
        existing.provider = Some(provider.clone());
        existing.auth_method = Some("IdC".to_string());
        if provider == "Enterprise" || new_email.is_some() {
            existing.email.clone_from(&new_email);
        }
        existing.user_id.clone_from(&stored_user_id);
        existing.client_id.clone_from(&account.client_id);
        existing.client_secret.clone_from(&account.client_secret);
        existing.client_id_hash.clone_from(&account.client_id_hash);
        existing.region = Some(region.clone());
        existing.start_url.clone_from(&start_url);
        existing.sso_session_id.clone_from(&account.sso_session_id);
        existing.id_token.clone_from(&account.id_token);
        existing.profile_arn.clone_from(&account.profile_arn);
        existing.password.clone_from(&account.password);
        existing.usage_data = Some(usage_result.usage_data);
        if existing
            .machine_id
            .as_ref()
            .is_none_or(|id| id.trim().is_empty())
        {
            existing.machine_id = Some(machine_id);
        }
        update_account_status(existing, usage_result.is_banned, usage_result.is_auth_error);
        existing.clone()
    } else {
        if provider == "Enterprise" || new_email.is_some() {
            account.email = new_email;
        } else {
            account.email = Some(display_id.clone());
        }
        account.user_id = stored_user_id;
        account.refresh_token = Some(final_refresh_token);
        account.usage_data = Some(usage_result.usage_data);
        update_account_status(
            &mut account,
            usage_result.is_banned,
            usage_result.is_auth_error,
        );
        store.accounts.insert(0, account.clone());
        account
    };

    save_store(&store)?;
    Ok(AddAccountResult {
        account: result_account,
        is_new,
    })
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

fn missing_arg(name: &str) -> Response {
    json_error(StatusCode::BAD_REQUEST, format!("missing argument: {name}"))
}

fn json_result<T: Serialize>(result: Result<T, String>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

fn arg_string(payload: &Value, name: &str) -> Result<String, Response> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| missing_arg(name))
}

fn payload_string(payload: &Value, name: &str) -> Result<String, String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("{name} is required"))
}

fn arg_bool(payload: &Value, name: &str) -> Result<bool, Response> {
    payload
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| missing_arg(name))
}

fn arg_string_vec(payload: &Value, name: &str) -> Result<Vec<String>, Response> {
    payload
        .get(name)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| json_error(StatusCode::BAD_REQUEST, error.to_string()))?
        .ok_or_else(|| missing_arg(name))
}

fn set_account_status_from_error(account: &mut Account, error: &str) {
    if error.starts_with("BANNED:") {
        account.status = "banned".to_string();
        account.enabled = false;
    } else if is_auth_error_message(error) {
        account.status = "invalid".to_string();
        account.enabled = false;
    }
}

async fn server_sync_account(state: &AdminState, id: String) -> Result<Value, String> {
    let mut account = {
        let mut store = state
            .accounts
            .lock()
            .map_err(|_| "account store lock failed".to_string())?;
        let mut should_save = false;
        let account = {
            let stored = store
                .accounts
                .iter_mut()
                .find(|account| account.id == id)
                .ok_or_else(|| "账号不存在".to_string())?;
            if stored
                .machine_id
                .as_ref()
                .is_none_or(|machine_id| machine_id.trim().is_empty())
            {
                ensure_account_machine_id(stored);
                should_save = true;
            }
            stored.clone()
        };
        if should_save {
            save_store(&store)?;
        }
        account
    };

    let access_token = account
        .access_token
        .clone()
        .ok_or_else(|| "No access token".to_string())?;

    let mut usage_result = get_usage_by_account(&account, &access_token).await;
    let mut warning = None;

    if matches!(usage_result, Ok(ref result) if result.is_auth_error) {
        match refresh_token_by_provider(&account).await {
            Ok(refresh) => {
                apply_refreshed_account_tokens(&mut account, &refresh);
                let retry_token = account
                    .access_token
                    .clone()
                    .ok_or_else(|| "No access token after refresh".to_string())?;
                usage_result = get_usage_by_account(&account, &retry_token).await;
            }
            Err(error) => {
                let mut store = state
                    .accounts
                    .lock()
                    .map_err(|_| "account store lock failed".to_string())?;
                if let Some(stored) = store.accounts.iter_mut().find(|item| item.id == id) {
                    set_account_status_from_error(stored, &error);
                    save_store(&store)?;
                }
                return Err(error);
            }
        }
    }

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let stored = store
        .accounts
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;

    if stored
        .machine_id
        .as_ref()
        .is_none_or(|machine_id| machine_id.trim().is_empty())
    {
        stored.machine_id.clone_from(&account.machine_id);
    }
    if account.access_token.is_some() {
        stored.access_token.clone_from(&account.access_token);
        stored.refresh_token.clone_from(&account.refresh_token);
        stored.expires_at.clone_from(&account.expires_at);
        stored.id_token.clone_from(&account.id_token);
        stored.sso_session_id.clone_from(&account.sso_session_id);
    }

    match usage_result {
        Ok(usage) => {
            if let Some(profile_arn) = usage.profile_arn.clone() {
                stored.profile_arn = Some(profile_arn);
            }
            stored.usage_data = Some(usage.usage_data);
            update_account_status(stored, usage.is_banned, usage.is_auth_error);
            if stored.status == "active" {
                stored.enabled = true;
            }
        }
        Err(error) => {
            warning = Some(format!("获取配额失败: {error}"));
            if !matches!(stored.status.as_str(), "banned" | "封禁" | "已封禁")
                && !is_auth_error_message(&error)
            {
                stored.status = "active".to_string();
                stored.enabled = true;
            }
        }
    }

    let account = stored.clone();
    save_store(&store)?;
    Ok(json!({ "account": account, "warning": warning }))
}

async fn server_refresh_account_token(state: &AdminState, id: String) -> Result<Account, String> {
    let mut account = {
        let mut store = state
            .accounts
            .lock()
            .map_err(|_| "account store lock failed".to_string())?;
        let mut should_save = false;
        let account = {
            let stored = store
                .accounts
                .iter_mut()
                .find(|account| account.id == id)
                .ok_or_else(|| "账号不存在".to_string())?;
            if stored
                .machine_id
                .as_ref()
                .is_none_or(|machine_id| machine_id.trim().is_empty())
            {
                ensure_account_machine_id(stored);
                should_save = true;
            }
            stored.clone()
        };
        if should_save {
            save_store(&store)?;
        }
        account
    };

    let refresh = refresh_token_by_provider(&account).await?;
    apply_refreshed_account_tokens(&mut account, &refresh);
    if matches!(
        account.status.as_str(),
        "invalid" | "失效" | "已失效" | "Token已失效"
    ) {
        account.status = "active".to_string();
        account.enabled = true;
    }

    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let stored = store
        .accounts
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;
    *stored = account.clone();
    save_store(&store)?;
    Ok(account)
}

async fn server_verify_account(
    state: &AdminState,
    params: VerifyAccountParams,
) -> Result<VerifyAccountResponse, String> {
    let VerifyAccountParams {
        access_token: _,
        refresh_token,
        provider,
        client_id,
        client_secret,
        region,
    } = params;

    let mut account = {
        let store = state
            .accounts
            .lock()
            .map_err(|_| "account store lock failed".to_string())?;
        store
            .accounts
            .iter()
            .find(|account| account.refresh_token.as_ref() == Some(&refresh_token))
            .cloned()
            .unwrap_or_else(|| {
                if provider == "Enterprise" {
                    Account::new_enterprise(
                        "pending".to_string(),
                        "Kiro Enterprise 账号".to_string(),
                    )
                } else {
                    Account::new("pending".to_string(), format!("Kiro {provider} 账号"))
                }
            })
    };

    account.provider = Some(provider.clone());
    account.refresh_token = Some(refresh_token.clone());
    if provider == "BuilderId" || provider == "Enterprise" {
        account.auth_method = Some("IdC".to_string());
        if let Some(client_id) = client_id {
            account.client_id = Some(client_id);
        }
        if let Some(client_secret) = client_secret {
            account.client_secret = Some(client_secret);
        }
        if let Some(region) = region {
            account.region = Some(region);
        }
    }
    ensure_account_machine_id(&mut account);

    let refresh = refresh_token_by_provider(&account).await?;
    apply_refreshed_account_tokens(&mut account, &refresh);
    let access_token = account
        .access_token
        .clone()
        .ok_or_else(|| "No access token after refresh".to_string())?;
    let refresh_token = account
        .refresh_token
        .clone()
        .ok_or_else(|| "No refresh token after refresh".to_string())?;

    let usage = get_usage_by_account(&account, &access_token).await?;
    if let Some(profile_arn) = usage.profile_arn.clone() {
        account.profile_arn = Some(profile_arn);
    }
    let usage_data = usage.usage_data.clone();
    account.usage_data = Some(usage.usage_data);
    update_account_status(&mut account, usage.is_banned, usage.is_auth_error);
    if account.status == "active" {
        account.enabled = true;
    }

    {
        let mut store = state
            .accounts
            .lock()
            .map_err(|_| "account store lock failed".to_string())?;
        if let Some(stored) = store.accounts.iter_mut().find(|item| {
            item.id == account.id || item.refresh_token.as_ref() == Some(&refresh_token)
        }) {
            *stored = account;
            save_store(&store)?;
        }
    }

    Ok(VerifyAccountResponse {
        usage_data,
        access_token,
        refresh_token,
    })
}

async fn server_list_available_models(
    state: &AdminState,
    id: String,
    force_refresh: bool,
) -> Result<Value, String> {
    let account = {
        let mut store = state
            .accounts
            .lock()
            .map_err(|_| "account store lock failed".to_string())?;
        let mut should_save = false;
        let account = {
            let stored = store
                .accounts
                .iter_mut()
                .find(|account| account.id == id)
                .ok_or_else(|| "账号不存在".to_string())?;
            if stored
                .machine_id
                .as_ref()
                .is_none_or(|machine_id| machine_id.trim().is_empty())
            {
                ensure_account_machine_id(stored);
                should_save = true;
            }
            stored.clone()
        };
        if should_save {
            save_store(&store)?;
        }
        account
    };
    let access_token = account
        .access_token
        .clone()
        .ok_or_else(|| "账号缺少 access_token，请先刷新 Token".to_string())?;
    let result = fetch_all_available_models(&account, &access_token).await?;
    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    if let Some(stored) = store.accounts.iter_mut().find(|item| item.id == id) {
        if stored
            .profile_arn
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            stored.profile_arn = result.resolved_profile_arn.clone();
        }
        if !force_refresh {
            write_available_models_cache(stored, &result.response)?;
        }
        save_store(&store)?;
    }
    serde_json::to_value(result.response).map_err(|error| error.to_string())
}

fn server_update_account(
    state: &AdminState,
    params: UpdateAccountParams,
) -> Result<Account, String> {
    let mut store = state
        .accounts
        .lock()
        .map_err(|_| "account store lock failed".to_string())?;
    let account = store
        .accounts
        .iter_mut()
        .find(|account| account.id == params.id)
        .ok_or_else(|| "账号不存在".to_string())?;

    if let Some(label) = params.label {
        account.label = label;
    }
    if let Some(status) = params.status {
        account.status = status;
    }
    if let Some(access_token) = params.access_token {
        account.access_token = Some(access_token);
    }
    if let Some(refresh_token) = params.refresh_token {
        account.refresh_token = Some(refresh_token);
    }
    if let Some(client_id) = params.client_id {
        account.client_id = Some(client_id);
    }
    if let Some(client_secret) = params.client_secret {
        account.client_secret = Some(client_secret);
    }
    if let Some(machine_id) = params.machine_id {
        account.machine_id = Some(machine_id);
    }
    if let Some(added_at) = params.added_at {
        let trimmed = added_at.trim();
        if !trimmed.is_empty() {
            account.added_at = trimmed.to_string();
        }
    }
    if let Some(expires_at) = params.expires_at {
        let trimmed = expires_at.trim();
        account.expires_at = (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    if let Some(enabled) = params.enabled {
        account.enabled = enabled;
    }
    if let Some(proxy_config) = params.proxy_config {
        account.proxy_config = proxy_config.enabled.then_some(proxy_config);
        account.available_models_cache = None;
    }

    let result = account.clone();
    save_store(&store)?;
    Ok(result)
}

async fn invoke_command(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Path(command): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    if let Err(response) = require_auth(&headers, &state) {
        return response;
    }

    match command.as_str() {
        "show_main_window" | "logout" | "cancel_kiro_login" => Json(json!(null)).into_response(),
        "get_supported_providers" => {
            Json(vec!["Google", "Github", "BuilderId", "Enterprise"]).into_response()
        }
        "get_current_user" => Json(json!({
            "id": "server-admin",
            "email": "server-admin",
            "name": "Server Admin"
        }))
        .into_response(),
        "get_accounts" => list_accounts(headers, State(state)).await,
        "get_available_accounts" => {
            let store = match state.accounts.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "account store lock failed",
                    )
                }
            };
            Json(
                store
                    .get_available_accounts()
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .into_response()
        }
        "delete_account" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
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
            json_result(store.delete(&id))
        }
        "delete_account_remote" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
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
            json_result(store.delete(&id))
        }
        "delete_accounts" => {
            let ids = match arg_string_vec(&payload, "ids") {
                Ok(ids) => ids,
                Err(response) => return response,
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
            json_result(store.delete_many(&ids))
        }
        "update_account" => {
            let params_value = payload.get("params").cloned().unwrap_or(payload);
            let params = match serde_json::from_value::<UpdateAccountParams>(params_value) {
                Ok(params) => params,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            json_result(server_update_account(&state, params))
        }
        "sync_account" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            json_result(server_sync_account(&state, id).await)
        }
        "refresh_account_token" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            json_result(server_refresh_account_token(&state, id).await)
        }
        "verify_account" => {
            let params_value = payload.get("params").cloned().unwrap_or(payload);
            let params = match serde_json::from_value::<VerifyAccountParams>(params_value) {
                Ok(params) => params,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            json_result(server_verify_account(&state, params).await)
        }
        "list_available_models" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            let force_refresh = payload
                .get("forceRefresh")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            json_result(server_list_available_models(&state, id, force_refresh).await)
        }
        "import_accounts" => {
            let json_text = match arg_string(&payload, "json") {
                Ok(json) => json,
                Err(response) => return response,
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
            json_result(store.import_from_json(&json_text))
        }
        "add_account_by_social" => {
            let refresh_token = match arg_string(&payload, "refreshToken") {
                Ok(refresh_token) => refresh_token,
                Err(response) => return response,
            };
            let provider = payload
                .get("provider")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let machine_id = payload
                .get("machineId")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let access_token = payload
                .get("accessToken")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            json_result(
                server_add_account_by_social(
                    &state,
                    refresh_token,
                    provider,
                    machine_id,
                    access_token,
                )
                .await,
            )
        }
        "add_account_by_idc" => json_result(server_add_account_by_idc(&state, &payload).await),
        "export_accounts" => {
            let ids = payload
                .get("ids")
                .cloned()
                .map(serde_json::from_value::<Option<Vec<String>>>)
                .transpose()
                .unwrap_or(None)
                .flatten();
            let store = match state.accounts.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "account store lock failed",
                    )
                }
            };
            let accounts = match ids {
                Some(ids) => store
                    .accounts
                    .iter()
                    .filter(|account| ids.contains(&account.id))
                    .cloned()
                    .collect::<Vec<_>>(),
                None => store.accounts.clone(),
            };
            json_result(serde_json::to_string_pretty(&accounts).map_err(|e| e.to_string()))
        }
        "get_groups" => {
            let store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            Json(store.get_groups()).into_response()
        }
        "get_tags" => {
            let store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            Json(store.get_tags()).into_response()
        }
        "add_group" => {
            let name = match arg_string(&payload, "name") {
                Ok(name) => name,
                Err(response) => return response,
            };
            let color = payload
                .get("color")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.add_group(name, color))
        }
        "update_group" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let color = payload
                .get("color")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.update_group(&id, name, color))
        }
        "reorder_groups" => {
            let ids = match arg_string_vec(&payload, "ids") {
                Ok(ids) => ids,
                Err(response) => return response,
            };
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.reorder_groups(&ids))
        }
        "delete_group" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            {
                let mut accounts = match state.accounts.lock() {
                    Ok(store) => store,
                    Err(_) => {
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "account store lock failed",
                        )
                    }
                };
                for account in &mut accounts.accounts {
                    if account.group_id.as_deref() == Some(id.as_str()) {
                        account.group_id = None;
                    }
                }
                if let Err(error) = save_store(&accounts) {
                    return json_error(StatusCode::BAD_REQUEST, error);
                }
            }
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.delete_group(&id))
        }
        "add_tag" => {
            let name = match arg_string(&payload, "name") {
                Ok(name) => name,
                Err(response) => return response,
            };
            let color = match arg_string(&payload, "color") {
                Ok(color) => color,
                Err(response) => return response,
            };
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.add_tag(name, color))
        }
        "update_tag" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let color = payload
                .get("color")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.update_tag(&id, name, color))
        }
        "delete_tag" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            {
                let mut accounts = match state.accounts.lock() {
                    Ok(store) => store,
                    Err(_) => {
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "account store lock failed",
                        )
                    }
                };
                for account in &mut accounts.accounts {
                    account.tag_links.retain(|link| link.tag_id != id);
                }
                if let Err(error) = save_store(&accounts) {
                    return json_error(StatusCode::BAD_REQUEST, error);
                }
            }
            let mut store = match state.group_tags.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "group/tag store lock failed",
                    )
                }
            };
            json_result(store.delete_tag(&id))
        }
        "set_account_group" => {
            let account_id = match arg_string(&payload, "accountId") {
                Ok(account_id) => account_id,
                Err(response) => return response,
            };
            let group_id = payload
                .get("groupId")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let mut store = match state.accounts.lock() {
                Ok(store) => store,
                Err(_) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "account store lock failed",
                    )
                }
            };
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == account_id) {
                account.group_id = group_id;
                json_result(save_store(&store))
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        "set_account_tags" => {
            let account_id = match arg_string(&payload, "accountId") {
                Ok(account_id) => account_id,
                Err(response) => return response,
            };
            let tag_ids = match arg_string_vec(&payload, "tagIds") {
                Ok(tag_ids) => tag_ids,
                Err(response) => return response,
            };
            let tag_names = {
                let tags = match state.group_tags.lock() {
                    Ok(store) => store.get_tags(),
                    Err(_) => {
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "group/tag store lock failed",
                        )
                    }
                };
                tags.into_iter()
                    .map(|tag| (tag.id, tag.name))
                    .collect::<HashMap<_, _>>()
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
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == account_id) {
                account
                    .tag_links
                    .retain(|link| tag_ids.contains(&link.tag_id));
                let existing = account
                    .tag_links
                    .iter()
                    .map(|link| link.tag_id.clone())
                    .collect::<Vec<_>>();
                for tag_id in tag_ids {
                    if !existing.contains(&tag_id) {
                        account.tag_links.push(AccountTagLink::new(
                            tag_id.clone(),
                            tag_names.get(&tag_id).cloned(),
                        ));
                    }
                }
                json_result(save_store(&store))
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        "add_tag_to_account" => {
            let account_id = match arg_string(&payload, "accountId") {
                Ok(account_id) => account_id,
                Err(response) => return response,
            };
            let tag_id = match arg_string(&payload, "tagId") {
                Ok(tag_id) => tag_id,
                Err(response) => return response,
            };
            let tag_name = {
                let store = match state.group_tags.lock() {
                    Ok(store) => store,
                    Err(_) => {
                        return json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "group/tag store lock failed",
                        )
                    }
                };
                store
                    .get_tags()
                    .into_iter()
                    .find(|tag| tag.id == tag_id)
                    .map(|tag| tag.name)
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
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == account_id) {
                if !account.tag_links.iter().any(|link| link.tag_id == tag_id) {
                    account
                        .tag_links
                        .push(AccountTagLink::new(tag_id, tag_name));
                }
                json_result(save_store(&store))
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        "remove_tag_from_account" => {
            let account_id = match arg_string(&payload, "accountId") {
                Ok(account_id) => account_id,
                Err(response) => return response,
            };
            let tag_id = match arg_string(&payload, "tagId") {
                Ok(tag_id) => tag_id,
                Err(response) => return response,
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
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == account_id) {
                account.tag_links.retain(|link| link.tag_id != tag_id);
                json_result(save_store(&store))
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        "remove_account_tags" => {
            let account_id = match arg_string(&payload, "accountId") {
                Ok(account_id) => account_id,
                Err(response) => return response,
            };
            let tag_ids = match arg_string_vec(&payload, "tagIds") {
                Ok(tag_ids) => tag_ids,
                Err(response) => return response,
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
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == account_id) {
                account
                    .tag_links
                    .retain(|link| !tag_ids.contains(&link.tag_id));
                json_result(save_store(&store))
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        "get_gateway_config" => json_result(gateway::get_gateway_config()),
        "save_gateway_config" => {
            let config_value = payload.get("config").cloned().unwrap_or(payload);
            let config = match serde_json::from_value::<GatewayConfig>(config_value) {
                Ok(config) => config,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            json_result(gateway::save_gateway_config(&config))
        }
        "get_gateway_status" => Json(gateway_status(&state).await).into_response(),
        "start_gateway" | "stop_gateway" => Json(gateway_status(&state).await).into_response(),
        "get_gateway_log_dir" => Json(
            std::env::var("KAM_DATA_DIR")
                .map(|dir| format!("{}/gateway/logs", dir.trim_end_matches(['/', '\\'])))
                .unwrap_or_default(),
        )
        .into_response(),
        "get_gateway_request_logs" => {
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(500);
            Json(state.log_store.get_last(limit).await).into_response()
        }
        "get_gateway_request_stats" => Json(state.log_store.get_stats().await).into_response(),
        "get_gateway_model_stats" => Json(state.log_store.get_model_stats().await).into_response(),
        "get_gateway_endpoint_stats" => {
            Json(state.log_store.get_endpoint_stats().await).into_response()
        }
        "clear_gateway_request_logs" => {
            state.log_store.clear().await;
            Json(json!(null)).into_response()
        }
        "get_app_settings" => json_result(app_settings_cmd::get_app_settings_inner()),
        "save_app_settings" => {
            let updates = payload.get("settings").cloned().unwrap_or(payload);
            let mut current = app_settings_cmd::get_app_settings_inner().unwrap_or_default();
            if let Ok(update_value) = serde_json::to_value(&updates) {
                let mut current_value =
                    serde_json::to_value(&current).unwrap_or_else(|_| json!({}));
                if let (Some(current_obj), Some(update_obj)) =
                    (current_value.as_object_mut(), update_value.as_object())
                {
                    for (key, value) in update_obj {
                        current_obj.insert(key.clone(), value.clone());
                    }
                    if let Ok(merged) = serde_json::from_value::<AppSettings>(current_value) {
                        current = merged;
                    }
                }
            }
            json_result(app_settings_cmd::save_settings_to_file(&current).map(|_| current))
        }
        "get_usage_history" => json_result(app_settings_cmd::get_usage_history().await),
        "save_usage_history_entry" => {
            let entry_value = payload.get("entry").cloned().unwrap_or(payload);
            let entry = match serde_json::from_value(entry_value) {
                Ok(entry) => entry,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            json_result(app_settings_cmd::save_usage_history_entry(entry).await)
        }
        "get_kiro_settings" => match kiro_settings_cmd::get_kiro_settings().await {
            Ok(settings) => Json(settings).into_response(),
            Err(error) => Json(json!({ "unavailable": true, "message": error })).into_response(),
        },
        "list_workspaces" => {
            match SessionStorage::new().and_then(|storage| storage.list_workspaces()) {
                Ok(workspaces) => Json(workspaces).into_response(),
                Err(error) => Json(json!({ "unavailable": true, "message": error.to_string() }))
                    .into_response(),
            }
        }
        "list_sessions" => {
            let workspace_hash = match arg_string(&payload, "workspaceHash") {
                Ok(workspace_hash) => workspace_hash,
                Err(response) => return response,
            };
            match SessionStorage::new().and_then(|storage| storage.list_sessions(&workspace_hash)) {
                Ok(sessions) => Json(sessions).into_response(),
                Err(error) => Json(json!({ "unavailable": true, "message": error.to_string() }))
                    .into_response(),
            }
        }
        "load_session" => {
            let workspace_hash = match arg_string(&payload, "workspaceHash") {
                Ok(workspace_hash) => workspace_hash,
                Err(response) => return response,
            };
            let session_id = match arg_string(&payload, "sessionId") {
                Ok(session_id) => session_id,
                Err(response) => return response,
            };
            json_result(
                SessionStorage::new()
                    .and_then(|storage| storage.load_session(&workspace_hash, &session_id))
                    .map_err(|error| error.to_string()),
            )
        }
        "delete_session" => {
            let workspace_hash = match arg_string(&payload, "workspaceHash") {
                Ok(workspace_hash) => workspace_hash,
                Err(response) => return response,
            };
            let session_id = match arg_string(&payload, "sessionId") {
                Ok(session_id) => session_id,
                Err(response) => return response,
            };
            json_result(
                SessionStorage::new()
                    .and_then(|storage| storage.delete_session(&workspace_hash, &session_id))
                    .map_err(|error| error.to_string()),
            )
        }
        "delete_workspace" => {
            let workspace_hash = match arg_string(&payload, "workspaceHash") {
                Ok(workspace_hash) => workspace_hash,
                Err(response) => return response,
            };
            json_result(
                SessionStorage::new()
                    .and_then(|storage| storage.delete_workspace(&workspace_hash))
                    .map_err(|error| error.to_string()),
            )
        }
        "export_session" => {
            let workspace_hash = match arg_string(&payload, "workspaceHash") {
                Ok(workspace_hash) => workspace_hash,
                Err(response) => return response,
            };
            let session_id = match arg_string(&payload, "sessionId") {
                Ok(session_id) => session_id,
                Err(response) => return response,
            };
            let format = payload
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("markdown");
            let export_format = match format {
                "json" => crate::services::session_storage::ExportFormat::Json,
                "markdown" => crate::services::session_storage::ExportFormat::Markdown,
                _ => return json_error(StatusCode::BAD_REQUEST, "Invalid format"),
            };
            json_result(
                SessionStorage::new()
                    .and_then(|storage| {
                        storage.export_session(&workspace_hash, &session_id, export_format)
                    })
                    .map_err(|error| error.to_string()),
            )
        }
        "search_sessions" => {
            let query = match arg_string(&payload, "query") {
                Ok(query) => query.to_lowercase(),
                Err(response) => return response,
            };
            match SessionStorage::new() {
                Ok(storage) => {
                    let mut results = Vec::new();
                    if let Ok(workspaces) = storage.list_workspaces() {
                        for workspace in workspaces {
                            if let Ok(sessions) = storage.list_sessions(&workspace) {
                                results.extend(sessions.into_iter().filter(|session| {
                                    session.title.to_lowercase().contains(&query)
                                }));
                            }
                        }
                    }
                    Json(results).into_response()
                }
                Err(error) => Json(json!({ "unavailable": true, "message": error.to_string() }))
                    .into_response(),
            }
        }
        "get_app_data_dir" => {
            Json(std::env::var("KAM_DATA_DIR").unwrap_or_default()).into_response()
        }
        "test_account_proxy" => {
            let proxy_value = payload
                .get("proxyConfig")
                .cloned()
                .unwrap_or_else(|| payload.clone());
            let proxy_config = match serde_json::from_value::<AccountProxyConfig>(proxy_value) {
                Ok(proxy_config) => proxy_config,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            json_result(crate::commands::proxy_cmd::test_account_proxy(proxy_config).await)
        }
        "get_cache_config" | "get_cache_stats" => {
            Json(json!({ "unavailable": true })).into_response()
        }
        "clear_all_cache"
        | "cleanup_expired_cache"
        | "open_gateway_log_dir"
        | "open_app_data_dir" => Json(json!(null)).into_response(),
        "generate_machine_guid" => Json(generate_account_machine_id()).into_response(),
        "set_overage_status" => {
            let id = match arg_string(&payload, "id") {
                Ok(id) => id,
                Err(response) => return response,
            };
            let enabled = match arg_bool(&payload, "enabled") {
                Ok(enabled) => enabled,
                Err(response) => return response,
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
            if let Some(account) = store.accounts.iter_mut().find(|item| item.id == id) {
                let status = if enabled { "ENABLED" } else { "DISABLED" };
                account.usage_data.get_or_insert_with(|| json!({}))["overageConfiguration"] =
                    json!({ "overageStatus": status });
                let result = account.clone();
                match save_store(&store) {
                    Ok(()) => Json(result).into_response(),
                    Err(error) => json_error(StatusCode::BAD_REQUEST, error),
                }
            } else {
                json_error(StatusCode::NOT_FOUND, "账号不存在")
            }
        }
        unsupported => json_error(
            StatusCode::NOT_IMPLEMENTED,
            format!("server mode does not support command: {unsupported}"),
        ),
    }
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
            "promptCacheTargetPercent": config.prompt_cache_target_percent,
            "promptCacheTtlSecs": config.prompt_cache_ttl_secs,
            "promptCacheMaxEntries": config.prompt_cache_max_entries,
            "promptCacheIgnoreClientControl": config.prompt_cache_ignore_client_control
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
    if let Some(ttl) = payload.prompt_cache_ttl_secs {
        if !(30..=3600).contains(&ttl) {
            return json_error(
                StatusCode::BAD_REQUEST,
                "promptCacheTtlSecs must be from 30 to 3600",
            );
        }
    }
    if let Some(max_entries) = payload.prompt_cache_max_entries {
        if max_entries < 1 {
            return json_error(
                StatusCode::BAD_REQUEST,
                "promptCacheMaxEntries must be >= 1",
            );
        }
    }

    let mut config = match gateway::get_gateway_config() {
        Ok(config) => config,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    config.prompt_cache_target_percent = payload.prompt_cache_target_percent;
    if let Some(ttl) = payload.prompt_cache_ttl_secs {
        config.prompt_cache_ttl_secs = ttl;
    }
    if let Some(max_entries) = payload.prompt_cache_max_entries {
        config.prompt_cache_max_entries = max_entries;
    }
    if let Some(ignore) = payload.prompt_cache_ignore_client_control {
        config.prompt_cache_ignore_client_control = ignore;
    }

    match gateway::save_gateway_config(&config) {
        Ok(()) => Json(json!({
            "ok": true,
            "promptCacheTargetPercent": config.prompt_cache_target_percent,
            "promptCacheTtlSecs": config.prompt_cache_ttl_secs,
            "promptCacheMaxEntries": config.prompt_cache_max_entries,
            "promptCacheIgnoreClientControl": config.prompt_cache_ignore_client_control,
            "restartRequired": true
        }))
        .into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        idc_refresh_token_fallback_identity, merge_optional_identity, resolve_idc_account_identity,
        resolve_stored_idc_user_id,
    };
    use sha2::{Digest, Sha256};

    fn expected_refresh_token_fallback(refresh_token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(refresh_token.as_bytes());
        let digest = hasher.finalize();
        let prefix = digest[..3]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("kiro_{prefix}")
    }

    #[test]
    fn idc_identity_falls_back_to_refresh_token_hash() {
        let refresh_token = "refresh-token-without-identity";

        assert_eq!(
            resolve_idc_account_identity(None, None, refresh_token),
            expected_refresh_token_fallback(refresh_token)
        );
        assert_eq!(
            idc_refresh_token_fallback_identity(refresh_token),
            expected_refresh_token_fallback(refresh_token)
        );
    }

    #[test]
    fn idc_identity_prefers_email_then_user_id() {
        assert_eq!(
            resolve_idc_account_identity(
                Some("user@example.com".to_string()),
                Some("user-id".to_string()),
                "refresh-token",
            ),
            "user@example.com"
        );
        assert_eq!(
            resolve_idc_account_identity(None, Some("user-id".to_string()), "refresh-token"),
            "user-id"
        );
    }

    #[test]
    fn optional_identity_merge_keeps_usage_before_jwt() {
        assert_eq!(
            merge_optional_identity(
                Some("usage@example.com".to_string()),
                None,
                Some("jwt@example.com".to_string()),
                Some("jwt-sub".to_string()),
            ),
            (
                Some("usage@example.com".to_string()),
                Some("jwt-sub".to_string())
            )
        );
    }

    #[test]
    fn enterprise_idc_stores_display_fallback_as_user_id() {
        assert_eq!(
            resolve_stored_idc_user_id("Enterprise", "kiro_abcdef", None),
            Some("kiro_abcdef".to_string())
        );
        assert_eq!(
            resolve_stored_idc_user_id("Enterprise", "kiro_abcdef", Some("real-user".to_string())),
            Some("real-user".to_string())
        );
        assert_eq!(
            resolve_stored_idc_user_id("BuilderId", "kiro_abcdef", None),
            None
        );
    }
}
