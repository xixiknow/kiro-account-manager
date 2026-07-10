/// 响应缓存模块
///
/// 实现三层缓存架构：
/// 1. 增量内存缓存（Delta Cache）- 检测消息增量，小变化直接复用
/// 2. LRU 内存缓存 - 快速访问热点数据
/// 3. 持久化缓存（文件系统）- 跨会话保存
use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// 是否启用摘要缓存
    pub summary_cache_enabled: bool,
    /// 消息增量阈值（小于此值时复用缓存）
    pub summary_cache_min_delta_messages: usize,
    /// 字符增量阈值（小于此值时复用缓存）
    pub summary_cache_min_delta_chars: usize,
    /// 缓存最大有效期（秒）
    pub summary_cache_max_age_seconds: u64,
    /// LRU 缓存容量
    pub lru_cache_capacity: usize,
    /// 是否启用持久化缓存
    pub persistent_cache_enabled: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            summary_cache_enabled: true,
            summary_cache_min_delta_messages: 3,
            summary_cache_min_delta_chars: 4000,
            summary_cache_max_age_seconds: 180, // 3 分钟
            lru_cache_capacity: 1000,
            persistent_cache_enabled: true,
        }
    }
}

/// 缓存条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// 缓存的响应内容
    pub response: String,
    /// 输入 token 数
    pub input_tokens: i32,
    /// 输出 token 数
    pub output_tokens: i32,
    /// 缓存创建时间（Unix 时间戳）
    pub created_at: u64,
    /// 缓存过期时间（Unix 时间戳）
    pub expires_at: u64,
    /// 消息数量
    pub message_count: usize,
    /// 总字符数
    pub total_chars: usize,
}

impl CacheEntry {
    /// 检查缓存是否过期
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now >= self.expires_at
    }

    /// 检查是否可以复用（增量检查）
    pub fn can_reuse(
        &self,
        new_message_count: usize,
        new_total_chars: usize,
        config: &CacheConfig,
    ) -> bool {
        if self.is_expired() {
            return false;
        }

        let delta_messages = new_message_count.saturating_sub(self.message_count);
        let delta_chars = new_total_chars.saturating_sub(self.total_chars);

        delta_messages < config.summary_cache_min_delta_messages
            && delta_chars < config.summary_cache_min_delta_chars
    }
}

/// 三层缓存系统
#[derive(Debug)]
pub struct ResponseCache {
    /// 配置
    config: CacheConfig,
    /// 第一层：增量缓存（会话 ID -> 最近的缓存条目）
    delta_cache: HashMap<String, CacheEntry>,
    /// 第二层：LRU 缓存
    lru_cache: LruCache<String, CacheEntry>,
    /// 持久化缓存目录
    cache_dir: Option<PathBuf>,
}

impl ResponseCache {
    /// 创建新的响应缓存
    pub fn new(config: CacheConfig, cache_dir: Option<PathBuf>) -> Self {
        let lru_capacity = NonZeroUsize::new(config.lru_cache_capacity).unwrap();

        Self {
            config,
            delta_cache: HashMap::new(),
            lru_cache: LruCache::new(lru_capacity),
            cache_dir,
        }
    }

    pub fn update_config(&mut self, config: CacheConfig) {
        if self.config.lru_cache_capacity != config.lru_cache_capacity {
            let lru_capacity = NonZeroUsize::new(config.lru_cache_capacity).unwrap();
            self.lru_cache = LruCache::new(lru_capacity);
        }
        self.config = config;
    }

    #[cfg(test)]
    pub fn config_snapshot(&self) -> CacheConfig {
        self.config.clone()
    }

    /// 生成缓存键
    fn cache_key(session_id: &str, messages_hash: &str) -> String {
        format!("{}:{}", session_id, messages_hash)
    }

