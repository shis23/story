fn main() {
    // /MANIFESTDEPENDENCY 只能传给 Windows 桌面链接器（MSVC/link.exe）。
    // 旧实现在 `#[cfg(target_os = "windows")]` 下无条件 println，但 build.rs
    // 是为 **host** 编译的：在 Windows 主机上交叉编译 Android 时，host cfg
    // 为真，cargo:rustc-link-arg 却会注入到 *target*（Android clang）的链接
    // 命令，导致 `clang: error: no such file or directory: '/MANIFESTDEPENDENCY...'`
    // 因此改用 Cargo 在调用 build script 时设置的 TARGET 环境变量做目标门控，
    // 仅当目标真的是 Windows 桌面时才发出该链接参数。
    let target_is_windows = std::env::var("TARGET")
        .map(|t| t.contains("windows"))
        .unwrap_or(false);
    if target_is_windows {
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }

    tauri_build::build()
}
