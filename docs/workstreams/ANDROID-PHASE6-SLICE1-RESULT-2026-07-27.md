# Android Phase 6 第一切片结果（2026-07-27）

> 分支：`codex/android-phase6-slice1`（独立 worktree，未 push）
> Base SHA：`29513a600a404563ef5aad30ad52fa97b3d0c90a`（开工时最新 main）
> 本切片范围：AND-2 系统选择器导入、Android keyring 冒烟就绪度、诊断包 save/share。
> **真机未连接**：所有 PASS 限于 compiled / desktop-test 层；emulator 与 physical-device 列为 BLOCKED。

## 0. 真机/设备连接状态

- **adb 设备列表**：空（`adb devices` 启动 daemon 后无设备 attached）。
- 环境：本机 `ANDROID_HOME=C:\Users\Predator\android-sdk`，NDK `27.2.12479018` 存在，`adb.exe` 存在（不在 PATH），rustup 已装 `aarch64-linux-android` 等 4 个 Android target。
- **结论**：本切片无真机/emulator 证据，凡涉及运行时行为的项一律不得记为 PASS。

## 1. 三项交付状态

| 项 | 状态 | 证据层级 | 说明 |
| --- | --- | --- | --- |
| AND-2 系统选择器导入 PNG/JSON | **PARTIAL** | compiled + desktop-test | 导入路径本就是字节缓冲（不依赖临时 URI 长期存活），架构 Android 友好；本轮补 `get_app_data_dir()` 的 Android 分支，使导入内容落到应用私有沙箱；真机导入（中文文件名、大文件、Downloads/Documents）未验。 |
| Android keyring SecretRef 写/读/删 | **PARTIAL（编译通过）** | compiled (aarch64) | `android-native-keyring-store` 后端已接线，`cargo check --target aarch64-linux-android` 通过；真机写/读/删未跑（无设备）。 |
| 诊断包 save/share | **PARTIAL** | compiled + desktop-test | `log_export_bundle` 返回脱敏 JSON（不含 API key，已有回归测试固化）；Android 走 SAF `save()` 文档选择器，**无 share sheet**；FileProvider 脚手架存在但未被代码使用；真机 save/share 未验。 |

> 本切片不使用“应该可用”作为 PASS。三项运行时真机行为均为 BLOCKED（无设备），上表 PARTIAL 仅指已完成的编译与桌面测试部分。

## 2. 代码改动（最小差异）

改动文件（2 个，+98/-3）：

- `crates/tauri-app/src/lib.rs`
  - `get_app_data_dir()`：新增 `cfg!(target_os = "android")` 分支。优先读 `STORYFORGE_DATA_DIR` 环境变量（真机冒烟探针/调试覆盖），否则落到 `/data/data/com.storyforge.app/files`（与 Tauri `app_data_dir()` 一致）。修复前 Android 会落入 `else`（XDG）分支解析到无意义路径或 exe_dir 回退（只读 APK 内）。
  - 新增测试 `get_app_data_dir_always_returns_created_dir_and_never_exe_fallback_on_desktop`：固化桌面侧不回归（解析结果必须是已创建目录；桌面不回退 exe_dir/data）。Android 常量分支由代码评审 + 真机冒烟覆盖，不在桌面断言。
- `crates/infra-import/src/lib.rs`
  - 新增测试 `and2_import_accepts_only_bytes_independent_of_source_uri`：固化 AND-2 契约——`import_character(&[u8])` 只接收字节、从不接收路径/URI，因此选择器返回的临时 `content://` URI 被回收与导入成功无关；中文角色名内容可解析；`MAX_IMPORT_SIZE` 边界（等于上限通过 size guard 仅在格式判断失败、超上限被拒）。

未修改：`crates/tauri-app/src/lib.rs` 中任何 `#[tauri::command]` 签名（`import_character` 已是字节入参，无需改）、`tauri.conf.json`（CSP 不在本切片）、`.gitea/workflows/**`、写作流水线、SQLite 后端、`gen/android`。

## 3. 三项详情与现有事实核对

### 3.1 AND-2 系统选择器导入

