# Android Phase 6 第一切片结果（2026-07-27，返修版）

> 分支：`codex/android-phase6-slice1`（独立 worktree，**未 push**）
> Base SHA：`29513a600a404563ef5aad30ad52fa97b3d0c90a`（开工时最新 main）
> 本切片范围：AND-2 系统选择器导入、Android keyring 冒烟就绪度、诊断包 save/share。
>
> **真机/emulator 未连接**：凡运行时行为（picker、keyring roundtrip、SAF save/share）一律 BLOCKED。
> 本文严格区分四层证据：**desktop-test / Android-app-compiled / emulator / physical-device**，不用“应该可用”当 PASS。

## 0. SHA 与 code-under-test

- Base：`29513a600a404563ef5aad30ad52fa97b3d0c90a`
- Head（返修后，提交前）：见 `git log` 最新 commit（本文件随最后一个 commit 一起入库）。
- Code-under-test：本切片改动 = `git diff main...HEAD` 在 `crates/infra-import/src/lib.rs`、`crates/tauri-app/src/lib.rs`、`crates/tauri-app/build.rs` 三处的实际内容（不含 `Cargo.toml`/`gen/schemas` 的 CRLF 噪音，已 revert）。

## 1. 真机/设备与 Android 编译环境状态

- **adb 设备列表**：空（`adb devices` 无设备 attached）。
- 本机：`ANDROID_HOME=C:\Users\Predator\android-sdk`，NDK `27.2.12479018`，`adb.exe` 存在（不在 PATH），rustup 已装 `aarch64-linux-android` 等 4 个 Android target，Tauri CLI `2.11.2`，JDK 17/21 均在。
- **结论**：无真机/emulator；运行时行为全部 BLOCKED。

## 2. 四项交付状态（按证据层级标注）

| 项 | desktop-test | Android-app-compiled | emulator | physical-device |
| --- | --- | --- | --- | --- |
| AND-2 导入 parser 字节契约 | ✅ PASS（`infra-import` 57 passed） | n/a（纯逻辑，无平台依赖） | BLOCKED | BLOCKED |
| AND-2 `get_app_data_dir()` Android 分支 | ✅ PASS（纯路径解析 + 迁移测试） | ⚠️ build.rs 链接门控已修并独立验证；**APK 整体构建 BLOCKED**（见 §4） | BLOCKED | BLOCKED |
| Android keyring（SecretRef 写/读/删） | ✅ PASS（connection_store 11 + secret_store 单测） | ✅ PASS（`cargo check -p storyforge-infra-util --target aarch64-linux-android`） | BLOCKED | BLOCKED |
| 诊断包 save/share + 脱敏 | ✅ PASS（diagnostic 2 passed，无密钥泄露） | n/a（逻辑无平台依赖；save 走 SAF 是运行时行为） | BLOCKED | BLOCKED |

> **没有任何一项被标为真机/emulator PASS。** Android-app-compiled 列里 keyring 后端编译通过；APK 整体构建 BLOCKED。

## 3. 返修内容（对应五条审查问题）

### 3.1 修复非隔离测试（审查 #1）

**问题**：旧测试 `get_app_data_dir_always_returns_created_dir_and_never_exe_fallback_on_desktop` 直接调生产 `get_app_data_dir()`，会创建真实 `%APPDATA%/StoryForge` 并可能触发 `migrate_from_exe_dir_if_needed()`。

**修复**（`crates/tauri-app/src/lib.rs`）：
- 抽出纯路径解析函数 `resolve_app_data_dir(target, env, exe_parent)`（`:360`）：显式接收 `AppDataDirTarget` 枚举（`:330`）、环境变量视图闭包、exe 父目录；**无副作用**（不读真实环境、不建目录、不迁移）。
- `get_app_data_dir()`（`:410`）改为薄封装：读真实环境 → 调纯函数 → 建目录 → 迁移。
- `migrate_from_exe_dir_if_needed(new_dir, exe_parent)`（`:432`）改为显式接收 `exe_parent`（不再内部 `current_exe()`），便于用临时目录测试。
- 新增 5 个隔离测试，**全部不调生产 `get_app_data_dir()`、不改进程环境变量、不碰真实 `%APPDATA%`/`$HOME`**：
  - `resolve_app_data_dir_returns_os_standard_paths_per_platform`：Win/Mac/Linux 纯解析。
  - `resolve_app_data_dir_android_prefers_env_override_then_sandbox_files`：Android env 覆盖 + 默认。
  - `resolve_app_data_dir_falls_back_to_exe_data_when_env_missing`：exe 回退（合成路径）。
  - `migrate_from_exe_dir_copies_when_new_dir_empty_and_skips_when_populated`：迁移幂等/不覆盖（临时目录）。
  - `migrate_from_exe_dir_is_noop_when_old_dir_absent`：fresh install 不报错（临时目录）。

