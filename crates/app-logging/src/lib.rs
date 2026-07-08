/// 应用日志系统（对应设计 §12）
///
/// 三类日志：后端日志（tracing）/ LLM 调用日志 / 前端+插件日志
/// 统一收集到环形缓冲，ERROR + LLM 调用落盘，支持导出脱敏 bundle。
pub mod interceptor;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use storyforge_domain::Id;
use storyforge_domain::agent::AgentRole;

// ─── 数据模型（对应设计 §12.3）─────────────────────────────────────────────

/// 日志类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogKind {
    /// 后端日志（tracing）
    Backend,
    /// LLM 调用日志
    LlmCall,
    /// 前端+插件日志
    FrontendPlugin,
}

/// 日志级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// 单条日志
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Id,
    pub kind: LogKind,
    pub level: LogLevel,
    pub timestamp: DateTime<Utc>,
    pub message: String,
    /// 结构化字段（便于筛选）
    pub fields: HashMap<String, serde_json::Value>,
    /// LLM 调用专属详情
    pub llm_detail: Option<LlmCallDetail>,
}

/// LLM 调用详情（对应设计 §12.3）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallDetail {
    /// 连接名（不含 key）
    pub connection_name: String,
    pub model: String,
    pub profile_id: Option<Id>,
    pub agent_role: Option<AgentRole>,
    /// 完整请求 payload（含 prompt 正文）
    pub request_payload: String,
    /// 完整回复
    pub response_text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// 脱敏后的 LLM 调用详情（导出 bundle 用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallDetailRedacted {
    pub connection_name: String,
    pub model: String,
    pub profile_id: Option<Id>,
    pub agent_role: Option<AgentRole>,
    /// 正文替换为 <content N chars>
    pub request_payload_redacted: String,
    pub response_text_redacted: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
    pub error: Option<String>,
}

impl LlmCallDetail {
    /// 脱敏：正文替换为占位符
    pub fn redact(&self) -> LlmCallDetailRedacted {
        LlmCallDetailRedacted {
            connection_name: self.connection_name.clone(),
            model: self.model.clone(),
            profile_id: self.profile_id.clone(),
            agent_role: self.agent_role.clone(),
            request_payload_redacted: format!("<content {} chars>", self.request_payload.len()),
            response_text_redacted: format!("<content {} chars>", self.response_text.len()),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            latency_ms: self.latency_ms,
            error: self.error.clone(),
        }
    }
}

// ─── 日志过滤器 ────────────────────────────────────────────────────────────

/// 查询过滤器
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    pub kind: Option<LogKind>,
    pub level: Option<LogLevel>,
    pub keyword: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

// ─── 环形缓冲（对应设计 §12.4）─────────────────────────────────────────────

/// 每类日志的最大条数
const MAX_ENTRIES_PER_KIND: usize = 2000;

