#![allow(clippy::needless_pass_by_value)] // Tauri 命令需要按值传参
use tauri::{AppHandle, State};

use crate::core::account::Account;
use crate::gateway::{
    clear_gateway_request_logs as clear_gateway_request_logs_inner,
    get_gateway_config as get_gateway_config_inner,
    get_gateway_endpoint_stats as get_gateway_endpoint_stats_inner,
    get_gateway_log_dir as get_gateway_log_dir_inner,
    get_gateway_model_stats as get_gateway_model_stats_inner,
    get_gateway_request_logs as get_gateway_request_logs_inner,
    get_gateway_request_stats as get_gateway_request_stats_inner,
    get_gateway_status as get_gateway_status_inner, load_balancer, log_store,
    open_gateway_log_dir as open_gateway_log_dir_inner,
    save_gateway_config as save_gateway_config_inner, start_gateway as start_gateway_inner,
    stop_gateway as stop_gateway_inner, GatewayConfig, GatewayRequestLogEntry, GatewayRequestStats,
    GatewayStatus,
};
use crate::state::AppState;
use std::collections::HashMap;

fn config_for_manual_start(config: &GatewayConfig) -> GatewayConfig {
    config.clone()
}

#[cfg(test)]
fn config_after_manual_stop(config: &GatewayConfig) -> GatewayConfig {
    config.clone()
}

#[tauri::command]
pub async fn start_gateway(
    state: State<'_, AppState>,
    config: GatewayConfig,
) -> Result<GatewayStatus, String> {
    start_gateway_inner(&state, config_for_manual_start(&config)).await
}

#[tauri::command]
pub async fn stop_gateway(state: State<'_, AppState>) -> Result<(), String> {
    stop_gateway_inner(&state).await
}

#[tauri::command]
pub async fn get_gateway_status(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    get_gateway_status_inner(&state).await
}

#[tauri::command]
pub async fn get_gateway_config() -> Result<GatewayConfig, String> {
    get_gateway_config_inner()
}

#[tauri::command]
pub async fn save_gateway_config(config: GatewayConfig) -> Result<(), String> {
    save_gateway_config_inner(&config)
}

#[tauri::command]
pub async fn get_gateway_log_dir(app: AppHandle) -> Result<String, String> {
    get_gateway_log_dir_inner(&app)
}

#[tauri::command]
pub async fn get_gateway_request_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<GatewayRequestLogEntry>, String> {
    get_gateway_request_logs_inner(&state, limit).await
}

#[tauri::command]
pub async fn get_gateway_request_stats(
    state: State<'_, AppState>,
) -> Result<GatewayRequestStats, String> {
    get_gateway_request_stats_inner(&state).await
}

#[tauri::command]
pub async fn get_gateway_model_stats(
    state: State<'_, AppState>,
) -> Result<Vec<log_store::ModelStat>, String> {
    get_gateway_model_stats_inner(&state).await
}

#[tauri::command]
pub async fn get_gateway_endpoint_stats(
    state: State<'_, AppState>,
) -> Result<Vec<log_store::EndpointStat>, String> {
    get_gateway_endpoint_stats_inner(&state).await
}

#[tauri::command]
pub async fn open_gateway_log_dir(app: AppHandle) -> Result<String, String> {
    open_gateway_log_dir_inner(&app)
}

#[tauri::command]
pub async fn clear_gateway_request_logs(state: State<'_, AppState>) -> Result<(), String> {
    clear_gateway_request_logs_inner(&state).await
}

/// 一键配置2API客户端（Claude Code / Codex）
#[tauri::command]
pub async fn configure_proxy_clients(
    clients: Vec<String>,
    host: String,
    port: u16,
    api_key: String,
) -> Result<Vec<ProxyClientConfigResult>, String> {
    let mut results = Vec::new();
    let proxy_origin = if host == "0.0.0.0" || host == "::" {
        format!("http://127.0.0.1:{}", port)
    } else {
        format!("http://{}:{}", host, port)
    };
    let openai_base_url = format!("{}/v1", proxy_origin);

    for client in &clients {
        let result = match client.as_str() {
            "claudeCode" => configure_claude_code(&proxy_origin, &api_key),
            "codex" => configure_codex(&openai_base_url, &api_key),
            _ => Err(format!("不支持的客户端: {}", client)),
        };
        results.push(ProxyClientConfigResult {
            client: client.clone(),
            success: result.is_ok(),
            paths: result.as_ref().map(|p| p.clone()).unwrap_or_default(),
            error: result.err(),
        });
    }

    Ok(results)
}

#[derive(serde::Serialize)]
pub struct ProxyClientConfigResult {
    pub client: String,
    pub success: bool,
    pub paths: Vec<String>,
    pub error: Option<String>,
}

