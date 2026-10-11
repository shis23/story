//! 双向 JSON-RPC 对端（传输无关）。
//!
//! 线上格式：每行一个 JSON 对象。
//! - 请求：`{"id", "method", "params"}`（双方都可以发起）
//! - 响应：`{"id", "result"}` 或 `{"id", "error": {code, message, status?, retry_after_ms?}}`
//! - 通知：`{"method", "params"}`（无 id）
//!
//! sidecar 发起的请求（目前只有 `llm.stream`）可以在响应之前收到若干
//! `llm.chunk {id, chunk}` 通知，按请求 id 路由到调用方。`Peer` 只负责编号、
//! 挂起与路由；读写哪条管道由外壳决定（stdio 外壳见 `main.rs`）。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// 对端返回的错误（也用于本端回给对端的错误）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            status: None,
            retry_after_ms: None,
        }
    }

    /// 对端在请求完成前断开（stdin 关闭、插件重启）。
    pub fn disconnected() -> Self {
        Self::new("disconnected", "对端已断开")
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// 一次挂起的出站请求。
struct Pending {
    chunks: Option<mpsc::UnboundedSender<Value>>,
    done: oneshot::Sender<Result<Value, RpcError>>,
}

/// 出站请求的句柄：流式块 + 最终结果。
pub struct OutboundCall {
    pub id: String,
    pub chunks: mpsc::UnboundedReceiver<Value>,
    pub done: oneshot::Receiver<Result<Value, RpcError>>,
}

/// 入站消息分类结果。
#[derive(Debug)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// 已路由给挂起的出站请求（或无人认领，已丢弃）。
    Routed,
    Invalid(String),
}

pub struct Peer {
    out: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Peer {
    /// `out` 的接收端由外壳写到管道（每条一行）。
    pub fn new(out: mpsc::UnboundedSender<String>) -> Self {
        Self {
            out,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn send(&self, message: Value) {
        // 外壳已退出时丢弃即可：挂起的请求会在 `fail_all` 里收到 disconnected。
        let _ = self.out.send(message.to_string());
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "method": method, "params": params }));
    }

    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        match result {
            Ok(result) => self.send(json!({ "id": id, "result": result })),
            Err(error) => self.send(json!({ "id": id, "error": error })),
        }
    }

    /// 发起请求（`params` 原样发送），返回块接收端与结果接收端。
    pub fn call(&self, method: &str, params: Value) -> OutboundCall {
        let id = format!("s-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (chunk_tx, chunk_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.clone(),
                Pending {
                    chunks: Some(chunk_tx),
                    done: done_tx,
                },
            );
        self.send(json!({ "id": id, "method": method, "params": params }));
        OutboundCall {
            id,
            chunks: chunk_rx,
            done: done_rx,
        }
    }

    /// 放弃一个挂起的请求（取消后不再关心结果）。
    pub fn forget(&self, id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    /// 管道断开：所有挂起请求以 disconnected 结束。
    pub fn fail_all(&self) {
        let drained: Vec<Pending> = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain()
            .map(|(_, pending)| pending)
            .collect();
        for pending in drained {
            let _ = pending.done.send(Err(RpcError::disconnected()));
        }
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// 解析一行入站消息：响应与 `*.chunk` 通知就地路由，其余交给调用方处理。
    pub fn accept_line(&self, line: &str) -> Incoming {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => return Incoming::Invalid(format!("不是 JSON: {error}")),
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = value.get("id").cloned().filter(|id| !id.is_null());
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(method), Some(id)) => Incoming::Request { id, method, params },
            (Some(method), None) if method == "llm.chunk" => {
                self.route_chunk(&params);
                Incoming::Routed
            }
            (Some(method), None) => Incoming::Notification { method, params },
            (None, Some(id)) => {
                self.route_response(&id, &value);
                Incoming::Routed
            }
            (None, None) => Incoming::Invalid("既没有 method 也没有 id".into()),
        }
    }

    fn route_chunk(&self, params: &Value) {
        let Some(id) = params.get("id").and_then(Value::as_str) else {
            return;
        };
        let guard = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(sender) = guard.get(id).and_then(|pending| pending.chunks.as_ref()) {
            let _ = sender.send(params.get("chunk").cloned().unwrap_or(Value::Null));
        }
    }

    fn route_response(&self, id: &Value, message: &Value) {
        let Some(id) = id.as_str() else {
            return;
        };
        let Some(pending) = self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
        else {
            return;
        };
        let result = match message.get("error") {
            Some(error) if !error.is_null() => Err(serde_json::from_value(error.clone())
                .unwrap_or_else(|_| RpcError::new("invalid_error", error.to_string()))),
            _ => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = pending.done.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> (Peer, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Peer::new(tx), rx)
    }

    #[tokio::test]
    async fn outbound_call_routes_chunks_then_result() {
        let (peer, mut out) = peer();
        let mut call = peer.call("llm.stream", json!({ "x": 1 }));
        let sent: Value = serde_json::from_str(&out.recv().await.unwrap()).unwrap();
        assert_eq!(sent["method"], "llm.stream");
        assert_eq!(sent["id"], call.id.as_str());

        let chunk = json!({ "method": "llm.chunk", "params": { "id": call.id, "chunk": { "type": "text", "text": "a" } } });
        assert!(matches!(
            peer.accept_line(&chunk.to_string()),
            Incoming::Routed
        ));
        let done = json!({ "id": call.id, "result": { "content": "a" } });
        assert!(matches!(
            peer.accept_line(&done.to_string()),
            Incoming::Routed
        ));

        assert_eq!(call.chunks.recv().await.unwrap()["text"], "a");
        assert_eq!(call.done.await.unwrap().unwrap()["content"], "a");
        assert_eq!(peer.pending_count(), 0);
    }

    #[tokio::test]
    async fn error_response_and_disconnect_fail_pending_calls() {
        let (peer, _out) = peer();
        let failed = peer.call("llm.stream", Value::Null);
        let error = json!({ "id": failed.id, "error": { "code": "rate_limited", "message": "slow down", "status": 429 } });
        peer.accept_line(&error.to_string());
        let error = failed.done.await.unwrap().unwrap_err();
        assert_eq!(error.status, Some(429));

        let orphan = peer.call("llm.stream", Value::Null);
        peer.fail_all();
        assert_eq!(orphan.done.await.unwrap(), Err(RpcError::disconnected()));
    }

    #[test]
    fn classifies_requests_and_notifications() {
        let (peer, _out) = peer();
        assert!(matches!(
            peer.accept_line(r#"{"id":1,"method":"turn.run","params":{}}"#),
            Incoming::Request { .. }
        ));
        assert!(matches!(
            peer.accept_line(r#"{"method":"bridge.ping"}"#),
            Incoming::Notification { .. }
        ));
        assert!(matches!(peer.accept_line("not json"), Incoming::Invalid(_)));
        // 无人认领的响应直接丢弃，不报错。
        assert!(matches!(
            peer.accept_line(r#"{"id":"s-99","result":null}"#),
            Incoming::Routed
        ));
    }
}
