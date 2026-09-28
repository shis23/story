/// 记忆归档器（对应设计 §7.2）
///
/// 当 Recent Window 溢出时触发归档：批量取出 → LLM 压缩 → 嵌入 → 入向量库。
/// 借鉴 shujuku 的 summaryPromptGroup 工作流。
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use storyforge_domain::Id;
use storyforge_infra_llm::{Embedder, LlmClient};
use storyforge_infra_vector::{VectorKind, VectorRecord, VectorStore};

// ─── 归档配置（对应设计 §7.2 ArchiveConfig）────────────────────────────────

/// 归档配置
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    /// 触发阈值（Recent Window 超过此值才归档）
    pub threshold: usize,
    /// 每批取多少条消息
    pub archive_batch_size: usize,
    /// 一次触发归档几批
    pub archive_trigger_count: usize,
    /// 最大并发归档数
    pub max_concurrency: usize,
    /// 总结最大 token 数（近似，用字符数估算）
    pub summary_max_chars: usize,
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            threshold: 50,
            archive_batch_size: 3,
            archive_trigger_count: 9,
            max_concurrency: 3,
            summary_max_chars: 2000, // 约 500 TK
        }
    }
}

// ─── 归档总结（对应设计 §5 ArchivedSummary）─────────────────────────────────

/// 归档的远记忆总结
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedSummary {
    pub id: Id,
    /// 总结内容（≤500TK 高密度）
    pub content: String,
    /// 归档自哪些消息的索引范围
    pub source_range: (usize, usize),
    pub created_at: DateTime<Utc>,
    /// 嵌入向量
    pub vector: Option<Vec<f32>>,
    /// 关键词索引
    pub keywords: Vec<String>,
}

// ─── 归档器 ────────────────────────────────────────────────────────────────

/// 记忆归档器
pub struct MemoryArchiver {
    llm: Arc<dyn LlmClient>,
    embedder: Arc<Embedder>,
    vector_store: Arc<dyn VectorStore>,
    config: ArchiveConfig,
    /// Model name used for LLM summarization during archive.
    model: String,
}

