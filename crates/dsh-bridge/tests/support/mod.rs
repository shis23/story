//! 端到端测试共用：测试扮演 DSH 插件，拉起真实 sidecar 进程。
//!
//! 插件半边收到 `llm.stream` 时把请求还原成 `ChatRequest` 交给 `MockLlmClient`
//! （按系统提示词关键字给出导演 / 角色 / 编剧 / 后处理的脚本化回答），再按协议
//! 回 `llm.chunk` + 响应。

#![allow(dead_code)]

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use storyforge_domain::llm::{
    ChatMessage, ChatRequest, ChatRole, FunctionCall, SamplingParams, ToolCall, ToolSpec,
};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_llm::mock_client::MockLlmClient;

pub const STEP_TIMEOUT: Duration = Duration::from_secs(120);

pub const OPENING: &str = "林间的光斑落在木屋门前，Seraphina 抬起头望向你。";

pub fn write_card(dir: &Path) -> std::path::PathBuf {
    let card = json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "Seraphina",
            "description": "守护森林的女法师，温柔而警惕。",
            "personality": "温柔、警惕",
            "scenario": "旅人在森林里迷路，被 Seraphina 救回木屋。",
            "first_mes": OPENING,
            "mes_example": "",
            "creator_notes": "",
            "system_prompt": "",
            "post_history_instructions": "",
            "alternate_greetings": [],
            "tags": [],
            "creator": "dsh-bridge-test",
            "character_version": "1",
            "extensions": {}
        }
    });
    let path = dir.join("seraphina.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&card).unwrap()).unwrap();
    path
}

/// 状态栏卡：开场白末尾带 `<status>` 数据标签，卡自带一条展示正则把它换成 HTML
/// （只作用于最新一条消息，`maxDepth: 0`）。
pub const STATUS_OPENING: &str = "林间的光斑落在木屋门前。\n<status>体力 80</status>";

pub fn write_status_card(dir: &Path) -> std::path::PathBuf {
    let card = json!({
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "Seraphina",
            "description": "守护森林的女法师，温柔而警惕。",
            "personality": "温柔、警惕",
            "scenario": "旅人在森林里迷路，被 Seraphina 救回木屋。",
            "first_mes": STATUS_OPENING,
            "mes_example": "",
            "alternate_greetings": [],
            "tags": [],
            "creator": "dsh-bridge-test",
            "character_version": "1",
            "extensions": {
                "regex_scripts": [{
                    "id": "status-bar",
                    "scriptName": "状态栏美化",
                    "findRegex": "<status>(.*?)</status>",
                    "replaceString": "<div class=\"sf-bar\">$1</div>",
                    "placement": [2],
                    "disabled": false,
                    "markdownOnly": true,
                    "promptOnly": false,
                    "runOnEdit": true,
                    "substituteRegex": 0,
                    "trimStrings": [],
                    "minDepth": null,
                    "maxDepth": 0
                }]
            }
        }
    });
    let path = dir.join("status-card.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&card).unwrap()).unwrap();
    path
}

/// 协议参数 → ChatRequest（`llm::call_params` 的逆映射）。
pub fn chat_request(params: &Value) -> ChatRequest {
    let messages = params["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            let content = message["content"].as_str().unwrap_or_default().to_string();
            let mut out = match message["role"].as_str().unwrap() {
                "system" => ChatMessage::system(content),
                "user" => ChatMessage::user(content),
                "assistant" => ChatMessage::assistant(content),
                "tool" => ChatMessage::tool_result(
                    message["tool_call_id"].as_str().unwrap_or_default(),
                    content,
                ),
                other => panic!("unexpected role {other}"),
            };
            if let Some(calls) = message["tool_calls"].as_array() {
                out.tool_calls = Some(
                    calls
                        .iter()
                        .map(|call| ToolCall {
                            id: call["id"].as_str().unwrap().into(),
                            call_type: "function".into(),
                            function: FunctionCall {
                                name: call["name"].as_str().unwrap().into(),
                                arguments: call["arguments"].as_str().unwrap().into(),
                            },
                        })
                        .collect(),
                );
            }
            assert!(out.role != ChatRole::Tool || out.tool_call_id.is_some());
            out
        })
        .collect();
    let tools: Vec<ToolSpec> = params["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            ToolSpec::function(
                tool["name"].as_str().unwrap(),
                tool["description"].as_str().unwrap_or_default(),
                tool["parameters"].clone(),
            )
        })
        .collect();
    ChatRequest {
        messages,
        tools: (!tools.is_empty()).then_some(tools),
        params: SamplingParams::default(),
        model: params["model_hint"].as_str().unwrap_or_default().into(),
    }
}

