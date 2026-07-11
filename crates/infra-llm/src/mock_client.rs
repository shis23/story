/// Mock LLM 客户端（测试/开发用，无需真实 API key）
///
/// 按请求内容自动返回对应的模拟响应：
/// - 角色识别请求 → 产出卡内角色定义 JSON 数组
/// - 导演请求 → 产出 Plan JSON
/// - 子 Agent 请求 → 产出角色表演文本
/// - 编剧请求 → 产出成文 Markdown
use async_trait::async_trait;
use tokio::sync::{mpsc, watch};

use storyforge_domain::llm::{ChatRequest, ChatResponse, StreamChunk, ToolCall, Usage};

/// Mock 响应脚本
pub struct MockScript {
    /// 匹配 system prompt 中包含此字符串
    pub match_keyword: String,
    /// 模拟的完整响应文本
    pub response_content: String,
    /// 模拟的工具调用（可选）
    pub tool_calls: Vec<ToolCall>,
    /// 是否模拟流式输出（逐字发送）
    pub stream: bool,
}

/// Mock LLM 客户端
pub struct MockLlmClient {
    scripts: Vec<MockScript>,
}

impl MockLlmClient {
    /// 使用默认脚本（导演/子/编剧）创建
    pub fn with_defaults() -> Self {
        Self {
            scripts: default_scripts(),
        }
    }

    /// 使用自定义脚本创建
    pub fn new(scripts: Vec<MockScript>) -> Self {
        Self { scripts }
    }

    /// 根据请求匹配脚本
    fn find_script(&self, req: &ChatRequest) -> Option<&MockScript> {
        // 从 system prompt 中找匹配
        let system_text: String = req
            .messages
            .iter()
            .filter(|m| m.role == storyforge_domain::llm::ChatRole::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        self.scripts
            .iter()
            .find(|s| system_text.contains(&s.match_keyword))
    }
}

#[async_trait]
impl crate::LlmClient for MockLlmClient {
    async fn chat(
        &self,
        req: &ChatRequest,
    ) -> Result<ChatResponse, storyforge_domain::llm::LlmError> {
        let script = self.find_script(req);

        let (content, tool_calls) = if let Some(s) = script {
            (s.response_content.clone(), s.tool_calls.clone())
        } else {
            // 默认：返回一段通用文本
            (
                "（MockLlmClient：未匹配到特定脚本，返回默认响应）".to_string(),
                vec![],
            )
        };

        Ok(ChatResponse {
            content,
            tool_calls,
            finish_reason: Some("stop".into()),
            usage: Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cached_tokens: 0,
                cache_creation_tokens: 0,
            }),
        })
    }

    async fn chat_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::UnboundedSender<StreamChunk>,
        cancel: watch::Receiver<bool>,
    ) -> Result<ChatResponse, storyforge_domain::llm::LlmError> {
        let script = self.find_script(req);
        let content = script
            .map(|s| s.response_content.clone())
            .unwrap_or_else(|| "（Mock 默认响应）".to_string());
        let tool_calls = script.map(|s| s.tool_calls.clone()).unwrap_or_default();
        let do_stream = script.map(|s| s.stream).unwrap_or(true);

        if do_stream {
            // 逐字流式输出（模拟真实 SSE）
            let mut cancel = cancel;
            for ch in content.chars() {
                tokio::select! {
                    _ = cancel.changed() => {
                        if *cancel.borrow() {
                            return Err(storyforge_domain::llm::LlmError::Cancelled);
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(10)) => {
                        let _ = tx.send(StreamChunk {
                            delta_content: Some(ch.to_string()),
                            delta_tool_calls: None,
                            finish_reason: None,
                        });
                    }
                }
            }
        }

        // 发送最终 chunk
        let _ = tx.send(StreamChunk {
            delta_content: if do_stream {
                None
            } else {
                Some(content.clone())
            },
            delta_tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.clone())
            },
            finish_reason: Some("stop".into()),
        });

        Ok(ChatResponse {
            content,
            tool_calls,
            finish_reason: Some("stop".into()),
            usage: Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cached_tokens: 0,
                cache_creation_tokens: 0,
            }),
        })
    }
}

