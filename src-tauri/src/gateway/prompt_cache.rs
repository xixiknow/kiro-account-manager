//! Prompt Cache 模拟器
//! 在2API侧追踪 cache_control 断点，模拟 Anthropic 的 prompt caching 行为
//! 让 Claude Code 的 cache_control 字段产生实际效果的 usage 统计

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// 常量
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5 * 60); // 5 分钟
const ONE_HOUR_CACHE_TTL: Duration = Duration::from_secs(60 * 60); // 1 小时
const DEFAULT_MIN_CACHEABLE_TOKENS: usize = 1024;
const OPUS_MIN_CACHEABLE_TOKENS: usize = 4096;
pub const DEFAULT_STABLE_CACHE_TARGET_PERCENT: u16 = 90; // 理想稳态：约 90% 输入来自缓存，余下保留给最新上下文

/// 缓存使用统计
#[derive(Debug, Clone, Default)]
pub struct CacheUsage {
    pub cache_creation_input_tokens: usize,
    pub cache_read_input_tokens: usize,
}

/// 缓存断点
#[derive(Debug, Clone)]
struct CacheBreakpoint {
    fingerprint: [u8; 32],
    cumulative_tokens: usize,
    ttl: Duration,
}

/// 缓存 Profile（一次请求的缓存结构）
#[derive(Debug, Clone)]
pub struct CacheProfile {
    breakpoints: Vec<CacheBreakpoint>,
    total_input_tokens: usize,
    model: String,
}

/// 缓存条目
#[derive(Debug, Clone)]
struct CacheEntry {
    expires_at: Instant,
    ttl: Duration,
}

/// 可缓存的内容块
struct CacheableBlock {
    value: String,
    tokens: usize,
    ttl: Duration,
    is_message_end: bool,
}

/// Prompt Cache Tracker（全局单例）
pub struct PromptCacheTracker {
    entries_by_account: Mutex<HashMap<String, HashMap<[u8; 32], CacheEntry>>>,
}

impl PromptCacheTracker {
    pub fn new() -> Self {
        Self {
            entries_by_account: Mutex::new(HashMap::new()),
        }
    }

    /// 从 Anthropic 格式请求构建缓存 profile
    pub fn build_profile(
        &self,
        system: Option<&serde_json::Value>,
        messages: &[serde_json::Value],
        tools: Option<&[serde_json::Value]>,
        total_input_tokens: usize,
        model: &str,
        ttl_override: Option<Duration>,
        ignore_client_control: bool,
    ) -> Option<CacheProfile> {
        let blocks =
            self.flatten_cache_blocks(system, messages, tools, ttl_override, ignore_client_control);
        if blocks.is_empty() {
            return None;
        }

        let mut hasher = Sha256::new();
        let mut breakpoints = Vec::new();
        let mut cumulative_tokens = 0usize;
        let mut active_ttl = Duration::ZERO;

        for block in &blocks {
            self.hash_chunk(&mut hasher, &block.value);
            cumulative_tokens += block.tokens;

            let breakpoint_ttl = if block.ttl > Duration::ZERO {
                active_ttl = block.ttl;
                block.ttl
            } else if block.is_message_end && active_ttl > Duration::ZERO {
                active_ttl
            } else {
                Duration::ZERO
            };

            if breakpoint_ttl == Duration::ZERO {
                continue;
            }

            let fingerprint: [u8; 32] = hasher.clone().finalize().into();
            breakpoints.push(CacheBreakpoint {
                fingerprint,
                cumulative_tokens,
                ttl: breakpoint_ttl,
            });
        }

        if breakpoints.is_empty() {
            return None;
        }

        Some(CacheProfile {
            breakpoints,
            total_input_tokens: total_input_tokens.max(cumulative_tokens),
            model: model.to_string(),
        })
    }

    /// 计算缓存命中情况
    #[allow(dead_code)]
    pub fn compute(&self, account_id: &str, profile: &CacheProfile) -> CacheUsage {
        self.compute_with_target_percent(account_id, profile, DEFAULT_STABLE_CACHE_TARGET_PERCENT)
    }