### 3.2 修复高内存边界测试（审查 #2）

**问题**：旧 AND-2 测试同时持有 `at_limit`（100MiB）+ `over_limit`（100MiB+1）≈ 200 MiB。

**修复**（`crates/infra-import/src/lib.rs`）：
- 抽出纯 size guard `check_import_size(data_len, max)`（`:37`），`import_character` 复用它。
- 新测试 `and2_import_size_guard_boundary_uses_strict_greater_than` 用 **小阈值 16 字节**验证边界（等于上限通过、超限被拒、文案含“文件过大”与上限字节数），不再分配大缓冲。
- 字节契约测试 `and2_import_accepts_only_bytes_independent_of_source_uri` 只保留中文内容解析（小数据）。
- **生产 `MAX_IMPORT_SIZE`（100MiB）未降低。**

### 3.3 修复 Android 数据目录实现（审查 #3）

**约束核实（代码级证据，非猜测）**：调研 tauri `2.11.2` 源码确认——`get_app_data_dir()` 在 `run()` 最开始（`lib.rs:13309`，早于 `tauri::Builder::default()` 与 `.setup()`）被调用；而 `app.path().app_data_dir()` 在 Android 上经 `PathResolver → PluginHandle → AppHandle → Kotlin PathPlugin`（JNI，需运行中的 Activity）解析（tauri `src/path/android.rs:137`、`src/path/plugin.rs:236`、`src/plugin/mobile.rs:317`），**Builder 构造前完全不可用**；Tauri 也不设置任何环境变量。因此 **Builder 前注入框架路径在 tauri 2.11.2 上不可行**，唯一可行设计是“可覆盖环境变量 + 应用私有 files 目录常量”。

**修复**：
- 不再宣称硬编码路径“与 Tauri `app_data_dir()` 完全一致”。**核实 tauri `PathPlugin.getDataDir` 实际返回 `Context.getDataDir()`（`/data/data/com.storyforge.app`，不含 `/files`）**，与本目录为父子关系；已把 docstring 改为如实描述二者关系与收敛条件。
- `STORYFORGE_DATA_DIR` 环境变量覆盖保留，作为真机冒烟/特殊 ROM/多用户的逃生口。
- **长期演进门（写入本文作为 backlog，不在本切片实施）**：要改用框架 `app.path().app_data_dir()` 注入，须重构多个 `OnceLock` store 初始化点（`STORE`/`CONN_STORE`/`CAMPAIGN_STORE` 等，`lib.rs:111-197`），把首解析推迟到 `.setup()` 闭包内、并把 `storage_backend::resolve_backend` 与 `AppState::new()` 一起挪到 setup。这是本切片“最小差异、不碰 SQLite/CSP/写作流水线”约束之外的工程，标为后续。

### 3.4 Android 应用级构建（审查 #4）

**最短命令**（本机已尝试）：
```bash
cd crates/tauri-app
export ANDROID_HOME=... ANDROID_NDK_HOME=.../ndk/27.2.12479018 NDK_HOME=... JAVA_HOME=...
cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk
```

**结果：BLOCKED（未产出 APK）。**
- 首次尝试暴露并修复了一个 **真实链接器 bug**（不是我的业务代码）：`crates/tauri-app/build.rs` 旧实现在 `#[cfg(target_os = "windows")]` 下无条件 `println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:...")`。在 Windows 主机上交叉编译 Android 时，`build.rs` 是为 **host** 编译的（host cfg=windows 为真），但 `cargo:rustc-link-arg` 会注入到 *target*（Android clang）链接命令，导致：
  `clang: error: no such file or directory: '/MANIFESTDEPENDENCY:type=win32 name=Microsoft.Windows.Common-Controls ...'`
- **修复**（`build.rs`）：改用 Cargo 调用 build script 时设置的 `TARGET` 环境变量做目标门控——仅当 `TARGET` 含 `windows` 时才发出该链接参数。
- **修复已独立验证**（不依赖完整 APK 构建）：用一个等价的 `TARGET` 门控小程序确认 `TARGET=aarch64-linux-android → target_is_windows=false`（不发出）、`TARGET=x86_64-pc-windows-msvc → target_is_windows=true`（桌面仍正常发出，无回归）；桌面 `cargo build --lib` 干净通过。
- **APK 整体构建 BLOCKED 原因**：Windows 主机交叉编译 NDK（全量编译 ring/libsqlite3 native + Rust aarch64 debug + gradle）耗时极长，本次实跑 70+ 分钟未完成且被中断，**未产出 `libstoryforge_lib.so`/`.apk`**。本地 Windows 不是合适的 Android 编译验证主机；建议改用 Linux CI runner（见 §6）。
- **不得**用桌面 `cargo check` 冒充 Android app compiled。本轮 Android app 级证据仅到：build.rs 链接门控修复 + 独立验证 + `cargo check -p storyforge-infra-util --target aarch64-linux-android`（keyring 后端）通过。