/// 配置 Claude Code: ~/.claude/settings.json
fn configure_claude_code(proxy_origin: &str, api_key: &str) -> Result<Vec<String>, String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "无法获取用户主目录".to_string())?;

    let settings_path = std::path::Path::new(&home)
        .join(".claude")
        .join("settings.json");
    let legacy_path = std::path::Path::new(&home)
        .join(".claude")
        .join("claude.json");

    // 选择正确的配置文件路径
    let path = if settings_path.exists() || !legacy_path.exists() {
        &settings_path
    } else {
        &legacy_path
    };

    // 确保目录存在
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    // 备份原文件
    let mut written_paths = Vec::new();
    if path.exists() {
        let backup_path = format!(
            "{}.kiro-backup-{}",
            path.display(),
            chrono::Local::now().format("%Y%m%d%H%M%S")
        );
        std::fs::copy(path, &backup_path).map_err(|e| format!("备份失败: {}", e))?;
        written_paths.push(backup_path);
    }

    // 读取或创建配置
    let mut config: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path).map_err(|e| format!("读取配置失败: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // 确保 env 字段存在
    if !config.get("env").is_some() {
        config["env"] = serde_json::json!({});
    }

    let env = config["env"]
        .as_object_mut()
        .ok_or("env 字段不是对象".to_string())?;

    // 写入2API配置
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        serde_json::Value::String(proxy_origin.to_string()),
    );
    env.insert(
        "ANTHROPIC_API_KEY".to_string(),
        serde_json::Value::String(api_key.to_string()),
    );

    // 写入文件
    let content =
        serde_json::to_string_pretty(&config).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(path, &content).map_err(|e| format!("写入失败: {}", e))?;

    written_paths.insert(0, path.display().to_string());
    Ok(written_paths)
}

