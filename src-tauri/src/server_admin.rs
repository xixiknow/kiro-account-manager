use axum::{
    extract::{Query, State},
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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    auth::{auth_social, providers::SocialTokenResponse},
    clients::{
        aws_sso_client::{
            AWSSSOClient, ClientRegistration, DeviceAuthorizationResponse, TokenResponse,
        },
        kiro_auth_client::KiroAuthServiceClient,
    },
    commands::app_settings_cmd::{self, AppSettings},
    commands::common::{
        calc_expires_at, extract_user_info, extract_user_info_from_jwt, find_existing_account_idx,
        generate_account_machine_id, get_usage_by_provider_with_machine_id,
        resolve_idc_client_id_hash, save_store, update_account_status, KIRO_BUILDER_ID_START_URL,
    },
    commands::kiro_settings_cmd,
    core::account::{Account, AccountStore, GroupTagData, GroupTagStore},
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
        existing.profile_arn = token_response.profile_arn.clone();
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
        account.profile_arn = token_response.profile_arn.clone();
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
        existing.profile_arn = None;
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
        account.profile_arn = None;
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