**已实现（无需本轮改动）**：
- 导入入口 `frontend/src/composables/useCharacterImport.js::handleImport`：`@tauri-apps/plugin-dialog` `open()` → `@tauri-apps/plugin-fs` `readFile(filePath)` 读入 `Uint8Array` → `importCharacter(data)`（`frontend/src/tauri-api.js:12`）→ `invoke('import_character', { data: Array.from(data) })`。
- Rust `import_character(data: Vec<u8>, ...)`（`lib.rs:1071`）：纯字节解析（PNG tEXt chara/ccv3 块 或 JSON），`MAX_IMPORT_SIZE=100MiB` size guard，解析后 `CharacterInfo` 落 `characters.json`。**源文件 URI 不被后端持有**，导入完成即与选择器临时 URI 解耦。
- capability（`capabilities/default.json`）：`dialog:allow-open` + `fs:allow-read-file`，无 `fs:scope` 限制（依赖 dialog 授权的 scoped 访问）。

**本轮修复**：
- `get_app_data_dir()` 缺 Android 分支 → 导入的 `characters.json` 在 Android 上会落到错误位置。已补 Android 分支（见 §2）。

**真机未验（BLOCKED）**：中文文件名、较大文件（接近 100MiB）、从 Downloads/Documents/第三方文件管理器选择的实际 `content://` URI 是否被 Tauri fs plugin 的 `readFile` 正确解析。这些是运行时行为，无设备不得宣称通过。

### 3.2 Android keyring（SecretRef 写/读/删）

**已实现（compiled 通过）**：
- `crates/infra-util/src/secret_store.rs`：`SecretStore` trait（`put_secret`/`get_secret`/`delete_secret`），`SystemSecretStore` 经 `keyring` crate 操作系统凭据库。`ensure_native_store()` 按 `cfg(target_os=...)` 选后端：Android → `android_native_keyring_store::Store::new()`（`secret_store.rs:60-66`）。服务名 `StoryForge`，account = 完整 SecretRef 串（如 `storyforge-secret:v1:llm-connection:<id>`）。
- Cargo：workspace `keyring = { version = "4.1.3", features = ["android-native-keyring-store"] }`；`crates/infra-util/Cargo.toml:17-18` 声明 Android target dep。
- 消费链：`ConnectionStore`（`connection_store.rs`）`save`/`set_active`/`delete` 经 `secure_api_key`/`resolve_secret_value`/`delete_secret` 读写 keyring；`embed.json` 同模式（`EMBED_SECRET_KIND="embedder"`）。
- 参考冒烟：`system_keyring_write_read_delete_roundtrip`（`secret_store.rs:153`，`#[ignore]`，写真实 OS 凭据库）。Windows Credential Manager 冒烟已记录通过；Android 同形测试需真机。

**编译门（本轮实跑，PASS）**：
```
cargo check -p storyforge-infra-util --target aarch64-linux-android
# Finished dev profile in 13.63s（含 android-native-keyring-store 1.0.0、keyring 4.1.3、jni 0.21.1）
```

**真机未验（BLOCKED）**：Android Keystore provider 实际可用性、`Store::new()` 是否在目标设备成功初始化、写/读/删 roundtrip。`ensure_native_store` 失败会缓存错误并让所有后续 secret 操作返回 `Err`（无 fallback）——冒烟必须能检出而非假设成功。

**用户/后续最短真机冒烟步骤**（接真机后执行）：
1. `cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk`（生成 `app-arm64-debug.apk`）。
2. `adb install -r app-arm64-debug.apk` 启动 app。
3. 在连接配置页创建一个 OpenAI 兼容连接（填测试 key），设为 active。
4. `adb shell run-as com.storyforge.app ls files/` 确认 `connections.json` 存在且**不含明文 key**（只含 `storyforge-secret:v1:llm-connection:*`）。
5. 重启 app，确认 active 连接仍可解析（keyring 读成功）；删除连接后确认 keyring 条目被清。

### 3.3 诊断包 save/share

**已实现（compiled + desktop-test PASS）**：
- `log_export_bundle(redact_content, state)`（`lib.rs:7212`）返回 `serde_json::Value`：`export_bundle()`（`app-logging/src/lib.rs:398`）含 `system_info`（os/arch/app_version）、`backend_logs`/`llm_logs`/`frontend_logs`（`redact_content=true` 时 LLM payload 用 `<content N chars>` 占位）、`counts`；命令再注入 `diagnostic_context`（`diagnostic_context_for_data_dir`，`lib.rs:7261`）——23 个 store 文件**仅记 name/exists/bytes/has_bytes 元数据，不读内容**。
- 前端入口 `frontend/src/components-v2/debug/LogPanel.vue::handleExport`（L141）：`logExportBundle(true)` → `JSON.stringify` → `plugin-dialog` `save()` → `plugin-fs` `writeBinaryFile()`。
- **无密钥泄露**：`LlmCallDetail` 无 api_key 字段；store 文件摘要不读内容。已有回归测试 `test_diagnostic_context_summarizes_stores_without_secret_values`、`test_diagnostic_export_bundle_summarizes_secret_stores_without_leaking_keys`（本轮实跑均 PASS，覆盖 `connections.json`/`embed.json` 内植入的明文 key 与 Bearer）。