/// 环形缓冲（VecDeque：push/pop_front 均为 O(1)；历史用 Vec 导致 remove(0) 是 O(n)）
pub struct LogBuffer {
    entries: HashMap<LogKind, VecDeque<LogEntry>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        let mut entries = HashMap::new();
        entries.insert(LogKind::Backend, VecDeque::new());
        entries.insert(LogKind::LlmCall, VecDeque::new());
        entries.insert(LogKind::FrontendPlugin, VecDeque::new());
        Self { entries }
    }

    /// 追加日志（LRU 淘汰：超限移除最旧的）
    pub fn push(&mut self, entry: LogEntry) {
        if let Some(buf) = self.entries.get_mut(&entry.kind) {
            if buf.len() >= MAX_ENTRIES_PER_KIND {
                buf.pop_front(); // O(1)，旧实现 Vec::remove(0) 是 O(n)
            }
            buf.push_back(entry);
        }
    }

    /// 查询日志
    pub fn query(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let mut results = Vec::new();

        let kinds: Vec<LogKind> = if let Some(k) = filter.kind {
            vec![k]
        } else {
            vec![LogKind::Backend, LogKind::LlmCall, LogKind::FrontendPlugin]
        };

        for kind in kinds {
            if let Some(buf) = self.entries.get(&kind) {
                for entry in buf.iter().rev() {
                    // 级别过滤
                    if let Some(min_level) = filter.level
                        && entry.level < min_level
                    {
                        continue;
                    }
                    // 时间过滤
                    if let Some(since) = filter.since
                        && entry.timestamp < since
                    {
                        continue;
                    }
                    if let Some(until) = filter.until
                        && entry.timestamp > until
                    {
                        continue;
                    }
                    // 关键词过滤
                    if let Some(ref kw) = filter.keyword
                        && !entry.message.contains(kw.as_str())
                    {
                        continue;
                    }
                    results.push(entry.clone());

                    // 限制
                    if let Some(limit) = filter.limit
                        && results.len() >= limit
                    {
                        return results;
                    }
                }
            }
        }

        results
    }

    /// 获取单条 LLM 调用详情
    pub fn get_llm_call(&self, id: &Id) -> Option<&LogEntry> {
        self.entries
            .get(&LogKind::LlmCall)?
            .iter()
            .find(|e| &e.id == id)
    }

    /// 清空指定类型（或全部）
    pub fn clear(&mut self, kind: Option<LogKind>) {
        if let Some(k) = kind {
            if let Some(buf) = self.entries.get_mut(&k) {
                buf.clear();
            }
        } else {
            for buf in self.entries.values_mut() {
                buf.clear();
            }
        }
    }

    /// 获取各类日志条数
    pub fn counts(&self) -> HashMap<LogKind, usize> {
        self.entries.iter().map(|(k, v)| (*k, v.len())).collect()
    }
}

// ─── 日志 Store（环形缓冲 + 持久化）────────────────────────────────────────

/// 日志存储（环形缓冲 + 落盘）
pub struct LogStore {
    buffer: Mutex<LogBuffer>,
    persist_lock: Mutex<()>,
    /// 日志文件目录（落盘用）
    log_dir: PathBuf,
}

impl LogStore {
    pub fn new(log_dir: PathBuf) -> Self {
        // 确保目录存在
        std::fs::create_dir_all(&log_dir).ok();
        let mut buffer = LogBuffer::new();
        // P1-2 修复：启动时回填历史日志。原实现只建空内存缓冲，跨会话历史全丢，
        // 调试面板看不到之前的日志。这里读取 log_dir/*.jsonl 按行反序列化后 push，
        // 文件按日期升序读取（旧→新），保证最新条目最后入队、LRU 截断时保留最近。
        backfill_from_dir(&mut buffer, &log_dir);
        Self {
            buffer: Mutex::new(buffer),
            persist_lock: Mutex::new(()),
            log_dir,
        }
    }

    /// 追加日志（ERROR + LLM 调用自动落盘）
    pub fn push(&self, entry: LogEntry) {
        // 需要落盘的：ERROR 级别 + LLM 调用
        let should_persist = entry.level == LogLevel::Error || entry.kind == LogKind::LlmCall;

        let mut buf = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        buf.push(entry.clone());
        drop(buf);

        if should_persist {
            self.persist_entry(&entry);
        }
    }

    /// 查询日志
    pub fn query(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let buf = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        buf.query(filter)
    }

    /// 获取单条 LLM 调用详情
    pub fn get_llm_call(&self, id: &Id) -> Option<LlmCallDetail> {
        let buf = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        buf.get_llm_call(id).and_then(|e| e.llm_detail.clone())
    }

    /// 清空
    pub fn clear(&self, kind: Option<LogKind>) {
        let mut buf = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        buf.clear(kind);
    }

