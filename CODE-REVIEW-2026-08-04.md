# StoryForge 全量代码审查报告

**审查范围**：StoryForge 项目全部代码（~17.5 万行 Rust + ~3 万行前端 + 脚本/CI/文档）
**审查方法**：6 个并行子代理分领域审查 + 主审逐项复核关键发现（grep/精读源码验证）
**审查日期**：2026-08-04
**审查人**：ZCode（主审 + Sonnet 子代理）
**仓库状态**：分支 `main`，提交 `028acd9`（working tree clean）

---

> ## 🔧 修复状态（2026-08-04 追加）
>
> 本报告已全量复审并修复，详见 **`CODE-REVIEW-FIXES-2026-08-04.md`**。
>
> | 级别 | 总数 | 已修复 | 延期/降级 |
> |------|------|--------|-----------|
> | High | 7 | **7 ✅** | 0 |
> | Medium | 8 | **6 ✅** | 2（M-2 harness、M-5 CI/audit） |
> | Low | 21 | **6 ✅** | 15（理论边界/低 ROI/开发态） |
>
> **验证**：`cargo check --workspace` + `cargo clippy`（改动 crate）全 clean；改动 crate 单测全绿（infra-regex 25、infra-vector 16、infra-sqlite 40、domain 314、quality_gate 19、app-memory 9、infra-import 57）；前端 `node --test` 24/24。新增 9 个针对性测试（ReDoS 超时、配额、维度错误、恢复计数器、路径脱敏、长间隔合法回忆等）。

---



## 一、总体评价

**整体工程质量：高，显著高于一般水平。**

这是一款经过多轮迭代加固的成熟代码库。从 git log 可见多次「Gate N 第 X 次评审修复」的痕迹，说明已建立严格的评审闭环。

### 设计良好的方面（值得肯定）

| 维度 | 实现 | 位置 |
|------|------|------|
| **SQL 注入防护** | 所有 `format!` 拼接的表名/列名均为硬编码常量；`quote_identifier` 正确转义双引号 | `infra-sqlite/src/audit.rs:172` |
| **SSRF 防护** | card-shell fetcher 拒绝 IP-literal URL，每跳重定向都校验 host 白名单，40MiB 上限，30s 超时 | `tauri-app/src/card_shell_cache.rs:149-161,326-361` |
| **API key 安全** | OS keyring 存储，磁盘只存 SecretRef，Debug 手动打码，连接摘要不含 key，启动迁移明文 key | `tauri-app/src/connection_store.rs`, `domain/src/llm.rs:30` |
| **LLM 客户端健壮性** | 响应大小限制（`MAX_LLM_RESPONSE_BYTES`）、流式空闲超时（90s）、取消机制（watch）、TLS 用 rustls | `infra-llm/src/http_client.rs:69-108,362-412` |
| **前端 XSS 防护** | DOMPurify 清洗不可信 HTML；`formatContent` 先 HTML 转义再格式化；postMessage origin 校验（`event.source === iframeRef.contentWindow`） | `RichContent.vue:29`, `formatContent.js:11-15`, `PluginHost.vue:254-256` |
| **card-shell CSP** | host 白名单经 `sanitizeHosts` 校验（仅 hostname 正则），fail-closed 到 data/blob/cache | `cardShellCsp.js:26-36` |
| **事务完整性** | `UnitOfWork` Drop 回滚 + 所有错误路径 `?` 传播；cutover 用 fsync+rename+父目录 fsync 持久化 | `infra-sqlite`（子代理确认无 commit/rollback 漏配） |
| **Accept 屏障** | draft_hash 单一权威（`compute_draft_hash`），CAS + expected_revision 原子性，Accept/Discard 副作用互斥 | `domain/src/turn.rs:542`, `infra-sqlite/preaccept.rs` |
| **CI 最小权限** | 三个 Gitea workflow 全部 `permissions: contents: read`，无 `pull_request_target`，无 git 源依赖，无硬编码 secret | `.gitea/workflows/*.yml` |