impl MemoryArchiver {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        embedder: Arc<Embedder>,
        vector_store: Arc<dyn VectorStore>,
        config: ArchiveConfig,
        model: String,
    ) -> Self {
        Self {
            llm,
            embedder,
            vector_store,
            config,
            model,
        }
    }

    /// 检查并执行归档（如果 Recent Window 超过阈值）
    ///
    /// `recent_window` 是当前的近期消息列表。
    /// 返回归档的总结列表（可能为空）。
    ///
    /// 注意：本函数不负责水位/幂等——调用方应只传入尚未归档的前缀
    ///（见 Conversation.archived_upto），否则向量池会出现重复 ArchivedSummary。
    pub async fn maybe_archive(
        &self,
        recent_window: &[String],
    ) -> Result<Vec<ArchivedSummary>, MemoryError> {
        self.maybe_archive_with_meta(recent_window, None).await
    }

    /// 同 `maybe_archive`，可附带 campaign/conversation metadata 写入向量记录。
    pub async fn maybe_archive_with_meta(
        &self,
        recent_window: &[String],
        meta: Option<&ArchiveMeta>,
    ) -> Result<Vec<ArchivedSummary>, MemoryError> {
        if recent_window.len() < self.config.threshold {
            debug!(target: "app-memory", "近期窗口 {} 条 < 阈值 {}，跳过归档", 
                recent_window.len(), self.config.threshold);
            return Ok(vec![]);
        }
        self.archive_prefix(recent_window, meta).await
    }

    /// 归档给定消息前缀（不做 threshold 检查）。
    ///
    /// 用于水位驱动路径：调用方已确认需要归档，只传入未归档切片。
    /// 最多归档 `archive_batch_size * archive_trigger_count` 条。
    /// 返回的 `source_range` 相对于入参切片下标（0-based）。
    pub async fn archive_prefix(
        &self,
        messages: &[String],
        meta: Option<&ArchiveMeta>,
    ) -> Result<Vec<ArchivedSummary>, MemoryError> {
        if messages.is_empty() {
            return Ok(vec![]);
        }

        let batch_size = self.config.archive_batch_size.max(1);
        let trigger_count = self.config.archive_trigger_count.max(1);
        let total_to_archive = batch_size.saturating_mul(trigger_count).min(messages.len());

        info!(target: "app-memory", "触发归档：取出最早的 {total_to_archive} 条消息压缩为长期记忆");

        let to_archive: Vec<(usize, String)> = messages
            .iter()
            .enumerate()
            .take(total_to_archive)
            .map(|(i, s)| (i, s.clone()))
            .collect();

        let mut summaries = Vec::new();
        let batches: Vec<Vec<(usize, String)>> = to_archive
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        let batch_count = batches.len();

        use futures::StreamExt;
        // W-06/W-15：按 batch index 收集结果（不再只收集成功项），max_concurrency
        // 钳制到 ≥1（0 会让 buffer_unordered 永不 poll 内层、collect 永久挂起）。
        let stream = futures::stream::iter(batches.into_iter().enumerate())
            .map(|(batch_idx, batch)| {
                let llm = self.llm.clone();
                let config = self.config.clone();
                let model = self.model.clone();
                async move {
                    let msgs: Vec<&str> = batch.iter().map(|(_, m)| m.as_str()).collect();
                    let start_idx = batch.first().map(|(i, _)| *i).unwrap_or(0);
                    let end_idx = batch.last().map(|(i, _)| *i).unwrap_or(0);
                    match archive_batch(&*llm, &msgs, config.summary_max_chars, &model).await {
                        Ok((content, keywords)) => (
                            batch_idx,
                            Ok(ArchivedSummary {
                                id: Id::new(),
                                content,
                                source_range: (start_idx, end_idx),
                                created_at: Utc::now(),
                                vector: None,
                                keywords,
                            }),
                        ),
                        Err(e) => {
                            warn!(target: "app-memory", "批次 {batch_idx} 归档失败: {e}");
                            (batch_idx, Err(e))
                        }
                    }
                }
            })
            .buffer_unordered(self.config.max_concurrency.max(1));

        let results: Vec<(usize, Result<ArchivedSummary, MemoryError>)> = stream.collect().await;
        let mut slots: Vec<Option<ArchivedSummary>> = (0..batch_count).map(|_| None).collect();
        for (batch_idx, result) in results {
            match result {
                Ok(summary) => slots[batch_idx] = Some(summary),
                Err(e) => warn!(
                    target: "app-memory",
                    "批次 {batch_idx} 归档失败（水位不得越过该缺口）: {e}"
                ),
            }
        }
        // W-06：只返回"连续成功前缀"。缺口之后即使成功也不返回（下次从缺口重试），
        // 避免调用方按 max(source_range.1)+1 推进 archived_upto 后留下永久空洞。
        for (batch_idx, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(summary) => summaries.push(summary),
                None => {
                    warn!(
                        target: "app-memory",
                        "批次 {batch_idx} 缺席，本轮只返回前 {batch_idx} 批（缺口下次重试）"
                    );
                    break;
                }
            }
        }

        // W-06：嵌入/入库失败同样构成缺口——截断到失败点之前，绝不"失败了还推进水位"。
        let mut embedded_upto = summaries.len();
        for (index, summary) in summaries.iter_mut().enumerate() {
            let vector = match self.embedder.embed(&summary.content).await {
                Ok(vector) => vector,
                Err(e) => {
                    warn!(
                        target: "app-memory",
                        "总结 {} 嵌入失败（水位不得越过该缺口）: {e}",
                        summary.id
                    );
                    embedded_upto = index;
                    break;
                }
            };
            let mut metadata = std::collections::HashMap::new();
            if let Some(m) = meta {
                if let Some(cid) = &m.campaign_id {
                    metadata.insert("campaign_id".into(), serde_json::Value::String(cid.clone()));
                }
                if let Some(vid) = &m.conversation_id {
                    metadata.insert(
                        "conversation_id".into(),
                        serde_json::Value::String(vid.clone()),
                    );
                }
                metadata.insert(
                    "source".into(),
                    serde_json::Value::String("message_archive".into()),
                );
            }
            if let Err(e) = self.vector_store.upsert(VectorRecord {
                id: summary.id.clone(),
                content: summary.content.clone(),
                vector: vector.clone(),
                keywords: summary.keywords.clone(),
                kind: VectorKind::ArchivedSummary,
                metadata,
            }) {
                warn!(
                    target: "app-memory",
                    "总结 {} 入向量库失败（水位不得越过该缺口）: {e}",
                    summary.id
                );
                embedded_upto = index;
                break;
            }
            // 仅入库成功后才标记 vector（旧实现失败也置 Some，导致召回永远看不到）
            summary.vector = Some(vector);
            info!(target: "app-memory", "总结 {} 已嵌入并入库", summary.id);
        }
        summaries.truncate(embedded_upto);

        info!(target: "app-memory", "归档完成：{} 条总结", summaries.len());
        Ok(summaries)
    }
}