    /// 获取缓存（三层查找）
    pub fn get(
        &mut self,
        session_id: &str,
        messages_hash: &str,
        message_count: usize,
        total_chars: usize,
    ) -> Option<CacheEntry> {
        let key = Self::cache_key(session_id, messages_hash);

        // 第一层：增量缓存检查
        if self.config.summary_cache_enabled {
            if let Some(entry) = self.delta_cache.get(session_id) {
                if entry.can_reuse(message_count, total_chars, &self.config) {
                    return Some(entry.clone());
                }
            }
        }

        // 第二层：LRU 缓存
        if let Some(entry) = self.lru_cache.get(&key) {
            if !entry.is_expired() {
                // 更新增量缓存
                self.delta_cache
                    .insert(session_id.to_string(), entry.clone());
                return Some(entry.clone());
            } else {
                // 移除过期条目
                self.lru_cache.pop(&key);
            }
        }

        // 第三层：持久化缓存
        if self.config.persistent_cache_enabled {
            if let Some(entry) = self.load_from_disk(&key) {
                if !entry.is_expired() {
                    // 回填到内存缓存
                    self.lru_cache.put(key.clone(), entry.clone());
                    self.delta_cache
                        .insert(session_id.to_string(), entry.clone());
                    return Some(entry);
                } else {
                    // 删除过期的磁盘缓存
                    let _ = self.delete_from_disk(&key);
                }
            }
        }

        None
    }

    /// 保存缓存（三层写入）
    pub fn put(
        &mut self,
        session_id: &str,
        messages_hash: &str,
        response: String,
        input_tokens: i32,
        output_tokens: i32,
        message_count: usize,
        total_chars: usize,
    ) {
        let key = Self::cache_key(session_id, messages_hash);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = CacheEntry {
            response,
            input_tokens,
            output_tokens,
            created_at: now,
            expires_at: now + self.config.summary_cache_max_age_seconds,
            message_count,
            total_chars,
        };

        // 写入增量缓存
        self.delta_cache
            .insert(session_id.to_string(), entry.clone());

        // 写入 LRU 缓存
        self.lru_cache.put(key.clone(), entry.clone());

        // 写入持久化缓存
        if self.config.persistent_cache_enabled {
            let _ = self.save_to_disk(&key, &entry);
        }
    }

    /// 清除会话的增量缓存
    pub fn clear_session(&mut self, session_id: &str) {
        self.delta_cache.remove(session_id);
    }

    /// 清除所有缓存
    pub fn clear_all(&mut self) {
        self.delta_cache.clear();
        self.lru_cache.clear();

        if self.config.persistent_cache_enabled {
            if let Some(cache_dir) = &self.cache_dir {
                let _ = fs::remove_dir_all(cache_dir);
                let _ = fs::create_dir_all(cache_dir);
            }
        }
    }

    /// 从磁盘加载缓存
    fn load_from_disk(&self, key: &str) -> Option<CacheEntry> {
        let cache_dir = self.cache_dir.as_ref()?;
        let file_path = cache_dir.join(format!("{}.json", Self::sanitize_key(key)));

        if !file_path.exists() {
            return None;
        }

        // 安全检查：文件大小限制
        const MAX_CACHE_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
        let metadata = fs::metadata(&file_path).ok()?;
        if metadata.len() > MAX_CACHE_FILE_SIZE {
            log::warn!("[安全] 缓存文件过大: {} bytes", metadata.len());
            return None;
        }

        let content = fs::read_to_string(&file_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 保存缓存到磁盘
    fn save_to_disk(&self, key: &str, entry: &CacheEntry) -> Result<(), std::io::Error> {
        let cache_dir = self
            .cache_dir
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "缓存目录未设置"))?;

        fs::create_dir_all(cache_dir)?;

        let file_path = cache_dir.join(format!("{}.json", Self::sanitize_key(key)));
        let content = serde_json::to_string(entry)?;
        fs::write(file_path, content)?;

        Ok(())
    }