**未发现 Critical 级漏洞**（无 RCE、无 SQL 注入、无 key 外泄、无远程未授权访问）。

### 主要风险主题

最突出的风险集中在 **DoS / 资源耗尽类**（ReDoS、导入无大小限制、向量库无上限）和 **错误处理一致性**（少数 panic/unwrap、错误吞没、信息泄漏）。这些都是本地威胁模型下的中高风险，非远程利用。

---

## 二、High 级发现

> 以下发现经主审复核确认属实（标注 ✅ 复核确认）。

### H-1. ReDoS：正则引擎仅有长度上限，无超时；预设导入无试编译 ✅

**位置**：`crates/infra-regex/src/lib.rs:113-137`；`crates/infra-import/src/lib.rs:90-93`

**证据**：
```rust
// infra-regex/src/lib.rs:113
fn apply_single_script(text: &str, script: &RegexScript) -> Result<String, RegexError> {
    const MAX_REGEX_INPUT_LEN: usize = 1024 * 1024; // 1MB
    if text.len() > MAX_REGEX_INPUT_LEN { return Err(...); }
    let re = regress::Regex::with_flags(...)?;  // regress 是回溯引擎
    re.replace_all(text, ...);  // 无超时
}

// infra-import/src/lib.rs:90 — import_preset 未调用 check_import_size，也未试编译 find_regex
pub fn import_preset(data: &[u8]) -> Result<Preset, ImportError> {
    let st: StPreset = serde_json::from_slice(strip_utf8_bom(data))?;  // 对比 import_character:50 有 check_import_size
    Ok(Preset::from_st(st))
}
```

**问题**：regress 是回溯正则引擎，1MB 输入仍允许指数级回溯（如 `^(a+)+$` 在全 'a' 输入上 catastrophic）。每次写作流水线（`apply_regex_scripts_for_target_at_depth`）应用预设正则时都会触发。恶意/有 bug 的角色卡或预设可植入灾难性正则，冻结 UI 数分钟。`import_preset` 既无大小检查也无试编译，与 `import_character`（lib.rs:50 有 `check_import_size`）不一致。

**影响**：本地 DoS（用户导入恶意角色卡/预设后，写作时 UI 卡死）。

**建议**：
1. 在 `apply_single_script` 外层包 `tokio::time::timeout` 或同步线程 + join 超时；
2. 在 `import_preset` / `import_global_regex_settings` / `GlobalRegexStore::replace_all` 入口对每条 `find_regex` 试编译并检测危险模式（嵌套量词、重叠交替）；
3. 给 `import_preset` 补 `check_import_size`，与 `import_character` 对齐。

---

### H-2. 多个导入/配置命令缺输入大小上限 ✅

**位置**：
- `crates/tauri-app/src/commands/import_export.rs:565` `import_campaign_bundle(bundle_json: String, ...)`
- `crates/tauri-app/src/commands/presets.rs:233` `import_global_regex_settings(settings_json: String)`
- `crates/tauri-app/src/commands/profiles.rs:53,100,127` `save_profile` / `save_agent_profile_config` / `import_agent_profile_config`

**问题**：这些命令直接 `serde_json::from_str(&input)` 前端字符串，无 `len()` 检查。对比 `commands/characters.rs:159` `import_character(data: Vec<u8>)` 走 100MiB 上限。前端（或被劫持的插件 iframe）发送超大 JSON 会在 IPC 反序列化时占大量内存，可能 OOM。Bundle 还会在 `rewrite_bundle_ids` 做 O(n) HashMap 构造。

**建议**：在命令入口统一加 `if input.len() > MAX { return Err(validation) }`（建议 Bundle 16MiB、profile/regex 4MiB）。

---

### H-3. JSON 启动恢复对持久性错误无限重放 Committing Turn ✅

**位置**：`crates/tauri-app/src/turn_lifecycle.rs:769-889`

