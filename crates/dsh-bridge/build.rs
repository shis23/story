fn main() {
    // 依赖 tauri-app 库会链入 comctl32 v6 才有的入口（TaskDialogIndirect）；不声明
    // Common-Controls v6 清单依赖，Windows 上进程启动即失败（0xC0000139）。
    // 与 crates/tauri-app/build.rs 相同：按 TARGET（而非 host cfg）门控。另加
    // /MANIFEST:EMBED：默认生成的旁置 .manifest 按链接时的文件名命名，cargo 复制
    // 出的 `storyforge-dsh-bridge.exe` 匹配不上，必须嵌进 exe。
    let target_is_windows = std::env::var("TARGET")
        .map(|t| t.contains("windows"))
        .unwrap_or(false);
    if target_is_windows {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