pub struct FakePlugin {
    pub child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    mock: MockLlmClient,
    next_id: u64,
    pub llm_calls: usize,
    /// 带原生 tools 的请求数 / system 里注入了文本工具说明的请求数。
    pub native_tool_requests: usize,
    pub fallback_prompt_requests: usize,
    pub events: Vec<String>,
    /// 完整的 `turn.event` 事件（按到达顺序）。
    pub event_log: Vec<Value>,
    pub settled: Vec<Value>,
    /// 为 true 时收到的 `llm.stream` 只记下不应答（模拟进行中的调用）。
    pub hold_llm: bool,
    pub held: Vec<Value>,
    /// 覆盖导演的回答（Plan JSON）；None 时用 MockLlmClient 的默认脚本。
    pub director_plan: Option<String>,
    /// 大于 0 时把子 Agent 的 `llm.stream`（系统提示词含「你是角色 X。」）攒到这么多个
    /// 同时在途再一起应答：分块交错发、结果倒序回，证明并行且按 id 路由不串流。
    pub subagent_barrier: usize,
    held_subagents: Vec<(Value, String)>,
    /// 观察到的同时在途子 Agent 调用数峰值 / 屏障放行的批数。
    pub max_in_flight_subagents: usize,
    pub subagent_batches: usize,
}