    /// 按指定目标百分比计算缓存命中情况
    pub fn compute_with_target_percent(
        &self,
        account_id: &str,
        profile: &CacheProfile,
        target_percent: u16,
    ) -> CacheUsage {
        if profile.breakpoints.is_empty() || account_id.is_empty() {
            return CacheUsage::default();
        }

        let min_tokens = self.min_cacheable_tokens(&profile.model);
        let last = &profile.breakpoints[profile.breakpoints.len() - 1];
        let mut last_tokens = last.cumulative_tokens.min(profile.total_input_tokens);
        let now = Instant::now();

        let mut entries_map = self
            .entries_by_account
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.prune_expired(&mut entries_map, now);

        let entries = entries_map.get_mut(account_id);
        if entries.is_none() || entries.as_ref().unwrap().is_empty() {
            // 首次请求：全部是 creation
            let effective_creation = if last_tokens >= min_tokens {
                last_tokens
            } else {
                0
            };
            return CacheUsage {
                cache_creation_input_tokens: effective_creation,
                cache_read_input_tokens: 0,
            };
        }

        let entries = entries.unwrap();

        // 稳态命中目标：可复用前缀按配置百分比折算，最新上下文保持为新写入。
        let target_percent = usize::from(target_percent.min(100));
        let max_cacheable = profile.total_input_tokens.saturating_mul(target_percent) / 100;
        if last_tokens > max_cacheable {
            last_tokens = max_cacheable;
        }

        // 从后往前匹配最长前缀
        let mut matched_tokens = 0usize;
        for bp in profile.breakpoints.iter().rev() {
            if bp.cumulative_tokens < min_tokens {
                continue;
            }
            if let Some(entry) = entries.get_mut(&bp.fingerprint) {
                if entry.expires_at > now {
                    // 命中：刷新过期时间
                    entry.expires_at = now + entry.ttl;
                    matched_tokens = bp.cumulative_tokens.min(profile.total_input_tokens);
                    if matched_tokens > last_tokens {
                        matched_tokens = last_tokens;
                    }
                    break;
                }
            }
        }

        let creation = last_tokens.saturating_sub(matched_tokens);
        CacheUsage {
            cache_creation_input_tokens: creation,
            cache_read_input_tokens: matched_tokens,
        }
    }

    /// 更新缓存条目（请求成功后调用）
    pub fn update(&self, account_id: &str, profile: &CacheProfile, max_entries: usize) {
        if profile.breakpoints.is_empty() || account_id.is_empty() {
            return;
        }
        let max_entries = max_entries.max(1);

        let min_tokens = self.min_cacheable_tokens(&profile.model);
        let now = Instant::now();

        let mut entries_map = self
            .entries_by_account
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entries = entries_map.entry(account_id.to_string()).or_default();

        for bp in &profile.breakpoints {
            if bp.cumulative_tokens < min_tokens {
                continue;
            }
            entries.insert(
                bp.fingerprint,
                CacheEntry {
                    expires_at: now + bp.ttl,
                    ttl: bp.ttl,
                },
            );
        }

        // 限制条目数
        if entries.len() > max_entries {
            let mut sorted: Vec<_> = entries.iter().map(|(k, v)| (*k, v.expires_at)).collect();
            sorted.sort_by_key(|(_, exp)| *exp);
            let to_remove = entries.len() - max_entries;
            for (key, _) in sorted.iter().take(to_remove) {
                entries.remove(key);
            }
        }
    }

    // ============ 内部方法 ============

    fn flatten_cache_blocks(
        &self,
        system: Option<&serde_json::Value>,
        messages: &[serde_json::Value],
        tools: Option<&[serde_json::Value]>,
        ttl_override: Option<Duration>,
        ignore_client_control: bool,
    ) -> Vec<CacheableBlock> {
        let mut blocks = Vec::new();
        // system/tools 自动缓存块使用的默认 TTL（可被配置覆盖）
        let default_ttl = ttl_override.unwrap_or(DEFAULT_CACHE_TTL);

        // 解析块的有效 TTL：
        // - ignore_client_control：忽略客户端 cache_control，统一用 default_ttl
        // - 否则：尊重客户端 extract_ttl，未标记时回退 fallback（system/tools 用 default_ttl，
        //   消息块用 Duration::ZERO 表示不缓存）
        let resolve_ttl = |value: &serde_json::Value, fallback: Duration| -> Duration {
            if ignore_client_control {
                return default_ttl;
            }
            let client_ttl = self.extract_ttl(value);
            if client_ttl > Duration::ZERO {
                client_ttl
            } else {
                fallback
            }
        };

        // 工具定义（自动可缓存）
        if let Some(tools) = tools {
            for tool in tools {
                let value = self.canonicalize(tool);
                let tokens = estimate_tokens(&value);
                blocks.push(CacheableBlock {
                    value,
                    tokens,
                    ttl: resolve_ttl(tool, default_ttl),
                    is_message_end: false,
                });
            }
        }

        // System prompt（自动可缓存）
        if let Some(system) = system {
            match system {
                serde_json::Value::String(s) => {
                    let tokens = estimate_tokens(s);
                    blocks.push(CacheableBlock {
                        value: self.canonicalize(system),
                        tokens,
                        ttl: default_ttl,
                        is_message_end: false,
                    });
                }
                serde_json::Value::Array(arr) => {
                    for block in arr {
                        let value = self.canonicalize(block);
                        let tokens = estimate_tokens(&value);
                        blocks.push(CacheableBlock {
                            value,
                            tokens,
                            ttl: resolve_ttl(block, default_ttl),
                            is_message_end: false,
                        });
                    }
                }
                _ => {}
            }
        }

        // Messages：默认只有显式标记 cache_control 的才可缓存；
        // ignore_client_control 开启时消息块也用统一 TTL 生成断点。
        for (i, msg) in messages.iter().enumerate() {
            let content = msg.get("content");
            let _is_last_msg = i == messages.len() - 1;

            match content {
                Some(serde_json::Value::String(s)) => {
                    let value = self.canonicalize(msg);
                    let tokens = estimate_tokens(s);
                    blocks.push(CacheableBlock {
                        value,
                        tokens,
                        ttl: resolve_ttl(msg, Duration::ZERO),
                        is_message_end: true,
                    });
                }
                Some(serde_json::Value::Array(arr)) => {
                    let last_idx = arr.len().saturating_sub(1);
                    for (j, block) in arr.iter().enumerate() {
                        let value = self.canonicalize(block);
                        let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        let tokens = estimate_tokens(if text.is_empty() { &value } else { text });
                        blocks.push(CacheableBlock {
                            value,
                            tokens,
                            ttl: resolve_ttl(block, Duration::ZERO),
                            is_message_end: j == last_idx,
                        });
                    }
                }
                _ => {}
            }
        }

        blocks
    }

