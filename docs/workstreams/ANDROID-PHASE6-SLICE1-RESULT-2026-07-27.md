# Android Phase 6 Slice 1 — 合并复核结果（2026-07-27）

## 结论

分支 `codex/android-phase6-slice1`（head `9863595`）已通过 merge commit
`9c19418` 合入本地 `main`，随后完成主线复核修正。

代码层结论为 **PASS**；APK、模拟器和真机层仍为 **BLOCKED**，未把桌面测试或
交叉编译检查冒充设备验收。

## 合并后复核修正

### Android 数据目录

原分支把默认目录写成 `/data/data/com.storyforge.app/files`。这个路径在 Android
多用户、工作资料和厂商实现下不可靠，也不等同于 Tauri 运行时解析结果。

主线修正后：

- 在 `tauri::Builder::setup` 内调用 `app.path().app_data_dir()`，使用运行中
  Android Context 解析出的真实目录。
- 用 `OnceLock<PathBuf>` 保存一次初始化结果；后续 store 共用同一路径。
- `STORYFORGE_DATA_DIR` 仍是显式测试/特殊环境覆盖口。
- Android 在 setup 前拿不到框架路径时失败关闭，不再回退到硬编码包路径。
- storage backend、`AppState`、tracing 和 recovery 都移动到路径初始化之后。
- 纯函数测试覆盖 `/data/user/10/...` 多用户路径、显式覆盖和缺少框架路径的失败关闭。

### 其他原分支修正

- 导入大小边界改用纯 `check_import_size(len, max)`，测试不再同时分配约 200 MiB。
- 数据目录迁移测试全部使用临时目录，不读写真实 `%APPDATA%`/HOME。
- `build.rs` 用 Cargo 的 `TARGET` 判断是否注入 Windows manifest 链接参数，避免
  Windows 主机交叉编译 Android 时把 MSVC 参数传给 clang。

## 验证

- `cargo fmt --all -- --check`：PASS。
- `cargo test --workspace`：PASS；需要真实 LLM/设备的测试保持 ignored。
- `cargo test -p storyforge --lib resolve_app_data_dir`：3 passed。
- `cargo test -p storyforge --lib migrate_from_exe_dir`：2 passed。
- `cargo test -p storyforge-infra-import --lib`：PASS。
- `cargo test -p storyforge-infra-util`：PASS（真机 keyring roundtrip ignored）。

## 尚未验证

- Android APK 整体构建：本机 Windows 交叉编译此前运行 70+ 分钟未产出 APK，本轮不
  复述为 PASS。
- 模拟器/真机：`content://` picker（含中文文件名、Downloads/Documents、大文件）、
  keyring 写读删 roundtrip、SAF save/share 均未运行。
- 无设备证据，因此 Android runtime 结论保持 BLOCKED。