    /// 从磁盘删除缓存
    fn delete_from_disk(&self, key: &str) -> Result<(), std::io::Error> {
        let cache_dir = self
            .cache_dir
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "缓存目录未设置"))?;

        let file_path = cache_dir.join(format!("{}.json", Self::sanitize_key(key)));
        if file_path.exists() {
            fs::remove_file(file_path)?;
        }

        Ok(())
    }

    /// 清理过期的磁盘缓存
    pub fn cleanup_expired(&mut self) -> Result<usize, std::io::Error> {
        let cache_dir = self
            .cache_dir
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "缓存目录未设置"))?;

        if !cache_dir.exists() {
            return Ok(0);
        }

        let mut removed_count = 0;

        for entry in fs::read_dir(cache_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(cache_entry) = serde_json::from_str::<CacheEntry>(&content) {
                        if cache_entry.is_expired() {
                            fs::remove_file(&path)?;
                            removed_count += 1;
                        }
                    }
                }
            }
        }

        Ok(removed_count)
    }

    /// 获取缓存统计信息
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            delta_cache_size: self.delta_cache.len(),
            lru_cache_size: self.lru_cache.len(),
            persistent_cache_enabled: self.config.persistent_cache_enabled,
        }
    }

    /// 清理键名（移除不安全字符并限制长度）
    fn sanitize_key(key: &str) -> String {
        const MAX_KEY_LENGTH: usize = 255;
        let sanitized: String = key
            .chars()
            .take(MAX_KEY_LENGTH)
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        sanitized
    }
}

/// 缓存统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub delta_cache_size: usize,
    pub lru_cache_size: usize,
    pub persistent_cache_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_expiration() {
        let entry = CacheEntry {
            response: "test".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            created_at: 0,
            expires_at: 0,
            message_count: 5,
            total_chars: 1000,
        };

        assert!(entry.is_expired());
    }

    #[test]
    fn test_cache_entry_can_reuse() {
        let config = CacheConfig::default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = CacheEntry {
            response: "test".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            created_at: now,
            expires_at: now + 300,
            message_count: 5,
            total_chars: 1000,
        };

        // 小增量，应该可以复用
        assert!(entry.can_reuse(7, 2000, &config));

        // 大增量，不应该复用
        assert!(!entry.can_reuse(10, 6000, &config));
    }

    #[test]
    fn test_response_cache_basic() {
        let config = CacheConfig::default();
        let mut cache = ResponseCache::new(config, None);

        // 保存缓存
        cache.put(
            "session1",
            "hash1",
            "response1".to_string(),
            100,
            50,
            5,
            1000,
        );

        // 获取缓存
        let result = cache.get("session1", "hash1", 5, 1000);
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "response1");
    }

    #[test]
    fn test_response_cache_delta_reuse() {
        let config = CacheConfig::default();
        let mut cache = ResponseCache::new(config, None);

        // 保存缓存
        cache.put(
            "session1",
            "hash1",
            "response1".to_string(),
            100,
            50,
            5,
            1000,
        );

        // 小增量查询，应该命中增量缓存
        let result = cache.get("session1", "hash2", 7, 2000);
        assert!(result.is_some());
        assert_eq!(result.unwrap().response, "response1");
    }

    #[test]
    fn test_response_cache_lru_eviction() {
        let mut config = CacheConfig::default();
        config.lru_cache_capacity = 2;
        let mut cache = ResponseCache::new(config, None);

        // 添加 3 个条目
        cache.put("s1", "h1", "r1".to_string(), 100, 50, 5, 1000);
        cache.put("s2", "h2", "r2".to_string(), 100, 50, 5, 1000);
        cache.put("s3", "h3", "r3".to_string(), 100, 50, 5, 1000);

        // 第一个应该被驱逐
        let result = cache.get("s1", "h1", 5, 1000);
        assert!(result.is_none());

        // 后两个应该还在
        let result = cache.get("s2", "h2", 5, 1000);
        assert!(result.is_some());
    }
}
