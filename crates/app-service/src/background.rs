//! 后台任务派发：宿主可安装自己的 tokio 运行时句柄。
//!
//! 原先直接用 `tauri::async_runtime::spawn`（它在没有运行时上下文的线程上也能派发，
//! 首次调用时惰性创建全局运行时）。抽出到宿主无关层后保持同一语义：优先用宿主安装的
//! 句柄（桌面端装 tauri 的运行时），否则惰性创建一个进程级后台运行时。

use std::future::Future;
use std::sync::OnceLock;

static HOST_RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// 安装宿主的运行时句柄（只生效一次，重复安装返回 false）。
pub fn install_background_runtime(handle: tokio::runtime::Handle) -> bool {
    HOST_RUNTIME.set(handle).is_ok()
}

fn handle() -> tokio::runtime::Handle {
    if let Some(handle) = HOST_RUNTIME.get() {
        return handle.clone();
    }
    FALLBACK_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("storyforge-background")
                .build()
                .expect("failed to build the background tokio runtime")
        })
        .handle()
        .clone()
}

/// 在后台运行时上派发任务；可在任意线程（含无运行时上下文的线程）调用。
pub fn spawn_background<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle().spawn(future)
}