**证据**：
```rust
// line 877-888：CampaignNotFound 和默认错误分支都保持 Committing
Err(CommitError::CampaignNotFound(_)) => {
    self.record_recovery_issue(&turn_id,
        format!("启动恢复 Campaign {campaign_id} 不存在，保持 Committing"));  // Campaign 已不存在 = 永远不会成功
}
Err(error) => {
    self.record_recovery_issue(&turn_id,
        format!("启动恢复遇到可重试错误，保持 Committing: {error}"));  // 持久 IO 错误也保持
}
// record_recovery_issue (line 751) 只写 failure_reason，Turn 仍 Committing
```

**问题**：`record_recovery_issue` 只记 `failure_reason`，Turn 保持 Committing 态。下次启动 `list_recoverable_turns()` 又返回它 → 跨启动无限重放。特别是 `CampaignNotFound`（Campaign 已删除）意味着该 Turn **永远不会成功**，却保持 Committing 而非 Failed，每次启动都重试 + 全表扫描 + lock 获取。无重试计数器。

**建议**：
1. `CampaignNotFound` 应直接标 Failed（Campaign 不存在 = 不可恢复）；
2. 对默认错误分支加重放计数器（持久化到 TurnRecord），超 N 次升级为 Failed。

---

### H-4. `write_authority_binding` 吞掉 import_runs UPDATE 错误 ✅

**位置**：`crates/infra-sqlite/src/cutover.rs:582-589`

**证据**：
```rust
fn write_authority_binding(db: &Database, authority_id: &str, cutover_nonce: &str) -> Result<()> {
    db.connection().execute("INSERT INTO authority_binding ...")?;  // 错误传播
    let _ = db.connection().execute("UPDATE import_runs SET authority_id = ...");  // 错误吞没
    Ok(())
}
```

**问题**：`authority_binding` 表写入成功但 `import_runs` UPDATE 失败被静默忽略时，`validate_marker_db_binding`（cutover.rs:465）的回退分支会读 `import_runs.authority_id`，可能得到错误的身份判定。这是 cutover「提交点」路径，错误不应吞没。

**建议**：传播错误（`?`）。cutover 刚 import 完必有 completed 行，UPDATE 影响 0 行本身就是不一致信号。

---

### H-5. 生产路径 `find_attempt_mut(...).unwrap()` ✅

**位置**：`crates/infra-sqlite/src/preaccept.rs:754`

**证据**：
```rust
let attempt = turn.find_attempt(attempt_id)  // line 724: 只读查找，已 ok_or_else 校验
    .ok_or_else(|| SqliteError::RecordNotFound(...))?;
// ... write_conversation(tx, &conversation)?;  // line 751: 期间写盘
let attempt = turn.find_attempt_mut(attempt_id).unwrap();  // line 754: 重新可变查找，unwrap
```

**问题**：当前因 724 行前置校验 + `turn` 是 `&mut` 且期间无并发修改而安全。但：(a) 同函数其他分支都用 `?` + `ok_or_else`，唯独此处 unwrap，不一致；(b) 未来若在 724-754 间插入 reload，不变式静默失效；(c) SQLite 适配层 unwrap = 进程 abort，与该 crate 的 fail-closed 设计相悖。

**建议**：改为 `turn.find_attempt_mut(attempt_id).ok_or_else(|| SqliteError::RecordNotFound(...))?`。

---

### H-6. 转发任务 JoinHandle 从不 await，panic 静默丢弃

**位置**：`crates/app-pipeline/src/lib.rs:1105,2037,2474,2749`；`crates/app-agent/src/runtime.rs:768`；`crates/app-agent/src/character_extractor.rs:58`

**问题**：多处 `tokio::spawn(async move { while let Some(...) = ...recv().await { ... } })` 用于转发子 channel 进度到主 `event_tx`，JoinHandle 全部丢弃从未 await。若闭包内 panic，panic 被静默吞掉无日志。对比 `sequential_crew.rs:285` 正确 `let _ = forward.await`。

**影响**：进度转发静默失败时，用户侧表现为「某子 agent 进度卡住」但实际仍在跑，排查困难。闭包逻辑极简（只有 send），panic 概率低。

