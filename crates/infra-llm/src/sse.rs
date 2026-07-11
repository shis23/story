/// 自研 SSE（Server-Sent Events）解析器
///
/// 设计来源：TT 的 SseEventAccumulator（手动按行解析 data:，不用 eventsource crate）。
/// 流程：reqwest::Response::chunk() 循环 → 按 \n 分行 → 解析 data: → 空行时 dispatch。
use storyforge_domain::llm::{LlmError, StreamChunk};

/// SSE 事件累积器（解析 data: 字段，空行时产出事件）
///
/// 同时维护累积的完整内容和工具调用，供调用者在流结束后构造 ChatResponse。
pub struct SseEventAccumulator {
    data: Vec<u8>,
    /// 累积的完整文本内容
    pub full_content: String,
    /// 累积的工具调用（按 stream index 增量合并）
    pub all_tool_calls: Vec<AccumulatedToolCall>,
    /// 最终的 finish_reason
    pub finish_reason: Option<String>,
    /// 末 chunk 的 usage（需请求时设 stream_options.include_usage=true 才有）
    pub usage: Option<storyforge_domain::llm::Usage>,
}

/// 累积中的工具调用（流式合并用，带 stream index）
pub struct AccumulatedToolCall {
    /// 流式协议里的 index（用于跨 chunk 合并同一个 tool_call）
    pub stream_index: Option<usize>,
    pub id: String,
    pub function: AccumulatedFunction,
}

pub struct AccumulatedFunction {
    pub name: String,
    pub arguments: String,
}