/// 归档写入向量库时的可选标签（campaign / conversation 隔离）。
#[derive(Debug, Clone, Default)]
pub struct ArchiveMeta {
    pub campaign_id: Option<String>,
    pub conversation_id: Option<String>,
}

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

/// 归档一批消息（LLM 压缩）
///
/// 复用 shujuku 的总结 prompt：高密度，优先级：人物关系/关键事件/目标变化/冲突/道具/伏笔。
async fn archive_batch(
    llm: &dyn LlmClient,
    messages: &[&str],
    max_chars: usize,
    model: &str,
) -> Result<(String, Vec<String>), MemoryError> {
    let messages_text = messages.join("\n---\n");

    let prompt = format!(
        r#"你负责将一批较早的对话整理为可供长期召回的远记忆大总结。
目标：生成一条可被向量召回使用的高密度长期记忆。
硬性长度约束：最终输出最高 {} 字符；信息过多优先压缩，不要扩写。
内容优先级：人物关系、关键事件、目标变化、冲突、重要道具、地点、时间线、未解决伏笔。
输出要求：只输出最终远记忆大总结正文。

待整理的对话：
{}"#,
        max_chars, messages_text
    );

    let req = storyforge_domain::llm::ChatRequest {
        messages: vec![storyforge_domain::llm::ChatMessage::user(&prompt)],
        tools: None,
        params: storyforge_domain::llm::SamplingParams {
            temperature: Some(0.3),
            max_tokens: Some(max_chars as u32 / 2), // 粗略估算
            ..Default::default()
        },
        model: model.to_string(),
    };

    let resp = llm.chat(&req).await.map_err(MemoryError::Llm)?;

    // 提取关键词（简单实现：从总结中提取高频词）
    let keywords = extract_keywords(&resp.content);

    Ok((resp.content, keywords))
}

/// 简单关键词提取（从文本中提取出现频率较高的词）
///
/// 对 CJK 文本使用 bigram 分词，对拉丁文本使用整词。
/// 也用于 RoundSummary → 向量库关键词索引，供远记忆召回。
///
/// 同频时按首次出现顺序稳定排序，避免 HashMap 乱序导致短摘要丢关键 bigram。
pub fn extract_keywords(text: &str) -> Vec<String> {
    let mut word_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut first_seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut order = 0usize;

    let mut bump = |word: String| {
        if !first_seen.contains_key(&word) {
            first_seen.insert(word.clone(), order);
            order += 1;
        }
        *word_counts.entry(word).or_insert(0) += 1;
    };

    // 按空白和标点（含 CJK 标点）分段
    for segment in text.split(|c: char| c.is_whitespace() || is_punctuation(c)) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        if segment.chars().any(is_cjk_char) {
            // CJK 文本：提取 bigram 作为关键词
            let cjk_chars: Vec<char> = segment.chars().filter(|c| is_cjk_char(*c)).collect();
            for pair in cjk_chars.windows(2) {
                bump(pair.iter().collect());
            }
            // 也提取段中混杂的拉丁单词
            for latin_word in segment.split(|c: char| is_cjk_char(c) || is_punctuation(c)) {
                let w = latin_word.trim();
                if w.len() >= 2 && w.is_ascii() {
                    bump(w.to_string());
                }
            }
        } else {
            // 纯拉丁文本：整词
            if segment.len() >= 2 && segment.len() <= 20 {
                bump(segment.to_string());
            }
        }
    }

    // 频率降序；同频按首次出现升序。短摘要取前 24 个，降低漏关键词概率。
    let mut words: Vec<(String, usize, usize)> = word_counts
        .into_iter()
        .map(|(w, c)| {
            let seen = *first_seen.get(&w).unwrap_or(&usize::MAX);
            (w, c, seen)
        })
        .collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));
    words.into_iter().take(24).map(|(w, _, _)| w).collect()
}