**建议**：转发任务应 `.await` 或存 handle 在收尾处 await；排水任务加注释说明「故意 fire-and-forget」。

---

### H-7. 向量库内存无上限 + 维度不匹配静默跳过

**位置**：`crates/infra-vector/src/lib.rs:240-294`

**证据**：
```rust
fn upsert(&self, record: VectorRecord) -> Result<(), VectorError> {
    let mut records = self.records.write()?;
    records.insert(record.id.clone(), record);  // 无数量/字节上限
    self.persist_records(&records)?;
}
// search 路径：维度不匹配只 warn! 并跳过，不返回错误
None => { warn!("向量维度不匹配，跳过记录 id={}"); return None; }
```

**问题**：(1) `BruteForceStore` 全内存，长期运行 campaign 远记忆持续累积无 GC/上限 → 内存不可逆增长。(2) 维度不一致时静默跳过，trait 已声明 `VectorError::DimensionMismatch` 但实现从不返回（dead variant），恶意/损坏记录污染检索结果集（条数变少却不报错）。

**建议**：加 `max_records`/`max_total_bytes` 配额；维度不匹配返回 `DimensionMismatch` 错误。

> **注**：子代理曾担忧 `hnsw_rs`，但主审复核确认 hnsw_rs **未被任何 crate 实际使用**（workspace 声明但无 Cargo.toml 引用，lock 里无 hnsw），向量库实际用 `BruteForceStore`。该担忧是误报。

---

## 三、Medium 级发现

### M-1. 错误信息回显内部绝对路径与敏感内容 ✅

**位置**：
- `crates/tauri-app/src/commands/import_export.rs:89,91,93,95,106,108,112,114,121,124` — 多处 `path.display()` 把绝对数据目录路径（如 `C:\Users\<用户名>\...`）拼进 Err 返回前端
- `crates/tauri-app/src/commands/meta.rs:82-86` — Patch 反序列化失败把整条世界书条目 JSON `value` 回显：`format!("...value: {v}")`
- `crates/infra-sqlite/src/cutover.rs:1135-1139` — 生产路径 `eprintln!` 打印绝对 DB 路径（调试残留 `[BI2]`）

**问题**：绝对路径泄漏帮助定位用户名/安装目录；世界书条目可能含私密剧情内容。该 crate 在 `BackendDiagnostics` 刻意做路径脱敏，却在此处不一致。

**建议**：IPC 边界统一 sanitize 路径（用 `<data_dir>` 占位）；meta.rs 错误只保留 index 不回显 value；删除 cutover.rs 的 eprintln 残留。

---

### M-2. endurance 写循环重试分类器依赖错误字符串前缀匹配

**位置**：`crates/harness-real-llm/tests/endurance_sqlite_real_llm.rs:443-456`

**证据**：
```rust
if error.contains("LLM 错误: 服务端错误 (5xx)")
    || error.contains("LLM 错误: 超时")
    || error.contains("LLM 错误: 速率限制 (429)")
    || error.contains("LLM 错误: HTTP 请求失败")
{ return WriteFailureClass::Transient; }
// 默认 fallback 也是 Transient
```

**问题**：依赖中文 Display 文案稳定。若 `infra-llm` 调整 `#[error(...)]` 文案，分类器静默退化（未匹配 → 默认 Transient），可能把本应 fail-closed 的错误重试 5 次（每次退避 5+15+30+60s ≈ 110s）才停，掩盖真实故障。最近 3 个 commit（f070933, b438d6f, 9c23173）都在调整此分类器，方向是「宽进 transient」。

**建议**：改为消费结构化 `PipelineError`/`AcceptError` enum 变体，或让 `infra-llm` 暴露 `is_provider_transient()` 帮助函数，文案变更时编译期失败。

---

### M-3. `production::list_active_turns` 在 prepared stmt 仍 borrow 时嵌套查询

**位置**：`crates/infra-sqlite/src/production.rs:295-313`