**Android save/share 现状（真机未验）**：
- Android 上 `plugin-dialog` `save()` + `plugin-fs` `writeBinaryFile()` 走系统文档选择器（SAF / DocumentsUI），**不是 share sheet**。
- `AndroidManifest.xml` 已声明 `FileProvider`（authority `${applicationId}.fileprovider`，`file_paths.xml` 暴露 external-path/cache-path），但**全仓库无任何 `ACTION_SEND` / `FileProvider.getUriForFile` 调用**——脚手架存在但未接线。若产品需要 share sheet（分享到聊天/邮件），需新增 Tauri share 插件或 Kotlin intent；**不在本切片范围**。

**真机未验（BLOCKED）**：SAF save 在目标 Android 版本是否成功写出 JSON、文件名/路径是否符合用户预期。

## 4. 验证命令与结果

| 命令 | 结果 |
| --- | --- |
| `cargo test -p storyforge-infra-import --lib` | 56 passed, 0 failed, 1 ignored（含新增 `and2_import_accepts_only_bytes_independent_of_source_uri`） |
| `cargo test -p storyforge-infra-util` | 6 passed, 0 failed, 1 ignored（`#[ignore]` 的真机 keyring roundtrip） |
| `cargo test -p storyforge --test capabilities` | 2 passed（capability 收窄 + 前端 helper 匹配） |
| `cargo test -p storyforge --lib connection_store` | 11 passed（SecretRef 写/读/删/迁移/删除级联） |
| `cargo test -p storyforge --lib diagnostic` | 2 passed（诊断包脱敏，无密钥泄露） |
| `cargo test -p storyforge --lib get_app_data_dir_always_returns_created_dir_and_never_exe_fallback_on_desktop` | 1 passed（新增，桌面 data_dir 解析不回归） |
| `cargo check -p storyforge-infra-util --target aarch64-linux-android` | Finished（Android aarch64 编译门通过，含 keyring Android 后端） |
| `npm run build`（frontend） | built in 6.07s（dist 生成，tauri generate_context 依赖） |
| `git diff --check` | exit 0（无空白错误） |

> 工作树 `frontend/node_modules` 与 `frontend/dist` 为本地构建产物，未入库（`.gitignore` 已忽略 `frontend/node_modules/`、`frontend/dist/`）。生成的 `gen/schemas/*.json` CRLF 变更已 revert，不入本次提交。

## 5. 明确剩余阻塞

1. **真机/emulator 缺失**：AND-2 导入、keyring 写/读/删、诊断包 save 三项的运行时行为全部待真机验证。本切片仅完成编译与桌面测试。
2. **Android keyring 真机可用性**：`android_native_keyring_store::Store::new()` 在目标设备是否成功初始化未知；失败则所有连接密钥无法解析，属主流程阻塞风险（OPEN-ISSUES 已记 medium）。
3. **诊断包 share sheet**：当前只有 SAF save，无 share intent；若产品需分享到其他 app，需单独工程（不在本切片）。
4. **`get_app_data_dir()` Android 路径常量**：`/data/data/com.storyforge.app/files` 为约定路径，与 Tauri `app_data_dir()` 一致；多用户/特殊 ROM 下应以 `STORYFORGE_DATA_DIR` 覆盖或改用 `app.path()` 注入（后者需重构多个 OnceLock 初始化点，超出本切片最小差异）。

## 6. 纪律遵守

- 独立 worktree/分支，未 reset/checkout/覆盖其他代理改动。
- 未修改禁止项：`.gitea/workflows/**`、`tauri.conf.json` CSP、写作流水线、SQLite 后端、`EXECUTION-PROGRAM-2026-07-27.md`、`gen/android`。
- `crates/tauri-app/src/lib.rs` 改动保持最小差异，仅 `get_app_data_dir()` 一个函数 + 一个测试，已在 RESULT 点名。
- 未 push。
- 无 API key / 设备私密数据进入补丁、文档或 artifact。
