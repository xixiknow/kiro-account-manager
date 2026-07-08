// 应用自身设置命令 (存到 ~/.kiro-account-manager/app-settings.json)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: Option<String>,
    pub locale: Option<String>, // 界面语言
    pub lock_model: Option<bool>,
    pub locked_model: Option<String>,
    pub auto_refresh: Option<bool>,
    pub auto_refresh_interval: Option<i32>,
    pub browser_path: Option<String>,
    // 隐私模式：脱敏显示邮箱
    pub privacy_mode: Option<bool>,
    // 自动换号设置
    pub auto_switch_enabled: Option<bool>,
    pub auto_switch_threshold: Option<f64>,
    pub auto_switch_interval: Option<i32>,
    // Kiro IDE 开关设置（用户偏好）
    pub enable_codebase_indexing: Option<bool>,
    pub enable_tab_autocomplete: Option<bool>,
    pub usage_summary: Option<bool>,
    pub enable_debug_logs: Option<bool>,
    pub notify_action_required: Option<bool>,
    pub notify_failure: Option<bool>,
    pub notify_success: Option<bool>,
    pub notify_billing: Option<bool>,
    // 新增 Kiro IDE 设置
    pub trusted_tools: Option<Vec<String>>,
    pub reference_tracker: Option<bool>,
    pub configure_mcp: Option<String>,
    pub telemetry_content_collection: Option<bool>,
    pub telemetry_usage_analytics: Option<bool>,
    pub telemetry_edit_stats: Option<bool>,
    pub telemetry_feedback: Option<bool>,
    // Kiro IDE 自定义安装路径
    pub custom_kiro_path: Option<String>,
    // 关闭窗口时的行为
    pub close_to_tray: Option<bool>, // true=最小化到托盘, false=直接退出
    // 软件自身接口代理：followKiro=跟随 Kiro IDE 代理, disabled=强制直连
    pub app_proxy_mode: Option<String>,
}

// 兼容旧配置文件中的 redeem_server 字段（已废弃）
// 读取时忽略，不再写入

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Some("dark".to_string()),
            locale: Some("zh-CN".to_string()),
            lock_model: Some(false),
            locked_model: None,
            auto_refresh: Some(true),
            auto_refresh_interval: Some(50),
            browser_path: None,
            privacy_mode: Some(true), // 默认开启
            // 自动换号默认值
            auto_switch_enabled: Some(false),
            auto_switch_threshold: Some(1.0),
            auto_switch_interval: Some(5),
            // Kiro IDE 开关默认值
            enable_codebase_indexing: Some(true),
            enable_tab_autocomplete: Some(true),
            usage_summary: Some(true),
            enable_debug_logs: Some(false),
            notify_action_required: Some(true),
            notify_failure: Some(true),
            notify_success: Some(true),
            notify_billing: Some(true),
            trusted_tools: None,
            reference_tracker: Some(false),
            configure_mcp: Some("Enabled".to_string()),
            telemetry_content_collection: Some(false),
            telemetry_usage_analytics: Some(false),
            telemetry_edit_stats: Some(false),
            telemetry_feedback: Some(false),
            custom_kiro_path: None,
            close_to_tray: Some(false), // 默认直接退出，由用户主动开启最小化到托盘
            app_proxy_mode: Some("followKiro".to_string()),
        }
    }
}
impl AppSettings {
    fn apply_updates(&mut self, updates: Self) {
        macro_rules! apply_if_some {
            ($field:ident) => {
                if updates.$field.is_some() {
                    self.$field = updates.$field;
                }
            };
        }

        apply_if_some!(theme);
        apply_if_some!(locale);
        apply_if_some!(lock_model);
        apply_if_some!(locked_model);
        apply_if_some!(auto_refresh);
        apply_if_some!(auto_refresh_interval);
        apply_if_some!(browser_path);
        apply_if_some!(privacy_mode);
        apply_if_some!(auto_switch_enabled);
        apply_if_some!(auto_switch_threshold);
        apply_if_some!(auto_switch_interval);
        apply_if_some!(enable_codebase_indexing);
        apply_if_some!(enable_tab_autocomplete);
        apply_if_some!(usage_summary);
        apply_if_some!(enable_debug_logs);
        apply_if_some!(notify_action_required);
        apply_if_some!(notify_failure);
        apply_if_some!(notify_success);
        apply_if_some!(notify_billing);
        apply_if_some!(trusted_tools);
        apply_if_some!(reference_tracker);
        apply_if_some!(configure_mcp);
        apply_if_some!(telemetry_content_collection);
        apply_if_some!(telemetry_usage_analytics);
        apply_if_some!(telemetry_edit_stats);
        apply_if_some!(telemetry_feedback);
        apply_if_some!(custom_kiro_path);
        apply_if_some!(close_to_tray);
        apply_if_some!(app_proxy_mode);
    }
}

