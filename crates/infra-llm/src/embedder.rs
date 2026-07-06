/// 嵌入 API 客户端（对应设计 §7.5）
///
/// 调用远程 /v1/embeddings API 获取文本的向量表示。
/// 支持 OpenAI 兼容格式（OpenAI / SiliconFlow / DeepSeek 等）。
use serde::{Deserialize, Serialize};
use std::time::Duration;

use storyforge_domain::llm::LlmError;

// ─── 嵌入配置 ──────────────────────────────────────────────────────────────

/// 嵌入配置
///
/// H-4：手写 Debug 打码 api_key，避免 #[derive(Debug)] 在 tracing/dbg! 时泄漏 key
#[derive(Clone, Serialize, Deserialize)]
pub struct EmbedConfig {
    /// API 端点（如 https://api.openai.com/v1/embeddings）
    pub endpoint: String,
    /// API key（Debug 时打码为 ***）
    pub api_key: String,
    /// 模型名（如 text-embedding-3-small / bge-large-zh）
    pub model: String,
    /// 向量维度
    pub dim: usize,
}

impl std::fmt::Debug for EmbedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedConfig")
            .field("endpoint", &self.endpoint)
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("dim", &self.dim)
            .finish()
    }
}

// ─── 嵌入客户端 ────────────────────────────────────────────────────────────

/// 嵌入 API 客户端
pub struct Embedder {
    client: reqwest::Client,
    config: EmbedConfig,
}

impl Embedder {
    pub fn new(config: EmbedConfig) -> Result<Self, LlmError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| {
                    LlmError::Internal(format!("构建 embedder reqwest client 失败: {e}"))
                })?,
            config,
        })
    }

    /// 获取单个文本的嵌入向量
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let resp = self.embed_batch(&[text]).await?;
        resp.into_iter()
            .next()
            .ok_or_else(|| LlmError::Internal("嵌入 API 返回空结果".into()))
    }

    /// 批量获取嵌入向量
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(&self.config.endpoint)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(format!("嵌入请求失败: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Http(format!("嵌入 API 错误 {status}: {text}")));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Internal(format!("嵌入响应解析失败: {e}")))?;

        parse_embedding_response(&json, self.config.dim)
    }
}

/// 解析 OpenAI 兼容的嵌入响应
fn parse_embedding_response(
    json: &serde_json::Value,
    expected_dim: usize,
) -> Result<Vec<Vec<f32>>, LlmError> {
    let data = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| LlmError::Internal("嵌入响应缺少 data 数组".into()))?;

    let mut results = Vec::new();
    for item in data {
        let embedding = item
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| LlmError::Internal("嵌入项缺少 embedding".into()))?;

        let vec: Vec<f32> = embedding
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_f64()
                    .ok_or_else(|| {
                        LlmError::Internal(format!("嵌入元素 [{i}] 不是有效数值: {:?}", v))
                    })
                    .map(|n| n as f32)
            })
            .collect::<Result<Vec<_>, LlmError>>()?;

        if vec.len() != expected_dim {
            return Err(LlmError::Internal(format!(
                "嵌入维度不匹配：期望 {expected_dim}，实际 {}",
                vec.len()
            )));
        }

        results.push(vec);
    }

    Ok(results)
}

/// 创建默认嵌入配置（SiliconFlow bge-large-zh）
pub fn default_embed_config(api_key: String) -> EmbedConfig {
    EmbedConfig {
        endpoint: "https://api.siliconflow.cn/v1/embeddings".into(),
        api_key,
        model: "BAAI/bge-large-zh-v1.5".into(),
        dim: 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_embedding_response() {
        let json = serde_json::json!({
            "data": [
                { "embedding": [0.1, 0.2, 0.3], "index": 0 },
                { "embedding": [0.4, 0.5, 0.6], "index": 1 }
            ]
        });

        let result = parse_embedding_response(&json, 3).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
        assert_eq!(result[1], vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn test_parse_embedding_dimension_mismatch() {
        let json = serde_json::json!({
            "data": [
                { "embedding": [0.1, 0.2], "index": 0 }
            ]
        });

        let result = parse_embedding_response(&json, 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_embed_config() {
        let config = default_embed_config("test-key".into());
        assert_eq!(config.model, "BAAI/bge-large-zh-v1.5");
        assert_eq!(config.dim, 1024);
    }

    #[test]
    fn test_parse_embedding_non_numeric_element_errors() {
        let json = serde_json::json!({
            "data": [
                { "embedding": [0.1, "bad", 0.3], "index": 0 }
            ]
        });

        let result = parse_embedding_response(&json, 3);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("不是有效数值"));
    }
}