    /// 落盘到文件（按天滚动，JSONL 格式）
    fn persist_entry(&self, entry: &LogEntry) {
        let date = entry.timestamp.format("%Y-%m-%d").to_string();
        let file_path = self.log_dir.join(format!("{date}.jsonl"));
        let _persist_guard = self.persist_lock.lock().unwrap_or_else(|p| p.into_inner());

        if let Ok(json) = serde_json::to_string(entry) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)
                && let Err(e) = writeln!(f, "{json}")
            {
                eprintln!("Failed to write log entry: {e}");
            }
        }
    }

    /// 清理过期日志文件（保留最近 N 天）
    pub fn cleanup_old_files(&self, keep_days: u32) {
        let cutoff = Utc::now() - chrono::Duration::days(keep_days as i64);
        let cutoff_str = cutoff.format("%Y-%m-%d").to_string();

        if let Ok(entries) = std::fs::read_dir(&self.log_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // 文件名格式：YYYY-MM-DD.jsonl
                    if let Some(date_part) = name.strip_suffix(".jsonl")
                        && date_part < cutoff_str.as_str()
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

/// 启动时回填历史日志（P1-2）。
///
/// 读取 `log_dir/*.jsonl`，每行反序列化为 `LogEntry` 后 push 进 buffer。
/// 文件按文件名（=日期 YYYY-MM-DD）升序排序后依次读取，保证旧条目先入队、
/// 新条目后入队——LRU 截断（每类 MAX_ENTRIES_PER_KIND=2000）会淘汰最旧条目，
/// 从而保留最近会话的历史。解析失败的行跳过（容错）。
fn backfill_from_dir(buffer: &mut LogBuffer, log_dir: &std::path::Path) {
    let read_dir = match std::fs::read_dir(log_dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    // 收集 *.jsonl 文件名并按日期升序排序（文件名=YYYY-MM-DD，字典序=时间序）
    let mut files: Vec<String> = read_dir
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .filter(|n| n.ends_with(".jsonl"))
                .map(|s| s.to_string())
        })
        .collect();
    files.sort();
    for name in files {
        let path = log_dir.join(&name);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<LogEntry>(line) {
                Ok(entry) => buffer.push(entry),
                // 容错：损坏/不兼容的行跳过，不阻断回填
                Err(_) => continue,
            }
        }
    }
}

// ─── 导出 bundle（对应设计 §12.5）─────────────────────────────────────────

/// 导出选项
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// 隐藏写作正文（替换为占位符）
    pub redact_content: bool,
    /// 隐藏角色卡/世界书内容
    pub redact_character: bool,
    /// 隐藏连接名
    pub redact_connection: bool,
}

fn log_entry_to_value(entry: &LogEntry) -> Result<serde_json::Value, String> {
    serde_json::to_value(entry).map_err(|e| format!("日志条目序列化失败: {e}"))
}

/// 导出 bundle（返回 JSON，前端打包为 ZIP）
pub fn export_bundle(store: &LogStore, opts: &ExportOptions) -> Result<serde_json::Value, String> {
    let backend_logs = store.query(&LogFilter {
        kind: Some(LogKind::Backend),
        limit: Some(2000),
        ..Default::default()
    });

    let llm_logs_raw = store.query(&LogFilter {
        kind: Some(LogKind::LlmCall),
        limit: Some(200),
        ..Default::default()
    });

    let frontend_logs = store.query(&LogFilter {
        kind: Some(LogKind::FrontendPlugin),
        limit: Some(2000),
        ..Default::default()
    });

    // LLM 日志脱敏
    let llm_logs: Vec<serde_json::Value> = llm_logs_raw
        .iter()
        .map(|entry| {
            if opts.redact_content {
                if let Some(ref detail) = entry.llm_detail {
                    Ok({
                        let redacted = detail.redact();
                        serde_json::json!({
                        "id": entry.id,
                        "timestamp": entry.timestamp,
                        "level": entry.level,
                        "message": entry.message,
                        "llm_detail": redacted,
                        })
                    })
                } else {
                    log_entry_to_value(entry)
                }
            } else {
                log_entry_to_value(entry)
            }
        })
        .collect::<Result<_, _>>()?;

    Ok(serde_json::json!({
        "exported_at": Utc::now().to_rfc3339(),
        "system_info": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "app_version": env!("CARGO_PKG_VERSION"),
        },
        "backend_logs": backend_logs,
        "llm_logs": llm_logs,
        "frontend_logs": frontend_logs,
        "counts": {
            "backend": backend_logs.len(),
            "llm": llm_logs.len(),
            "frontend": frontend_logs.len(),
        },
    }))
}

// ─── tracing → LogStore 桥接 ───────────────────────────────────────────────