impl FakePlugin {
    pub fn spawn(data_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_storyforge-dsh-bridge"))
            .arg("--data-dir")
            .arg(data_dir)
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sidecar");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            stdin,
            lines,
            mock: MockLlmClient::with_defaults(),
            next_id: 1,
            llm_calls: 0,
            native_tool_requests: 0,
            fallback_prompt_requests: 0,
            events: Vec::new(),
            event_log: Vec::new(),
            settled: Vec::new(),
            hold_llm: false,
            held: Vec::new(),
            director_plan: None,
            subagent_barrier: 0,
            held_subagents: Vec::new(),
            max_in_flight_subagents: 0,
            subagent_batches: 0,
        }
    }

    pub async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    /// 读一条消息；`llm.stream` 就地应答，事件与收尾通知记下来。
    pub async fn pump(&mut self) -> Value {
        let line = tokio::time::timeout(STEP_TIMEOUT, self.lines.next_line())
            .await
            .expect("sidecar 无响应")
            .expect("读 stdout 失败")
            .expect("sidecar 提前退出");
        let message: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout 混入非协议输出 ({error}): {line}"));
        match message["method"].as_str() {
            Some("llm.stream") if self.hold_llm => self.held.push(message.clone()),
            Some("llm.stream")
                if self.subagent_barrier > 0 && subagent_name(&message).is_some() =>
            {
                let name = subagent_name(&message).unwrap();
                self.held_subagents.push((message.clone(), name));
                self.max_in_flight_subagents =
                    self.max_in_flight_subagents.max(self.held_subagents.len());
                if self.held_subagents.len() == self.subagent_barrier {
                    self.answer_held_subagents().await;
                }
            }
            Some("llm.stream") => {
                self.answer_llm(&message).await;
            }
            Some("turn.event") => {
                let event = &message["params"]["event"];
                let kind = event["type"].as_str().unwrap_or("?");
                self.events.push(kind.to_string());
                self.event_log.push(event.clone());
            }
            Some("turn.settled") => self.settled.push(message["params"]["report"].clone()),
            _ => {}
        }
        message
    }

    async fn answer_llm(&mut self, request: &Value) {
        self.llm_calls += 1;
        let params = &request["params"];
        if params["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty())
        {
            self.native_tool_requests += 1;
        }
        if params["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("## 可用工具"))
            })
        {
            self.fallback_prompt_requests += 1;
        }
        let id = request["id"].clone();
        let mut response = self
            .mock
            .chat(&chat_request(&request["params"]))
            .await
            .expect("mock llm");
        if let Some(plan) = &self.director_plan
            && system_text(request).contains("写作导演")
        {
            response.content = plan.clone();
            response.tool_calls.clear();
        }
        if !response.content.is_empty() {
            self.send(json!({
                "method": "llm.chunk",
                "params": { "id": id, "chunk": { "type": "text", "text": response.content } },
            }))
            .await;
        }
        let tool_calls: Vec<Value> = response
            .tool_calls
            .iter()
            .map(|call| {
                json!({ "id": call.id, "name": call.function.name, "arguments": call.function.arguments })
            })
            .collect();
        self.send(json!({
            "id": id,
            "result": {
                "content": response.content,
                "reasoning": response.reasoning_content,
                "tool_calls": tool_calls,
                "finish_reason": response.finish_reason,
                "usage": { "input_tokens": 100, "output_tokens": 50 },
            },
        }))
        .await;
    }

    /// 屏障满了：所有在途子 Agent 调用的分块按轮次交错发出，结果倒序回。
    async fn answer_held_subagents(&mut self) {
        let held = std::mem::take(&mut self.held_subagents);
        self.llm_calls += held.len();
        self.subagent_batches += 1;
        for part in 1..=2 {
            for (request, name) in &held {
                self.send(json!({
                    "method": "llm.chunk",
                    "params": {
                        "id": request["id"],
                        "chunk": { "type": "text", "text": subagent_part(name, part) },
                    },
                }))
                .await;
            }
        }
        for (request, name) in held.iter().rev() {
            self.send(json!({
                "id": request["id"],
                "result": {
                    "content": format!("{}{}", subagent_part(name, 1), subagent_part(name, 2)),
                    "reasoning": null,
                    "tool_calls": [],
                    "finish_reason": "stop",
                    "usage": { "input_tokens": 100, "output_tokens": 50 },
                },
            }))
            .await;
        }
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.send_request(method, params).await;
        self.response(&id).await
    }

    /// 只发请求不等响应，返回请求 id（配合 [`Self::response`]）。
    pub async fn send_request(&mut self, method: &str, params: Value) -> String {
        let id = format!("p-{}", self.next_id);
        self.next_id += 1;
        self.send(json!({ "id": id, "method": method, "params": params }))
            .await;
        id
    }

    /// 一直读到指定请求的响应（期间照常应答模型调用、记录事件）。
    pub async fn response(&mut self, id: &str) -> Result<Value, Value> {
        loop {
            let message = self.pump().await;
            if message["id"] == id && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    /// 应答所有被扣下的模型调用，之后的调用照常即时应答。
    pub async fn release_held(&mut self) {
        self.hold_llm = false;
        for request in std::mem::take(&mut self.held) {
            self.answer_llm(&request).await;
        }
    }

    pub async fn wait_notification(&mut self, method: &str) -> Value {
        loop {
            let message = self.pump().await;
            if message["method"] == method {
                return message["params"].clone();
            }
        }
    }

    pub async fn wait_settled(&mut self, count: usize) -> Value {
        while self.settled.len() < count {
            self.pump().await;
        }
        self.settled[count - 1].clone()
    }
}

/// 子 Agent 第 `part` 段表演的固定文本，带角色标记，便于检查分块归属。两段合起来
/// 超过子 Agent 完成探测的 50 字门槛（不够长会被当成没写完而追问到轮次上限）。
pub fn subagent_part(name: &str, part: usize) -> String {
    format!("〔{name}〕第{part}段表演：{name}望着炉火，慢慢说出自己的打算，语气平静而笃定。")
}

fn system_text(request: &Value) -> String {
    request["params"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 子 Agent 请求的角色显示名（系统提示词里的「你是角色 X。」）；不是子 Agent 返回 None。
/// 取最后一处：79326ea 之前子 Agent 基础模板开头还有一句未替换的「你是角色 {name}。」，
/// 现在模板已不带身份句，只剩运行时追加的这一处。
fn subagent_name(request: &Value) -> Option<String> {
    let text = system_text(request);
    let start = text.rfind("你是角色 ")? + "你是角色 ".len();
    let end = text[start..].find('。')? + start;
    Some(text[start..end].to_string())
}