### 3.5 文档证据分级（审查 #5）

本文 §2 表格已按 desktop-test / Android-app-compiled / emulator / physical-device 四层分别标注。明确：
- **AND-2 parser 字节契约测试只证明 parser 层**（`import_character(&[u8])` 只收字节、不收路径），**不证明** content:// picker / readFile 的真机链路。后者是 Android 运行时行为，BLOCKED。
- picker、keyring roundtrip、SAF save/share 在 emulator/physical-device 两列均 BLOCKED。

## 4. 验证命令与结果（desktop-test + Android 编译门）

| 命令 | 结果 |
| --- | --- |
| `cargo test -p storyforge-infra-import --lib` | **57 passed**, 0 failed, 1 ignored（含新增 `and2_import_*` 两个测试） |
| `cargo test -p storyforge-infra-util` | 6 passed, 0 failed, 1 ignored（`#[ignore]` 真机 keyring roundtrip） |
| `cargo test -p storyforge --test capabilities` | 2 passed |
| `cargo test -p storyforge --lib connection_store` | 11 passed |
| `cargo test -p storyforge --lib diagnostic` | 2 passed（诊断包脱敏，无密钥泄露） |
| `cargo test -p storyforge --lib resolve_app_data_dir` | 3 passed（新增，纯路径解析，不碰真实环境） |
| `cargo test -p storyforge --lib migrate_from_exe_dir` | 2 passed（新增，临时目录，不碰真实数据） |
| `cargo check -p storyforge-infra-util --target aarch64-linux-android` | Finished（keyring Android 后端编译通过） |
| `build.rs` TARGET 门控独立验证 | Android→false(不发出)、Windows桌面→true(正常)，桌面 `cargo build --lib` 干净 |
| `cargo tauri android build --debug --target aarch64 ... --apk` | **BLOCKED**（见 §3.4：链接 bug 已修，但 Windows 交叉编译 70+ min 未完成/中断，无 APK） |
| `git diff --check` | exit 0 |

> 工作树 `frontend/node_modules`、`frontend/dist` 为本地构建产物（`.gitignore` 已忽略）；`Cargo.toml`/`gen/schemas/*.json` 的 CRLF 变更已 revert，不入提交。

## 5. 明确剩余阻塞

1. **Android app 级 APK 构建未产出**（BLOCKED）：build.rs 链接 bug 已修，但本机 Windows 交叉编译未跑完。建议改 Linux CI（见 §6）。
2. **真机/emulator 缺失**：AND-2 picker 导入（中文文件名/大文件/Downloads/Documents 的 content://）、keyring 写/读/删 roundtrip、SAF save/share 全部待真机。
3. **Android keyring 真机可用性**：`android_native_keyring_store::Store::new()` 在目标设备是否成功未知；失败则所有连接密钥无法解析（无 fallback，主流程阻塞风险）。
4. **诊断包 share sheet**：当前只有 SAF save，无 share intent（FileProvider 脚手架存在但未接线）；若需分享到其他 app 是单独工程，不在本切片。
5. **框架路径注入门（backlog）**：要弃用 `/data/data/com.storyforge.app/files` 常量改用 `app.path().app_data_dir()`，须重构 OnceLock store 初始化顺序（§3.3），超出本切片最小差异约束。

## 6. 关于 Android 构建改用 GitHub/Gitea Actions（用户已确认方向）

用户问“安卓构建是否可以在 github 上做”。结论：**可以且更合适**，但有约束：
- `.gitea/workflows/**` 归 Release CI 线，本切片**不改**。
- 可在 `.github/workflows/` 加一个**仅本切片用的 Android aarch64 APK 编译验证** workflow（文件所有权归 Android 线），不签名、不发布、不碰 release。
- **本切片按用户决定（“先交付代码，Android 构建标 BLOCKED”）处理**：不新增 CI workflow、不 push。APK 构建在本文标 BLOCKED，待后续接 CI/真机时跑。

## 7. 纪律遵守

- 独立 worktree/分支，未 reset/checkout/覆盖其他代理改动，未 rebase/push/force-push。
- 未修改禁止项：`.gitea/workflows/**`、`tauri.conf.json` CSP、写作流水线、SQLite 后端、`EXECUTION-PROGRAM-2026-07-27.md`、`gen/android` 业务逻辑。
- `crates/tauri-app/src/lib.rs` 改动保持最小差异：纯路径解析抽取 + 测试隔离 + Android 分支语义订正；`build.rs` 仅修链接门控。
- 未 push。
- 无 API key / 设备私密数据进入补丁或文档。