/// tracing → LogStore 桥接层
///
/// 把 tracing 的 event（info!/warn!/error!）转成 LogEntry 推进 LogStore，
/// 让前端日志面板能看到后端日志（§12.1 ①后端日志）。
struct LogStoreLayer {
    store: Arc<LogStore>,
}

impl LogStoreLayer {
    fn new(store: Arc<LogStore>) -> Self {
        Self { store }
    }
}

impl<S> tracing_subscriber::Layer<S> for LogStoreLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::DEBUG | tracing::Level::TRACE => LogLevel::Debug,
        };

        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);

        let target = event.metadata().target();
        let message = if visitor.0.is_empty() {
            target.to_string()
        } else {
            format!("[{target}] {}", visitor.0)
        };

        let entry = LogEntry {
            id: Id::new(),
            kind: LogKind::Backend,
            level,
            timestamp: Utc::now(),
            message,
            fields: Default::default(),
            llm_detail: None,
        };

        self.store.push(entry);
    }
}

/// tracing field visitor，提取 message 字段
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }
}

// ─── tracing 初始化 ────────────────────────────────────────────────────────

/// 初始化 tracing subscriber（后端日志 → LogStore + stderr）
///
/// 需要在 app 启动时调用一次。tracing 的输出：
/// - 写入 LogStore（前端日志面板可查，ERROR + LLM 调用落盘）
/// - 同时输出到 stderr（开发调试用）
pub fn init_tracing(store: Arc<LogStore>) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // 全局过滤：info 及以上（可被 RUST_LOG 环境变量覆盖）
    let filter = EnvFilter::from_default_env().add_directive("info".parse().unwrap());
    // fmt 输出到 stderr（开发调试）
    let fmt_layer = tracing_subscriber::fmt::layer();
    // LogStore 输出（前端日志面板可查）
    let store_layer = LogStoreLayer::new(store);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(store_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Barrier};

    fn make_entry(kind: LogKind, level: LogLevel, msg: &str) -> LogEntry {
        LogEntry {
            id: Id::new(),
            kind,
            level,
            timestamp: Utc::now(),
            message: msg.to_string(),
            fields: HashMap::new(),
            llm_detail: None,
        }
    }

    #[test]
    fn test_buffer_push_and_query() {
        let mut buf = LogBuffer::new();
        buf.push(make_entry(LogKind::Backend, LogLevel::Info, "test1"));
        buf.push(make_entry(LogKind::LlmCall, LogLevel::Info, "test2"));
        buf.push(make_entry(LogKind::Backend, LogLevel::Error, "test3"));

        // 全部
        let all = buf.query(&LogFilter::default());
        assert_eq!(all.len(), 3);

        // 按 kind
        let backend = buf.query(&LogFilter {
            kind: Some(LogKind::Backend),
            ..Default::default()
        });
        assert_eq!(backend.len(), 2);

        // 按 level
        let errors = buf.query(&LogFilter {
            level: Some(LogLevel::Error),
            ..Default::default()
        });
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "test3");
    }

    #[test]
    fn test_buffer_lru_eviction() {
        let mut buf = LogBuffer::new();
        for i in 0..2005 {
            buf.push(make_entry(
                LogKind::Backend,
                LogLevel::Info,
                &format!("msg{i}"),
            ));
        }
        let all = buf.query(&LogFilter {
            kind: Some(LogKind::Backend),
            ..Default::default()
        });
        assert_eq!(all.len(), MAX_ENTRIES_PER_KIND);
        // 最早的应该被淘汰
        assert!(!all.iter().any(|e| e.message == "msg0"));
    }

    #[test]
    fn test_llm_detail_redact() {
        let detail = LlmCallDetail {
            connection_name: "DeepSeek".into(),
            model: "deepseek-chat".into(),
            profile_id: None,
            agent_role: None,
            request_payload: "很长的prompt...".into(),
            response_text: "很长的回复...".into(),
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 1500,
            error: None,
        };
        let redacted = detail.redact();
        assert!(redacted.request_payload_redacted.contains("chars"));
        assert!(redacted.response_text_redacted.contains("chars"));
        assert_eq!(redacted.prompt_tokens, 100);
    }

    #[test]
    fn test_export_bundle() {
        let dir = std::env::temp_dir().join("storyforge_test_export");
        let store = LogStore::new(dir.clone());

        store.push(make_entry(LogKind::Backend, LogLevel::Info, "backend msg"));
        store.push(make_entry(LogKind::LlmCall, LogLevel::Info, "llm call"));

        let bundle = export_bundle(&store, &ExportOptions::default()).unwrap();
        assert_eq!(bundle["counts"]["backend"], 1);
        assert_eq!(bundle["counts"]["llm"], 1);

        // 清理
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_log_store_concurrent_persist_writes_complete_jsonl_lines() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_log_concurrent_{}",
            uuid::Uuid::new_v4()
        ));
        let store = Arc::new(LogStore::new(dir.clone()));
        let threads = 12;
        let per_thread = 40;
        let barrier = Arc::new(Barrier::new(threads));

        let handles: Vec<_> = (0..threads)
            .map(|thread_id| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for i in 0..per_thread {
                        let mut entry = make_entry(
                            LogKind::LlmCall,
                            LogLevel::Info,
                            &format!("thread-{thread_id}-entry-{i}"),
                        );
                        entry.fields.insert(
                            "payload".into(),
                            serde_json::Value::String("x".repeat(8192)),
                        );
                        store.push(entry);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let date = Utc::now().format("%Y-%m-%d").to_string();
        let log_path = dir.join(format!("{date}.jsonl"));
        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<_> = content.lines().collect();
        assert_eq!(lines.len(), threads * per_thread);

        let messages: HashSet<String> = lines
            .iter()
            .map(|line| {
                let entry: LogEntry = serde_json::from_str(line).unwrap();
                entry.message
            })
            .collect();
        assert_eq!(messages.len(), threads * per_thread);
        assert!(messages.contains("thread-0-entry-0"));
        assert!(messages.contains(&format!("thread-{}-entry-{}", threads - 1, per_thread - 1)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-2：LogStore::new 应回填磁盘 jsonl 历史，否则跨会话日志全丢、调试面板空白。
    #[test]
    fn test_log_store_backfills_history_on_startup() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_log_backfill_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 模拟上一会话落盘的 3 条历史日志（直接写 jsonl，模拟 persist_entry 产物）
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let log_path = dir.join(format!("{date}.jsonl"));
        let mut content = String::new();
        let entries = vec![
            make_entry(LogKind::LlmCall, LogLevel::Info, "old-call-1"),
            make_entry(LogKind::LlmCall, LogLevel::Info, "old-call-2"),
            make_entry(LogKind::Backend, LogLevel::Error, "old-error"),
        ];
        for e in &entries {
            content.push_str(&serde_json::to_string(e).unwrap());
            content.push('\n');
        }
        std::fs::write(&log_path, content).unwrap();

        // 新建 LogStore（模拟重启）应回填这 3 条
        let store = LogStore::new(dir.clone());
        let all = store.query(&LogFilter::default());
        let messages: HashSet<String> = all.iter().map(|e| e.message.clone()).collect();
        assert!(messages.contains("old-call-1"), "应回填 LlmCall 历史");
        assert!(messages.contains("old-call-2"), "应回填 LlmCall 历史");
        assert!(messages.contains("old-error"), "应回填 Backend Error 历史");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-2：损坏的 jsonl 行不应阻断回填（容错）。
    #[test]
    fn test_log_store_backfill_skips_corrupt_lines() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_log_corrupt_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let log_path = dir.join(format!("{date}.jsonl"));
        let good = make_entry(LogKind::LlmCall, LogLevel::Info, "good-line");
        let good_json = serde_json::to_string(&good).unwrap();
        // 混入损坏行
        std::fs::write(
            &log_path,
            format!("{good_json}\nTHIS_IS_NOT_JSON\n{good_json}\n"),
        )
        .unwrap();

        let store = LogStore::new(dir.clone());
        let all = store.query(&LogFilter::default());
        let count = all
            .iter()
            .filter(|e| e.message == "good-line")
            .count();
        assert_eq!(count, 2, "损坏行应被跳过,两条有效行应回填");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