fn get_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("KAM_DATA_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::data_dir()
        .unwrap_or_else(|| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
        })
        .join(".kiro-account-manager")
}

fn get_app_settings_path() -> PathBuf {
    get_data_dir().join("app-settings.json")
}

fn ensure_parent_dir(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    Ok(())
}

async fn run_blocking_io<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

pub fn get_app_settings_inner() -> Result<AppSettings, String> {
    let path = get_app_settings_path();
    if !path.exists() {
        // 首次启动：创建并保存默认值
        let default_settings = AppSettings::default();
        save_settings_to_file(&default_settings)?;
        return Ok(default_settings);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取设置失败: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析设置失败: {e}"))
}

pub fn save_settings_to_file(settings: &AppSettings) -> Result<(), String> {
    let path = get_app_settings_path();
    ensure_parent_dir(&path)?;
    let content = serde_json::to_string_pretty(settings).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {e}"))
}

fn save_app_settings_inner(updates: AppSettings) -> Result<(), String> {
    let mut current = get_app_settings_inner().unwrap_or_default();

    current.apply_updates(updates);

    save_settings_to_file(&current)
}

#[tauri::command]
pub async fn get_app_settings() -> Result<AppSettings, String> {
    run_blocking_io(get_app_settings_inner).await
}

#[tauri::command]
pub async fn save_app_settings(settings: AppSettings) -> Result<(), String> {
    run_blocking_io(move || save_app_settings_inner(settings)).await
}

/// 获取自定义浏览器路径（供打开浏览器时使用）
pub fn get_browser_path() -> Option<String> {
    get_app_settings_inner()
        .ok()
        .and_then(|s| s.browser_path)
        .filter(|p| !p.is_empty())
}

// ============================================================
// 使用量历史记录功能
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryEntry {
    pub date: String, // YYYY-MM-DD
    pub total_quota: i32,
    pub total_used: i32,
    pub account_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistory {
    pub entries: Vec<UsageHistoryEntry>,
}

fn get_usage_history_path() -> PathBuf {
    get_data_dir().join("usage-history.json")
}

fn get_usage_history_inner() -> Result<UsageHistory, String> {
    let path = get_usage_history_path();
    if !path.exists() {
        return Ok(UsageHistory::default());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取历史记录失败: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析历史记录失败: {e}"))
}

fn merge_usage_history_entry(history: &mut UsageHistory, entry: UsageHistoryEntry) {
    // 如果当天已有记录，则更新；否则添加新记录
    if let Some(existing) = history.entries.iter_mut().find(|e| e.date == entry.date) {
        existing.total_quota = entry.total_quota;
        existing.total_used = entry.total_used;
        existing.account_count = entry.account_count;
    } else {
        history.entries.push(entry);
    }

    // 只保留最近 30 天的记录
    history.entries.sort_by(|a, b| a.date.cmp(&b.date));
    if history.entries.len() > 30 {
        let skip_count = history.entries.len() - 30;
        history.entries.drain(..skip_count);
    }
}

fn save_usage_history_entry_inner(entry: UsageHistoryEntry) -> Result<(), String> {
    let path = get_usage_history_path();
    ensure_parent_dir(&path)?;

    let mut history = get_usage_history_inner().unwrap_or_default();
    merge_usage_history_entry(&mut history, entry);

    let content = serde_json::to_string_pretty(&history).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn get_usage_history() -> Result<UsageHistory, String> {
    run_blocking_io(get_usage_history_inner).await
}

#[tauri::command]
pub async fn save_usage_history_entry(entry: UsageHistoryEntry) -> Result<(), String> {
    run_blocking_io(move || save_usage_history_entry_inner(entry)).await
}

// ============================================================
// 自定义 Kiro 安装路径
// ============================================================

#[tauri::command]
pub async fn get_custom_kiro_path() -> Result<Option<String>, String> {
    run_blocking_io(|| get_app_settings_inner().map(|s| s.custom_kiro_path)).await
}

#[tauri::command]
pub async fn set_custom_kiro_path(path: String) -> Result<(), String> {
    run_blocking_io(move || {
        save_app_settings_inner(AppSettings {
            custom_kiro_path: Some(path),
            ..Default::default()
        })
    })
    .await
}

#[tauri::command]
pub async fn clear_custom_kiro_path() -> Result<(), String> {
    run_blocking_io(|| {
        save_app_settings_inner(AppSettings {
            custom_kiro_path: Some(String::new()),
            ..Default::default()
        })
    })
    .await
}
