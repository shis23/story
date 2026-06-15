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
}

impl MemoryArchiver {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        embedder: Arc<Embedder>,
        vector_store: Arc<dyn VectorStore>,
        config: ArchiveConfig,
    ) -> Self {
        Self {
            llm,
            embedder,
            vector_store,
            config,
        }
    }

    /// 检查并执行归档（如果 Recent Window 超过阈值）
    ///
    /// `recent_window` 是当前的近期消息列表。
    /// 返回归档的总结列表（可能为空）。
    pub async fn maybe_archive(
        &self,
        recent_window: &[String],
    ) -> Result<Vec<ArchivedSummary>, MemoryError> {
        if recent_window.len() < self.config.threshold {
            debug!(target: "app-memory", "近期窗口 {} 条 < 阈值 {}，跳过归档", 
                recent_window.len(), self.config.threshold);
            return Ok(vec![]);
        }

        let batch_size = self.config.archive_batch_size;
        let trigger_count = self.config.archive_trigger_count;
        let total_to_archive = batch_size * trigger_count;
        let total_to_archive = total_to_archive.min(recent_window.len());

        info!(target: "app-memory", "触发归档：取出 {total_to_archive} 条消息");

        // 取出待归档的消息（窗口尾部），clone 为 owned data 避免 lifetime 问题
        let to_archive: Vec<(usize, String)> = recent_window
            .iter()
            .enumerate()
            .take(total_to_archive)
            .map(|(i, s)| (i, s.clone()))
            .collect();

        // 分批并发归档
        let mut summaries = Vec::new();
        let batches: Vec<Vec<(usize, String)>> = to_archive
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        // 并发归档（用 futures::stream::buffer_unordered）
        use futures::StreamExt;
        let stream = futures::stream::iter(batches.into_iter().enumerate())
            .map(|(batch_idx, batch)| {
                let llm = self.llm.clone();
                let config = self.config.clone();
                async move {
                    let messages: Vec<&str> = batch.iter().map(|(_, m)| m.as_str()).collect();
                    let start_idx = batch.first().map(|(i, _)| *i).unwrap_or(0);
                    let end_idx = batch.last().map(|(i, _)| *i).unwrap_or(0);
                    match archive_batch(&*llm, &messages, config.summary_max_chars).await {
                        Ok((content, keywords)) => Ok(ArchivedSummary {
                            id: Id::new(),
                            content,
                            source_range: (start_idx, end_idx),
                            created_at: Utc::now(),
                            vector: None,
                            keywords,
                        }),
                        Err(e) => {
                            warn!(target: "app-memory", "批次 {batch_idx} 归档失败: {e}");
                            Err(e)
                        }
                    }
                }
            })
            .buffer_unordered(self.config.max_concurrency);

        let results: Vec<Result<ArchivedSummary, MemoryError>> = stream.collect().await;
        for result in results {
            match result {
                Ok(summary) => summaries.push(summary),
                Err(e) => warn!(target: "app-memory", "归档批次失败: {e}"),
            }
        }

        // 嵌入每条总结 → 入向量库
        for summary in &mut summaries {
            match self.embedder.embed(&summary.content).await {
                Ok(vector) => {
                    let _ = self.vector_store.upsert(VectorRecord {
                        id: summary.id.clone(),
                        content: summary.content.clone(),
                        vector: vector.clone(),
                        keywords: summary.keywords.clone(),
                        kind: VectorKind::ArchivedSummary,
                        metadata: std::collections::HashMap::new(),
                    });
                    summary.vector = Some(vector);
                    info!(target: "app-memory", "总结 {} 已嵌入并入库", summary.id);
                }
                Err(e) => {
                    warn!(target: "app-memory", "总结 {} 嵌入失败: {e}", summary.id);
                }
            }
        }

        info!(target: "app-memory", "归档完成：{} 条总结", summaries.len());
        Ok(summaries)
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

/// 归档一批消息（LLM 压缩）
///
/// 复用 shujuku 的总结 prompt：高密度，优先级：人物关系/关键事件/目标变化/冲突/道具/伏笔。
async fn archive_batch(
    llm: &dyn LlmClient,
    messages: &[&str],
    max_chars: usize,
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
        model: "deepseek-chat".into(), // 后续从配置读取
    };

    let resp = llm.chat(&req).await.map_err(MemoryError::Llm)?;

    // 提取关键词（简单实现：从总结中提取高频词）
    let keywords = extract_keywords(&resp.content);

    Ok((resp.content, keywords))
}

/// 简单关键词提取（从文本中提取出现频率较高的词）
///
/// 对 CJK 文本使用 bigram 分词，对拉丁文本使用整词。
fn extract_keywords(text: &str) -> Vec<String> {
    let mut word_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

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
                let bigram: String = pair.iter().collect();
                *word_counts.entry(bigram).or_insert(0) += 1;
            }
            // 也提取段中混杂的拉丁单词
            for latin_word in segment.split(|c: char| is_cjk_char(c) || is_punctuation(c)) {
                let w = latin_word.trim();
                if w.len() >= 2 && w.is_ascii() {
                    *word_counts.entry(w.to_string()).or_insert(0) += 1;
                }
            }
        } else {
            // 纯拉丁文本：整词
            if segment.len() >= 2 && segment.len() <= 20 {
                *word_counts.entry(segment.to_string()).or_insert(0) += 1;
            }
        }
    }

    // 按频率排序，取前 10 个
    let mut words: Vec<(String, usize)> = word_counts.into_iter().collect();
    words.sort_by(|a, b| b.1.cmp(&a.1));
    words.into_iter().take(10).map(|(w, _)| w).collect()
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
}