**问题**：`stmt.query_map` 返回的 `Rows` 迭代器仍 borrow 连接时，循环内同连接 `load_validated_turn` 再次 prepare/查询。rusqlite 允许同连接多 stmt，但 WAL + 多 active stmt 时某些 SQLite 版本可能报 `database table is locked`。

**建议**：先把所有 id 收集成 `Vec<String>`（drop stmt 与 rows），再循环 `load_validated_turn`。`get_turn_by_variant`（251-270）已是此正确写法。

---

### M-4. delete_campaign_precursors 在 JSON 多文件场景非原子

**位置**：`crates/tauri-app/src/playthrough_lifecycle.rs:130-150`

**问题**：JSON 路径跨多文件删除（conversation → compress_jobs → Campaign）无单一事务。若 compress_jobs 删除失败，conversations 已删但 Campaign 还在 → 用户看到「空壳」Campaign，且 compress_jobs 仍指向已删 campaign。SQLite 路径走单事务 cascade，但 JSON 路径仅靠 `save_active_pointer` 恢复指针，conversation 删除无法回滚。

**建议**：compress_jobs 删除移到 conversations 删除之前，或全部前置到 Campaign 删除后做 best-effort cleanup。

---

### M-5. 缺少 cargo-audit / cargo-deny 自动化；rustls/rustls-webpki 需确认 advisory

**位置**：`.gitea/workflows/*.yml`（无 cargo audit job）；`Cargo.lock`：`rustls 0.23.40`、`rustls-webpki 0.103.13`、`reqwest 0.12.28 + 0.13.4`（双版本）

**问题**：(1) 无自动化 RUSTSEC 扫描，advisory 不会被 PR 阻断。(2) reqwest 双主版本（0.12 应用 + 0.13 tauri 内嵌）意味着两套 HTTP/TLS 栈，安全补丁需各自跟踪。(3) rustls-webpki 涉及 TLS 证书校验，对 LLM 中继 + keyring 凭据流是高危面，需 `cargo audit` 确认是否落在 advisory 区间。

**建议**：CI 增加 `cargo audit --deny warnings` 或 `cargo deny check advisories`；评估应用侧能否统一到 reqwest 0.13。

---

### M-6. evaluateMvuValueExpr 用 JSON.parse 解释表达式 + dispatchMvuInteraction 直写路径

**位置**：`frontend/src/utils/mvuInteractions.js:80-84,96-131`

**问题**：(1) `JSON.parse` 本身不执行 JS（非 eval），但允许 card 把任意大/深嵌套 JSON 字面量写进变量，`buildMvuStatDataTree` 按 `.` 分段递归建树无深度限制。(2) `dispatchMvuInteraction`（用户点 MVU 按钮）直接 `persistShellVariableWrite` 绕过提案确认门（设计意图：「点击即确认」），但 mapping 来自 card schema，card 可塞任意 key 名（如 `__storyforge*` 内部命名空间）。

**建议**：`persistShellVariableWrite` 入口加 `__storyforge*` 命名空间黑名单（与 `buildMvuStatDataTree` 镜像）；`JSON.parse` 结果做深度+字节上限校验。

---

### M-7. cardShellDisplay 前端正则无超时（与后端 ReDoS 对称）

**位置**：`frontend/src/utils/cardShellDisplay.js:226-258`

**问题**：`parseStFindRegex` 用 `new RegExp` 编译 card manifest 的 trigger，`re.test(text)` 在长消息上可 catastrophic backtracking，冻结主线程 UI。与 H-1 后端 regress 是对称风险。

**建议**：限制 trigger pattern 长度；在 web worker 跑 test 或加长度截断。

---

### M-8. `is_attributed_private_leak` 48 字窗口可能误报合法回忆

**位置**：`crates/app-pipeline/src/quality_gate.rs:365-395`