impl SseEventAccumulator {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            full_content: String::new(),
            all_tool_calls: Vec::new(),
            finish_reason: None,
            usage: None,
        }
    }

    /// 处理一行数据（不含行尾 \n）
    pub fn on_line(
        &mut self,
        line: &[u8],
        sender: &tokio::sync::mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<(), LlmError> {
        // 空行 = 事件边界，dispatch
        if line.is_empty() {
            return self.dispatch(sender);
        }

        // 解析 data: 字段（SSE 规范：data: 后可带 0 或 1 个空格）
        // 历史 bug：旧实现 strip_prefix(line, b"data: ") 要求严格 1 个空格，
        // 部分代理/服务端发 data:{...} 紧贴格式会被静默丢弃 → 流式内容丢失。
        if let Some(rest) = strip_prefix(line, b"data:") {
            // strip 至多一个空格（SSE 规范允许 data:value 或 data: value）
            let rest = if rest.first() == Some(&b' ') {
                &rest[1..]
            } else {
                rest
            };
            if !self.data.is_empty() {
                self.data.push(b'\n');
            }
            self.data.extend_from_slice(rest);
        }
        // 忽略 event:、id:、retry: 等其他字段（OpenAI 不用它们）
        Ok(())
    }

    /// 流结束时 flush 剩余数据
    pub fn finish(
        &mut self,
        sender: &tokio::sync::mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<(), LlmError> {
        if !self.data.is_empty() {
            self.dispatch(sender)?;
        }
        Ok(())
    }

    /// dispatch 累积的 data 字段
    fn dispatch(
        &mut self,
        sender: &tokio::sync::mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<(), LlmError> {
        let data = std::mem::take(&mut self.data);

        // 跳过空数据
        if data.is_empty() {
            return Ok(());
        }

        let text = String::from_utf8(data)
            .map_err(|e| LlmError::StreamParse(format!("UTF-8 解码失败: {e}")))?;

        // [DONE] 标记流结束
        if text.trim() == "[DONE]" {
            return Ok(());
        }

        // 解析 JSON → StreamChunk
        let chunk: openai_types::StreamDelta = serde_json::from_str(&text)
            .map_err(|e| LlmError::StreamParse(format!("JSON 解析失败: {e}; data={text}")))?;

        let delta = chunk.choices.first().map(|c| &c.delta);

        let delta_content = delta.and_then(|d| d.content.clone());
        let chunk_finish = chunk.choices.first().and_then(|c| c.finish_reason.clone());

        // 累积完整文本内容
        if let Some(ref text) = delta_content {
            self.full_content.push_str(text);
        }
        // 累积工具调用（按 index 合并增量：第一个 chunk 给 id/name，后续给 arguments 片段）
        if let Some(calls) = chunk
            .choices
            .first()
            .and_then(|c| c.delta.tool_calls.as_ref())
        {
            for call in calls {
                // 按 index 找已存在的条目（流式合并的关键）
                if let Some(existing) = self
                    .all_tool_calls
                    .iter_mut()
                    .find(|c| c.stream_index == Some(call.index))
                {
                    if let Some(ref id) = call.id {
                        existing.id = id.clone();
                    }
                    if let Some(ref name) = call.function.name {
                        existing.function.name = name.clone();
                    }
                    if let Some(ref args) = call.function.arguments {
                        existing.function.arguments.push_str(args);
                    }
                } else {
                    self.all_tool_calls.push(AccumulatedToolCall {
                        stream_index: Some(call.index),
                        id: call.id.clone().unwrap_or_default(),
                        function: AccumulatedFunction {
                            name: call.function.name.clone().unwrap_or_default(),
                            arguments: call.function.arguments.clone().unwrap_or_default(),
                        },
                    });
                }
            }
        }
        if chunk_finish.is_some() {
            self.finish_reason = chunk_finish.clone();
        }

        // 末 chunk 携带 usage（choices 通常为空）。需 stream_options.include_usage=true
        if let Some(u) = &chunk.usage {
            self.usage = Some(storyforge_domain::llm::Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
                cached_tokens: u.prompt_cache_hit_tokens,
                cache_creation_tokens: u.prompt_cache_miss_tokens,
            });
        }

        // 推送 chunk（只带 content delta + finish_reason；
        // tool_call 增量已在上面累积，流结束后由调用者从 accumulator 取完整结果）
        let stream_chunk = StreamChunk {
            delta_content,
            delta_tool_calls: None,
            finish_reason: chunk_finish,
        };

        let _ = sender.send(stream_chunk);
        Ok(())
    }
}

impl Default for SseEventAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// OpenAI SSE 流式响应的 JSON 结构（仅用于解析 data: 字段）
///
/// 注意：流式协议里 tool_call 是**增量**的——第一个 chunk 才有 id/name，
/// 后续 chunk 只有 index + arguments 片段。所以这里不能用完整 ToolCall
/// （它要求 id/name/arguments 全必填），必须用宽松的增量类型。
pub(crate) mod openai_types {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct StreamDelta {
        pub choices: Vec<StreamChoice>,
        /// 末 chunk（choices 为空）会带 usage，需请求时设 stream_options.include_usage=true
        #[serde(default)]
        pub usage: Option<StreamUsage>,
    }

    /// 流式 usage（结构与非流式一致，字段类型与 domain::Usage 对齐为 u32）
    ///
    /// A2：增加可选缓存字段。DeepSeek 流式 usage 在顶层带
    /// `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`；
    /// OpenAI 嵌套在 `prompt_tokens_details.cached_tokens` 中（流式暂不解析嵌套，
    /// 由非流式路径的 `parse_cached_tokens` 覆盖）。
    #[derive(Debug, Deserialize)]
    pub struct StreamUsage {
        pub prompt_tokens: u32,
        pub completion_tokens: u32,
        #[serde(default)]
        pub total_tokens: u32,
        /// DeepSeek 缓存命中
        #[serde(default)]
        pub prompt_cache_hit_tokens: u32,
        /// DeepSeek 缓存未命中（≈ cache creation）
        #[serde(default)]
        pub prompt_cache_miss_tokens: u32,
    }

    #[derive(Debug, Deserialize)]
    pub struct StreamChoice {
        pub delta: Delta,
        pub finish_reason: Option<String>,
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct Delta {
        #[serde(default)]
        pub content: Option<String>,
        #[serde(default)]
        pub tool_calls: Option<Vec<StreamToolCallDelta>>,
    }

    /// 流式 tool_call 增量（宽松：id/name/arguments 都可选）
    ///
    /// - 第一个 chunk：有 `index` + `id` + `function.name`
    /// - 后续 chunk：只有 `index` + `function.arguments`（片段）
    #[derive(Debug, Deserialize)]
    pub struct StreamToolCallDelta {
        /// 必填：用于把跨 chunk 的增量合并到同一个 tool_call
        pub index: usize,
        #[serde(default)]
        pub id: Option<String>,
        #[serde(default)]
        pub function: StreamFunctionDelta,
    }

    #[derive(Debug, Default, Deserialize)]
    pub struct StreamFunctionDelta {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub arguments: Option<String>,
    }
}

/// 按 \n 分行处理原始字节流，驱动 SseEventAccumulator
///
/// 设计来源：TT 的 forward_sse_events()，在 buffer 中按 \n 分行。
pub fn forward_sse_events(
    chunk: &[u8],
    buffer: &mut Vec<u8>,
    accumulator: &mut SseEventAccumulator,
    sender: &tokio::sync::mpsc::UnboundedSender<StreamChunk>,
) -> Result<(), LlmError> {
    buffer.extend_from_slice(chunk);

    // 按 \n 分行（支持 \r\n 和 \n）
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        // 去掉行尾 \r\n 或 \n
        let line = strip_line_ending(&line);
        accumulator.on_line(line, sender)?;
    }

    Ok(())
}

fn strip_line_ending(line: &[u8]) -> &[u8] {
    if line.ends_with(b"\r\n") {
        &line[..line.len() - 2]
    } else if line.ends_with(b"\n") {
        &line[..line.len() - 1]
    } else {
        line
    }
}

fn strip_prefix<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if line.len() >= prefix.len() && &line[..prefix.len()] == prefix {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_accumulator_simple() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = SseEventAccumulator::new();

        // 模拟一个 SSE 事件：data: {"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}
        let json = r#"{"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}"#;
        let data_line = format!("data: {json}");
        acc.on_line(data_line.as_bytes(), &tx).unwrap();
        // 空行 = 事件边界
        acc.on_line(b"", &tx).unwrap();

        let chunk = rx.try_recv().unwrap();
        assert_eq!(chunk.delta_content.as_deref(), Some("你好"));
        assert!(chunk.finish_reason.is_none());
    }

    #[test]
    fn test_sse_done_signal() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = SseEventAccumulator::new();

        acc.on_line(b"data: [DONE]", &tx).unwrap();
        acc.on_line(b"", &tx).unwrap();

        // [DONE] 不产出 chunk
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_sse_multi_line_data() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = SseEventAccumulator::new();

        // SSE 规范：多个 data: 行用 \n 累积。这里测试完整的单行 data: 被正确解析。
        let json = r#"{"choices":[{"delta":{"content":"test"},"finish_reason":null}]}"#;
        acc.on_line(format!("data: {json}").as_bytes(), &tx)
            .unwrap();
        acc.on_line(b"", &tx).unwrap();

        let chunk = rx.try_recv().unwrap();
        assert_eq!(chunk.delta_content.as_deref(), Some("test"));
    }

    #[test]
    fn test_forward_sse_events_split_chunks() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = SseEventAccumulator::new();
        let mut buffer = Vec::new();

        let json = r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}"#;
        let event = format!("data: {json}\n\n");

        // 分两块发送（模拟网络分片）
        let mid = event.len() / 2;
        forward_sse_events(&event.as_bytes()[..mid], &mut buffer, &mut acc, &tx).unwrap();
        forward_sse_events(&event.as_bytes()[mid..], &mut buffer, &mut acc, &tx).unwrap();

        let chunk = rx.try_recv().unwrap();
        assert_eq!(chunk.delta_content.as_deref(), Some("hello"));
    }
}