/// 配置 Codex: ~/.codex/auth.json + ~/.codex/config.toml
fn configure_codex(openai_base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "无法获取用户主目录".to_string())?;

    let codex_dir = std::path::Path::new(&home).join(".codex");
    let auth_path = codex_dir.join("auth.json");
    let config_path = codex_dir.join("config.toml");

    // 确保目录存在
    std::fs::create_dir_all(&codex_dir).map_err(|e| format!("创建 .codex 目录失败: {}", e))?;

    let mut written_paths = Vec::new();
    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");

    // 1. 写入 auth.json
    if auth_path.exists() {
        let backup = format!("{}.kiro-backup-{}", auth_path.display(), timestamp);
        std::fs::copy(&auth_path, &backup).map_err(|e| format!("备份 auth.json 失败: {}", e))?;
    }

    let mut auth: serde_json::Value = if auth_path.exists() {
        let content = std::fs::read_to_string(&auth_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = auth.as_object_mut() {
        obj.insert(
            "OPENAI_API_KEY".to_string(),
            serde_json::Value::String(api_key.to_string()),
        );
    }

    let auth_content =
        serde_json::to_string_pretty(&auth).map_err(|e| format!("序列化 auth.json 失败: {}", e))?;
    std::fs::write(&auth_path, &auth_content).map_err(|e| format!("写入 auth.json 失败: {}", e))?;
    written_paths.push(auth_path.display().to_string());

    // 2. 写入 config.toml
    if config_path.exists() {
        let backup = format!("{}.kiro-backup-{}", config_path.display(), timestamp);
        std::fs::copy(&config_path, &backup)
            .map_err(|e| format!("备份 config.toml 失败: {}", e))?;
    }

    let existing_toml = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    // 构建新的 config.toml 内容
    let new_toml = build_codex_config_toml(&existing_toml, openai_base_url);
    std::fs::write(&config_path, &new_toml).map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    written_paths.push(config_path.display().to_string());

    Ok(written_paths)
}

/// 构建 Codex config.toml 内容
fn build_codex_config_toml(existing: &str, openai_base_url: &str) -> String {
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: Vec<&str> = existing.lines().collect();
    let mut result: Vec<String> = Vec::new();
    let mut in_custom_section = false;
    let mut base_url_replaced = false;

    for line in &lines {
        let trimmed = line.trim();

        // 进入 [model_providers.custom] section
        if trimmed == "[model_providers.custom]" || trimmed == "[model_providers.\"custom\"]" {
            in_custom_section = true;
            result.push(line.to_string());
            continue;
        }

        // 离开 custom section（遇到下一个 section）
        if in_custom_section && trimmed.starts_with('[') {
            in_custom_section = false;
        }

        // 在 custom section 内替换 base_url
        if in_custom_section && trimmed.starts_with("base_url") {
            result.push(format!("base_url = \"{}\"", openai_base_url));
            base_url_replaced = true;
            continue;
        }

        result.push(line.to_string());
    }

    // 如果没找到 [model_providers.custom]，追加一个
    if !base_url_replaced {
        result.push(String::new());
        result.push("[model_providers.custom]".to_string());
        result.push("name = \"custom\"".to_string());
        result.push(format!("base_url = \"{}\"", openai_base_url));
        result.push("wire_api = \"responses\"".to_string());
        result.push("requires_openai_auth = true".to_string());
    }

    result.join(newline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_start_preserves_auto_start_preference() {
        let config = GatewayConfig {
            enabled: false,
            ..GatewayConfig::default()
        };

        let next = config_for_manual_start(&config);

        assert!(
            !next.enabled,
            "manual start should not force auto-start preference on"
        );
    }

    #[test]
    fn manual_stop_preserves_auto_start_preference() {
        let config = GatewayConfig {
            enabled: true,
            ..GatewayConfig::default()
        };

        let next = config_after_manual_stop(&config);

        assert!(
            next.enabled,
            "manual stop should not clear auto-start preference"
        );
    }
}

/// 获取所有被速率限制的账号
#[tauri::command]
pub async fn get_rate_limited_accounts(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        Ok(lb.get_rate_limited_accounts().await)
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 获取所有被封禁的账号
#[tauri::command]
pub async fn get_banned_accounts(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        Ok(lb.get_banned_accounts().await)
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 清除账号的封禁标记
#[tauri::command]
pub async fn clear_banned_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        lb.clear_banned(&account_id).await;
        Ok(())
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 清除账号的速率限制标记
#[tauri::command]
pub async fn clear_rate_limit_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        lb.clear_rate_limit(&account_id).await;
        Ok(())
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 获取所有账号的健康状态
#[tauri::command]
pub async fn get_all_account_health(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        let health_map = lb.get_all_health().await;
        // 转换为可序列化的格式
        let serializable: HashMap<String, load_balancer::SerializableAccountHealth> = health_map
            .iter()
            .map(|(k, v)| (k.clone(), v.into()))
            .collect();
        Ok(serde_json::to_value(serializable).unwrap_or(serde_json::Value::Null))
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 重置账号健康状态
#[tauri::command]
pub async fn reset_account_health(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        lb.reset_health(&account_id).await;
        Ok(())
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 清理过期的健康状态数据
#[tauri::command]
pub async fn cleanup_stale_health(state: State<'_, AppState>) -> Result<(), String> {
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    if let Some(lb) = balancer {
        lb.cleanup_stale_health().await;
        Ok(())
    } else {
        Err("Gateway not initialized".to_string())
    }
}

/// 测试路由结果
#[derive(serde::Serialize)]
pub struct RouteTestResult {
    pub matched_accounts: Vec<String>,
    pub selected_account: Option<String>,
    pub error: Option<String>,
}

/// 根据 2API 配置从账号库筛选可用账号（不依赖 Tauri State，供桌面端与 server-web 复用）
pub fn filter_matched_accounts(
    store: &crate::core::account::AccountStore,
    config: &GatewayConfig,
) -> Vec<Account> {
    match config.account_mode.as_str() {
        "single" => store
            .accounts
            .iter()
            .filter(|account| config.account_id.as_deref() == Some(account.id.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
        "group" => store
            .accounts
            .iter()
            .filter(|account| {
                config.group_id.as_deref() == account.group_id.as_deref()
                    && account.is_available()
                    && account.enabled
            })
            .cloned()
            .collect::<Vec<_>>(),
        "pool" => store
            .accounts
            .iter()
            .filter(|account| {
                config.pool_account_ids.contains(&account.id)
                    && account.is_available()
                    && account.enabled
            })
            .cloned()
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    }
}

/// 把匹配/选中的账号渲染成 `label (id)` 展示字符串
pub fn format_route_account(account: &Account) -> String {
    format!("{} ({})", account.label, account.id)
}

/// 测试路由配置
#[tauri::command]
pub async fn test_route_config(
    state: State<'_, AppState>,
    config: GatewayConfig,
) -> Result<RouteTestResult, String> {
    // 导入 AccountStore
    use crate::core::account::AccountStore;

    let mut store = AccountStore::new();
    store.reload();

    // 根据配置筛选账号
    let matched_accounts = filter_matched_accounts(&store, &config);

    if matched_accounts.is_empty() {
        return Ok(RouteTestResult {
            matched_accounts: Vec::new(),
            selected_account: None,
            error: Some("未找到符合2API配置的可用账号".to_string()),
        });
    }

    // 使用负载均衡器选择账号
    let balancer = {
        let gateway = state.gateway.lock().unwrap();
        gateway.as_ref().map(|g| g.load_balancer.clone())
    };

    let selected_account: Option<Account> = if let Some(lb) = balancer {
        lb.select_account(&matched_accounts).await
    } else {
        // Gateway 未初始化时使用默认策略（轮询）
        matched_accounts.first().cloned()
    };

    Ok(RouteTestResult {
        matched_accounts: matched_accounts.iter().map(format_route_account).collect(),
        selected_account: selected_account.as_ref().map(format_route_account),
        error: None,
    })
}