**问题**：固定 48 字窗口。长间隔「合法回忆」（拥有者标签在窗口外但 probe 合法出现）会因 `!has_owner` 触发 Error，把合法回忆误报为越权，迫使作者修改或被迫 Degraded。`owner_name` 为空时（正文只写名字而 instance_id 不匹配）也会误报。

**建议**：窗口半径可配置；`owner_name` 缺失时放宽叙述层判定或降为 Warning。

---

## 四、Low 级发现（汇总）

| # | 位置 | 问题 |
|---|------|------|
| L-1 | `infra-sqlite/src/cutover.rs:1108` | `check_count` 的 `id_column` 参数未使用（`let _ = id_column`），给人「按 id 校验」错觉 |
| L-2 | `infra-sqlite/src/cutover.rs:804` | `CutoverState` 枚举定义后从未使用（死代码或未实现恢复分支） |
| L-3 | `infra-sqlite/src/importer.rs:466-494` | `find_completed_run` 与 `_tx` 版 SQL 重复，修改易漏改 |
| L-4 | `infra-sqlite/src/migrations.rs:170-183` | 每次 `migrate()` 对已应用 migration 走空事务 BEGIN/COMMIT，高频写场景噪音 |
| L-5 | `infra-sqlite/src/lease.rs:245-265` | `Drop` 在锁中毒时静默 return，建议加 `tracing::error!` 便于诊断 |
| L-6 | `domain/src/chronicle.rs:1016-1026` | `next_code_seq` 在 seq 达 u32::MAX 时复用最大值（静默冲突，理论边界） |
| L-7 | `domain/src/llm.rs:385-397` | `Usage.total_tokens` 为 u32，超大上下文（百万 token）可能截断；建议 u64 |
| L-8 | `domain/src/card_studio.rs:949` | LLM 评分 `v as u32` 截断 u64（LLM 幻觉大数时静默截断） |
| L-9 | `app-pipeline/src/quality_gate.rs:50-85` | `check_ngram_repetition` 用 `&[char]` 作 HashMap key，长文本内存压力 |
| L-10 | `app-agent/src/postprocess.rs:250-259` | `LikelyCompleted` 硬编码 confidence=0.5，DTO 无 confidence 字段 |
| L-11 | `app-pipeline/src/lib.rs:415` | `let _ = by_turn;` 抑制 unused warning，疑似死变量 |
| L-12 | `app-pipeline/src/sequential_crew.rs:149-151` | `should_stop_actor` 仅在 `#[cfg(test)]`，生产路径 `failures` map 是死数据 |
| L-13 | `harness-real-llm/src/evidence_retention.rs:918-935` | `allocate_run_id` check-then-create TOCTOU（UUIDv4 碰撞概率极低，实际安全） |
| L-14 | `harness-real-llm/src/evidence_retention.rs:2987-3003` | retention 清理按 mtime 排序，时钟跳变可删错 run（有 fail-closed 保护活跃 run） |
| L-15 | `frontend/package-lock.json` | `resolved` 指向 npmmirror.com 而非 npmjs（镜像投毒面 + 跨地域可用性） |
| L-16 | `frontend/vite.config.js:12` | dev server `host: true` 绑 0.0.0.0（开发态无鉴权） |
| L-17 | `reset-data.sh:20,22,48` | `node -e "require('$DATA/...')"` 路径含特殊字符可注入 |
| L-18 | `dev.sh:24,33,46` | `taskkill //F //IM storyforge.exe` 全杀同名进程（误杀其他 worktree 实例） |
| L-19 | `Cargo.toml` workspace.dependencies | `hnsw_rs = "0.3"` 声明但无 crate 引用（死声明） |
| L-20 | `Cargo.toml` | 缺显式 `[profile.release]`（依赖默认，无 strip/lto） |
| L-21 | 测试 fixture 多处 `sk-test`/`sk-abc` 等 | secret-scan 阈值是魔法数字 `sk-[A-Za-z0-9_-]{20,}`，未来长 fixture 会触发循环（如 commit 9c23173） |

---

## 五、子代理发现但经主审判定为「降级/误报」的点