/// 判断字符是否为 CJK 统一表意文字
fn is_cjk_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
        || ('\u{3400}'..='\u{4DBF}').contains(&c)
        || ('\u{F900}'..='\u{FAFF}').contains(&c)
}

/// 判断字符是否为标点（含 CJK 标点）
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        // CJK 标点符号 。，、；：！？「」『』（）【】《》
        || ('\u{3000}'..='\u{303F}').contains(&c)
        // 全角标点 ！＂＃＄％＆＇（）＊＋，－．／：；＜＝＞？＠
        || ('\u{FF01}'..='\u{FF0F}').contains(&c)
        || ('\u{FF1A}'..='\u{FF20}').contains(&c)
        || ('\u{FF3B}'..='\u{FF40}').contains(&c)
        || ('\u{FF5B}'..='\u{FF65}').contains(&c)
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("LLM 调用失败: {0}")]
    Llm(storyforge_domain::llm::LlmError),

    #[error("向量存储错误: {0}")]
    Vector(#[from] storyforge_infra_vector::VectorError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use storyforge_domain::llm::{ChatRequest, ChatResponse, LlmError, StreamChunk};
    use storyforge_infra_llm::LlmClient;
    use tokio::sync::{mpsc, watch};

    struct RecordingLlm {
        seen_model: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl LlmClient for RecordingLlm {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            *self.seen_model.lock().unwrap() = Some(req.model.clone());
            Ok(ChatResponse {
                content: "archived memory summary".into(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                usage: None,
            })
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            _tx: mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            self.chat(req).await
        }
    }

    #[test]
    fn test_extract_keywords() {
        let text = "Seraphina 和用户在雨中告别。Seraphina 说：有些告别是为了更好的重逢。";
        let keywords = extract_keywords(text);
        // 拉丁词应被提取
        assert!(keywords.contains(&"Seraphina".to_string()));
        // CJK bigram "告别" 出现两次，应在关键词中
        assert!(keywords.contains(&"告别".to_string()));
    }

    #[test]
    fn test_archive_config_default() {
        let config = ArchiveConfig::default();
        assert_eq!(config.threshold, 50);
        assert_eq!(config.max_concurrency, 3);
    }

    #[tokio::test]
    async fn archive_batch_uses_supplied_model() {
        let seen_model = Arc::new(Mutex::new(None));
        let llm = RecordingLlm {
            seen_model: seen_model.clone(),
        };

        let (summary, _) = archive_batch(
            &llm,
            &["first older message", "second older message"],
            512,
            "custom-archive-model",
        )
        .await
        .unwrap();

        assert_eq!(summary, "archived memory summary");
        assert_eq!(
            seen_model.lock().unwrap().as_deref(),
            Some("custom-archive-model")
        );
    }

    /// 第 N 次调用返回错误的 LLM（测试批次缺口）。
    struct FailingLlm {
        calls: std::sync::atomic::AtomicUsize,
        fail_on_call: usize,
    }

    #[async_trait]
    impl LlmClient for FailingLlm {
        async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse, LlmError> {
            let index = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if index == self.fail_on_call {
                return Err(LlmError::Internal(format!("scripted failure #{index}")));
            }
            Ok(ChatResponse {
                content: "archived memory summary".into(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                usage: None,
            })
        }

        async fn chat_stream(
            &self,
            req: &ChatRequest,
            _tx: mpsc::UnboundedSender<StreamChunk>,
            _cancel: watch::Receiver<bool>,
        ) -> Result<ChatResponse, LlmError> {
            self.chat(req).await
        }
    }

    /// 不可达端点的 embedder：构造不需要网络，调用必定失败（W-06 用）。
    fn unreachable_embedder() -> Arc<Embedder> {
        Arc::new(
            Embedder::new(storyforge_infra_llm::EmbedConfig {
                endpoint: "http://127.0.0.1:1/v1/embeddings".into(),
                api_key: "test".into(),
                model: "test-embed".into(),
                dim: 8,
            })
            .expect("构造 embedder 不应失败"),
        )
    }

    fn archiver_with(llm: Arc<dyn LlmClient>, config: ArchiveConfig) -> MemoryArchiver {
        MemoryArchiver::new(
            llm,
            unreachable_embedder(),
            Arc::new(storyforge_infra_vector::BruteForceStore::new()),
            config,
            "archive-model".into(),
        )
    }

    /// W-06：批次 0 失败时，批次 1 的成功结果不得返回（否则调用方按
    /// max(source_range.1)+1 推进水位会在缺口处留下永久空洞）。
    #[tokio::test]
    async fn archive_prefix_stops_at_first_failed_batch() {
        let config = ArchiveConfig {
            archive_batch_size: 1,
            archive_trigger_count: 3,
            max_concurrency: 1,
            ..Default::default()
        };
        let llm: Arc<dyn LlmClient> = Arc::new(FailingLlm {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_on_call: 0, // 第一批失败，后续成功
        });
        let archiver = archiver_with(llm, config);
        let messages = vec!["m0".to_string(), "m1".to_string(), "m2".to_string()];

        let summaries = archiver.archive_prefix(&messages, None).await.unwrap();
        assert!(
            summaries.is_empty(),
            "首个批次失败后不得返回其后批次的总结: {summaries:?}"
        );
    }

    /// W-06：嵌入失败时不得返回 summary（否则水位推进会丢掉这段记忆）。
    #[tokio::test]
    async fn archive_prefix_drops_summaries_when_embed_fails() {
        let config = ArchiveConfig {
            archive_batch_size: 2,
            archive_trigger_count: 1,
            max_concurrency: 1,
            ..Default::default()
        };
        let llm: Arc<dyn LlmClient> = Arc::new(FailingLlm {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_on_call: usize::MAX, // 全部成功
        });
        let archiver = archiver_with(llm, config);
        let messages = vec!["m0".to_string(), "m1".to_string()];

        let summaries = archiver.archive_prefix(&messages, None).await.unwrap();
        assert!(
            summaries.is_empty(),
            "嵌入失败时不得返回 summary（水位不得越过）: {summaries:?}"
        );
    }

    /// W-15：max_concurrency=0 必须被钳制（否则 buffer_unordered(0) 让 collect 永挂）。
    #[tokio::test]
    async fn archive_prefix_clamps_zero_concurrency() {
        let config = ArchiveConfig {
            archive_batch_size: 1,
            archive_trigger_count: 1,
            max_concurrency: 0,
            ..Default::default()
        };
        let llm: Arc<dyn LlmClient> = Arc::new(FailingLlm {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_on_call: usize::MAX,
        });
        let archiver = archiver_with(llm, config);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            archiver.archive_prefix(&["m0".to_string()], None),
        )
        .await;
        assert!(result.is_ok(), "max_concurrency=0 不得让归档永久挂起");
        // embed 不可达 → 返回空（但调用本身完成）
        assert!(result.unwrap().unwrap().is_empty());
    }
}