    fn extract_ttl(&self, value: &serde_json::Value) -> Duration {
        let cache_control = value.get("cache_control");
        let Some(cc) = cache_control else {
            return Duration::ZERO;
        };
        let Some(cc_type) = cc.get("type").and_then(|t| t.as_str()) else {
            return Duration::ZERO;
        };
        if !cc_type.eq_ignore_ascii_case("ephemeral") {
            return Duration::ZERO;
        }
        // 检查 ttl 字段
        if let Some(ttl_val) = cc.get("ttl") {
            if let Some(s) = ttl_val.as_str() {
                if s == "1h" || s == "1H" {
                    return ONE_HOUR_CACHE_TTL;
                }
            }
            if let Some(n) = ttl_val.as_u64() {
                if n > 0 {
                    return Duration::from_secs(n);
                }
            }
        }
        DEFAULT_CACHE_TTL
    }

    fn canonicalize(&self, value: &serde_json::Value) -> String {
        // 排除 cache_control 字段后序列化
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted: Vec<_> = map
                    .iter()
                    .filter(|(k, _)| k.as_str() != "cache_control")
                    .collect();
                sorted.sort_by_key(|(k, _)| k.as_str());
                let obj: serde_json::Map<String, serde_json::Value> = sorted
                    .into_iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default()
            }
            _ => serde_json::to_string(value).unwrap_or_default(),
        }
    }

    fn hash_chunk(&self, hasher: &mut Sha256, chunk: &str) {
        hasher.update(chunk.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(chunk.as_bytes());
        hasher.update(b"\0");
    }

    fn min_cacheable_tokens(&self, model: &str) -> usize {
        if model.to_lowercase().contains("opus") {
            OPUS_MIN_CACHEABLE_TOKENS
        } else {
            DEFAULT_MIN_CACHEABLE_TOKENS
        }
    }

    fn prune_expired(
        &self,
        entries_map: &mut HashMap<String, HashMap<[u8; 32], CacheEntry>>,
        now: Instant,
    ) {
        entries_map.retain(|_, entries| {
            entries.retain(|_, entry| entry.expires_at > now);
            !entries.is_empty()
        });
    }
}

/// 估算 token 数（字符数 / 4）
fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
}

// 全局单例
static GLOBAL_TRACKER: std::sync::OnceLock<PromptCacheTracker> = std::sync::OnceLock::new();