| 子代理发现 | 主审判定 | 原因 |
|-----------|---------|------|
| JSON 解析深嵌套栈溢出（H1-1，infra-import） | **降级为 Low** | `serde_json 1.0.150` 默认启用 128 层递归限制（1.0.128+ builtin），128 层 < 100KB 栈，不足以炸栈。仍建议显式 walk 校验任意 JSON（`raw_card_json`/`extensions`） |
| hnsw_rs 内存无上限 | **误报** | 主审复核 Cargo.lock 与所有 Cargo.toml，确认 hnsw_rs **不在依赖树**（workspace 声明但无引用），向量库实际用 BruteForceStore |
| `card_shell_register_doc/module` 可注入持久化恶意 JS | **维持 Medium** | 有 `MAX_SHELL_DOCS=128` + `MAX_SHELL_DOC_BYTES=16MiB` + token 是 UUIDv4×2 + 一次性消费（take）防护，但无时间过期，恶意角色卡可反复注册占满配额（DoS 非 RCE） |
| reqwest 0.13.4 由 tauri 拉入 | **确认属实** | lock 确实有双版本，但 tauri 是受信依赖，风险可控，维持 M-5 |

---

## 六、复核要点与建议优先级

### 应优先处理（高 ROI）

1. **H-1 ReDoS 防护**：给 `apply_single_script`（`infra-regex/src/lib.rs:113`）加超时，并在 `import_preset`/`import_global_regex_settings` 入口试编译每条 `find_regex`。这是当前最现实的「恶意角色卡 → 冻结 UI」路径，前后端对称（M-7）。
2. **H-2 统一输入大小上限**：`import_campaign_bundle`、`import_global_regex_settings`、`save_profile`、`save_agent_profile_config`、`import_agent_profile_config` 都缺 `len()` 检查。
3. **H-3 启动恢复**：`CampaignNotFound` 应标 Failed；默认错误分支加重放计数器。
4. **M-5 跑一次 `cargo audit`**：确认 rustls-webpki 0.103.13 / keyring 4.1.3 / rusqlite 0.32.1 是否落在 RUSTSEC advisory 区间，并固化进 CI。这是当前最大未知数。

### 中期改进

5. **M-1 错误信息脱敏**：IPC 边界统一 sanitize 绝对路径；meta.rs 不回显世界书条目正文。
6. **M-2 endurance 分类器**：改为结构化 enum 分类，防止文案漂移静默退化。
7. **H-7 向量库**：加配额 + 维度校验返回错误。
8. **M-4 JSON 删除原子性**：调整删除顺序。

### 低优先级

9. 清理死代码（CutoverState 枚举 L-2、hnsw_rs 声明 L-19、should_stop_actor L-12）。
10. 补 `[profile.release]`（L-20）、package-lock 镜像源（L-15）。

---

## 七、审查局限性说明

1. **未编译运行**：本次为静态审查，未执行 `cargo test`/`cargo clippy`/`cargo audit`。建议运行上述命令确认动态行为与 advisory。
2. **测试代码未深审**：除影响可靠性的 harness/endurance 分类器外，测试代码（`_test.rs`/`lib_tests*.rs`/`tests/`）未逐行审查。
3. **部分文件抽样**：`app-meta`（7.3K 行）、`harness-real-llm`（33K 行）等大 crate 由子代理覆盖，主审聚焦复核关键发现，未逐行重读全部。
4. **`cargo audit` 未执行**：依赖 advisory 状态需离线工具确认，是当前最大未验证项。

---

## 八、结论

StoryForge 是一款工程素养高的成熟代码库，核心安全机制（SQL 注入、SSRF、XSS、key 存储、TLS、事务完整性、Accept 屏障）设计严谨。**未发现 Critical 漏洞**。主要改进空间在：

- **DoS 防护**（ReDoS / 输入大小限制 / 向量库配额）
- **错误处理一致性**（少数 panic/吞没/信息泄漏）
- **CI 自动化**（cargo-audit）

建议按第六节优先级处理 High 级发现。