/// 默认脚本集（角色识别 / 后处理 / 导演 / 子 Agent / 编剧 / Meta / MVU）
fn default_scripts() -> Vec<MockScript> {
    vec![
        // MVU 五合一分析脚本：产出 MvuTranslation JSON（插最前，避开 "状态" 等宽泛词冲突）
        MockScript {
            match_keyword: "卡内状态栏分析".into(),
            response_content: r#"{
  "variable_schema": [
    {"key": "hp", "label": "生命值", "value_type": "int", "default": 100}
  ],
  "ui_bindings": [
    {"element": "hp_bar", "variable_key": "hp", "display": {"kind": "bar", "max": 100}}
  ],
  "update_rules": ["受伤时 hp 减少伤害值"],
  "interactions": [],
  "fallback_fragments": [],
  "routing": {"kind": "native"},
  "analysis_confidence": 0.9,
  "notes": ["mock：纯数据绑定卡"]
}"#
            .into(),
            tool_calls: vec![],
            stream: false,
        },
        // Meta 配置调试脚本：匹配 "配置调试助手"，返回诊断结论
        MockScript {
            match_keyword: "配置调试助手".into(),
            response_content: "我先检查一下当前配置。".into(),
            tool_calls: vec![],
            stream: false,
        },
        // 后处理脚本：产出三件套 JSON（插在角色识别后，避开"角色"冲突）
        MockScript {
            match_keyword: "后处理".into(),
            response_content: r#"{
  "knowledge_updates": [
    {"character_id": "林医生", "knowledge_text": "我看到陈警官在地下室发现了那具尸体", "source": "witnessed", "pinned": false}
  ],
  "variable_updates": [
    {"instance_id": "林医生", "key": "state", "value": "受伤"},
    {"instance_id": null, "key": "story_clock", "value": "第2天"}
  ],
  "task_updates": [
    {"task_id": null, "new_status": "pending", "new_task": {"title": "老王复仇", "description": "老王被陷害后发誓复仇", "triggers": [{"kind": "event", "description": "三个月期限到达"}], "related_characters": ["老王"]}}
  ]
}"#
            .into(),
            tool_calls: vec![],
            stream: false,
        },
        // 剧情总结脚本：产出本轮摘要文本
        MockScript {
            match_keyword: "本轮剧情总结".into(),
            response_content: "本轮中，林医生在急诊室目睹陈警官带来一具尸体。陈警官透露尸体是在医院地下室发现的，死亡时间约 48 小时前。林医生检查后发现死者颈部有奇怪的针孔，怀疑是非常规药物致死。两人决定暂不公开发现，先私下调查。这一发现加深了林医生对近期医院异常事件的可疑。".into(),
            tool_calls: vec![],
            stream: false,
        },
        // 角色识别脚本：输出卡内角色定义 JSON 数组（插入最前，避开 "角色" 关键词冲突）
        MockScript {
            match_keyword: "卡内角色识别".into(),
            response_content: r#"[
              {
                "name": "林医生",
                "persona_prompt": "你是一位三十出头的外科医生，说话简短精确，习惯先评估后行动。",
                "behavior_rules": "面对危重病人时先评估生命体征再处置；绝不主动透露病人隐私。",
                "base_backstory": ["你是本市三甲医院急诊科主治", "三年前经历过一次失败的手术"],
                "role_type": "protagonist",
                "group": "主角团"
              },
              {
                "name": "陈警官",
                "persona_prompt": "你是一名老刑警，观察力敏锐，话不多但每句都在点上。",
                "behavior_rules": "对证据保持怀疑；不在公开场合透露案情。",
                "base_backstory": ["你在刑警队干了二十年"],
                "role_type": "supporting",
                "group": "主角团"
              }
            ]"#
            .into(),
            tool_calls: vec![],
            stream: false,
        },
        // 导演脚本：在 content 中直接输出 Plan JSON（避免工具调用循环）
        MockScript {
            match_keyword: "写作导演".into(),
            response_content: serde_json::json!({
                "scene_brief": "一场雨中告别戏，两个角色在屋檐下对话",
                "subagent_tasks": [
                    {
                        "character_id": "Seraphina",
                        "brief": "演出告别时的温柔与不舍",
                        "context_package": {
                            "character_brief": "Seraphina，一位温柔的精灵法师",
                            "scene_brief": "雨中告别",
                            "relevant_lore": [],
                            "constant_lore": [],
                            "recent_window": [],
                            "task": "在这场雨中告别中，演出你温柔而不舍的情感"
                        }
                    }
                ]
            })
            .to_string(),
            tool_calls: vec![],
            stream: false,
        },
        // 子 Agent 脚本：产出表演文本
        MockScript {
            match_keyword: "角色".into(),
            response_content: "雨丝如银线般从屋檐滑落，Seraphina 静静站在那里，银色的长发被雨水打湿，紧贴在苍白的脸颊上。她伸出纤细的手指，轻轻触碰了你掌心的温度。\n\n「你知道的……」她的声音如风铃般轻柔，却带着一丝不易察觉的颤抖，「有些告别，是为了更好的重逢。」\n\n*她微微垂下眼帘，睫毛上凝结的水珠分不清是雨还是泪。那一刻，时间仿佛凝固在了她的指尖与你掌心之间。*".into(),
            tool_calls: vec![],
            stream: true,
        },
        // 编剧脚本：产出成文
        MockScript {
            match_keyword: "编剧".into(),
            response_content: "## 雨中告别\n\n雨幕低垂，将整个世界笼罩在一片朦胧的灰蓝色调中。屋檐下的积水映出两道身影，一道纤细如柳，一道沉默如山。\n\nSeraphina 站在那里，银色的长发被雨水浸透，水珠沿着发梢滴落，在她脚边汇成小小的溪流。她没有撑伞，只是静静地看着面前的人，目光中带着一种超越了悲伤的温柔。\n\n「你知道的……」\n\n她的声音很轻，几乎要被雨声淹没，却清晰地传入了你的耳中。那声音如风铃，如溪流，如所有美好事物在消逝前最后的回响。\n\n「有些告别，是为了更好的重逢。」\n\n*她伸出手指，轻轻触碰你的掌心。那一刻的温度，足以温暖此后所有漫长的雨季。*".into(),
            tool_calls: vec![],
            stream: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmClient;
    use storyforge_domain::llm::ChatMessage;

    #[tokio::test]
    async fn test_mock_director_emits_plan() {
        let client = MockLlmClient::with_defaults();
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system("你是写作导演。用户给你写作意图，你要输出 Plan"),
                ChatMessage::user("写一场戏"),
            ],
            tools: None,
            params: Default::default(),
            model: "mock".into(),
        };

        let resp = client.chat(&req).await.unwrap();
        // 导演现在在 content 中输出 Plan JSON（避免工具调用循环）
        assert!(resp.content.contains("scene_brief"));
        assert!(resp.content.contains("subagent_tasks"));
    }

    #[tokio::test]
    async fn test_mock_subagent_returns_performance() {
        let client = MockLlmClient::with_defaults();
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system("你是角色 Seraphina"),
                ChatMessage::user("演出你的部分"),
            ],
            tools: None,
            params: Default::default(),
            model: "mock".into(),
        };

        let resp = client.chat(&req).await.unwrap();
        assert!(resp.content.contains("Seraphina"));
        assert!(resp.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_mock_editor_returns_draft() {
        let client = MockLlmClient::with_defaults();
        let req = ChatRequest {
            messages: vec![
                ChatMessage::system("你是编剧。收集所有子 Agent 的表演，合并成连贯成文"),
                ChatMessage::user("合并这些表演"),
            ],
            tools: None,
            params: Default::default(),
            model: "mock".into(),
        };

        let resp = client.chat(&req).await.unwrap();
        assert!(resp.content.contains("雨中告别"));
    }
}