pub fn global_prompt_cache_tracker() -> &'static PromptCacheTracker {
    GLOBAL_TRACKER.get_or_init(PromptCacheTracker::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_prompt_reports_ninety_percent_read_cache() {
        let tracker = PromptCacheTracker::new();
        let system = serde_json::Value::String("a".repeat(40_000));
        let profile = tracker
            .build_profile(
                Some(&system),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5-20250929",
                None,
                false,
            )
            .expect("system prompt should be cacheable");

        let first = tracker.compute("account-a", &profile);
        assert_eq!(first.cache_creation_input_tokens, 10_000);
        assert_eq!(first.cache_read_input_tokens, 0);

        tracker.update("account-a", &profile, 2000);

        let second = tracker.compute("account-a", &profile);
        assert_eq!(second.cache_read_input_tokens, 9_000);
        assert_eq!(second.cache_creation_input_tokens, 0);
    }

    #[test]
    fn repeated_prompt_honors_custom_target_percent() {
        let tracker = PromptCacheTracker::new();
        let system = serde_json::Value::String("a".repeat(40_000));
        let profile = tracker
            .build_profile(
                Some(&system),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5-20250929",
                None,
                false,
            )
            .expect("system prompt should be cacheable");

        tracker.update("account-a", &profile, 2000);

        let usage = tracker.compute_with_target_percent("account-a", &profile, 75);
        assert_eq!(usage.cache_read_input_tokens, 7_500);
        assert_eq!(usage.cache_creation_input_tokens, 0);
    }

    #[test]
    fn ignore_client_control_caches_unmarked_message() {
        // 无 cache_control 的长用户消息，在 ignore 模式下也应产生断点并命中
        let tracker = PromptCacheTracker::new();
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "a".repeat(40_000),
        })];

        // 关闭 ignore：消息块无标记 → 不缓存 → 无 profile
        assert!(tracker
            .build_profile(None, &messages, None, 10_000, "claude-sonnet-4-5", None, false)
            .is_none());

        // 开启 ignore：统一 TTL 生成断点
        let profile = tracker
            .build_profile(None, &messages, None, 10_000, "claude-sonnet-4-5", None, true)
            .expect("ignore mode should cache unmarked message");
        tracker.update("account-ic", &profile, 2000);
        let usage = tracker.compute("account-ic", &profile);
        assert_eq!(usage.cache_read_input_tokens, 9_000);
        assert_eq!(usage.cache_creation_input_tokens, 0);
    }

    #[test]
    fn short_ttl_expires_before_next_request() {
        // 短 TTL 写入后等待超过 TTL，第二次请求应 miss（验证 ttl_override 被真正应用）
        let tracker = PromptCacheTracker::new();
        let system = serde_json::Value::String("a".repeat(40_000));
        let profile = tracker
            .build_profile(
                Some(&system),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5-20250929",
                Some(Duration::from_millis(20)),
                false,
            )
            .expect("system prompt should be cacheable");

        tracker.update("account-ttl", &profile, 2000);
        std::thread::sleep(Duration::from_millis(40));
        // 条目已过期 → 第二次仍是 creation
        let usage = tracker.compute("account-ttl", &profile);
        assert_eq!(usage.cache_read_input_tokens, 0);
        assert_eq!(usage.cache_creation_input_tokens, 10_000);
    }

    #[test]
    fn long_ttl_renews_on_hit() {
        // 长 TTL：写入后立即再次请求应命中（滑动续期）
        let tracker = PromptCacheTracker::new();
        let system = serde_json::Value::String("a".repeat(40_000));
        let profile = tracker
            .build_profile(
                Some(&system),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5-20250929",
                Some(Duration::from_secs(3600)),
                false,
            )
            .expect("system prompt should be cacheable");

        tracker.update("account-ttl-long", &profile, 2000);
        let usage = tracker.compute("account-ttl-long", &profile);
        assert_eq!(usage.cache_read_input_tokens, 9_000);
        assert_eq!(usage.cache_creation_input_tokens, 0);
    }

    #[test]
    fn max_entries_evicts_oldest() {
        // max_entries=1 时，插入第二个模型的条目应淘汰最老的
        let tracker = PromptCacheTracker::new();
        let sys_a = serde_json::Value::String("a".repeat(40_000));
        let sys_b = serde_json::Value::String("b".repeat(40_000));
        // profile_a 用较短 TTL → expires_at 明确更早，淘汰顺序确定
        let profile_a = tracker
            .build_profile(
                Some(&sys_a),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5",
                Some(Duration::from_secs(60)),
                false,
            )
            .unwrap();
        let profile_b = tracker
            .build_profile(
                Some(&sys_b),
                &[],
                None,
                10_000,
                "claude-sonnet-4-5",
                Some(Duration::from_secs(600)),
                false,
            )
            .unwrap();

        tracker.update("account-cap", &profile_a, 1);
        tracker.update("account-cap", &profile_b, 1);

        // profile_a 应被淘汰 → miss
        let usage_a = tracker.compute("account-cap", &profile_a);
        assert_eq!(usage_a.cache_read_input_tokens, 0);
        // profile_b 仍在
        let usage_b = tracker.compute("account-cap", &profile_b);
        assert_eq!(usage_b.cache_read_input_tokens, 9_000);
    }
}
