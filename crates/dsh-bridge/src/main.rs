//! storyforge-dsh-bridge：DSH 插件版 PoC 的 sidecar。
//!
//! 用法：`storyforge-dsh-bridge --data-dir <目录>`。由 DSH 插件宿主半边拉起，
//! stdin/stdout 每行一个 JSON-RPC 消息（协议见库文档）；日志只写 stderr。

use std::path::PathBuf;

fn parse_data_dir() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            return args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--data-dir 缺少参数".to_string());
        }
    }
    Err("用法：storyforge-dsh-bridge --data-dir <目录>".into())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_dir = match parse_data_dir() {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    // 后台任务（向量索引补嵌入等）派发到本进程的运行时，不另起一个。
    storyforge_app_service::background::install_background_runtime(
        tokio::runtime::Handle::current(),
    );
    if let Err(message) = storyforge_dsh_bridge::run_stdio(data_dir).await {
        tracing::error!("初始化失败: {message}");
        std::process::exit(1);
    }
}
