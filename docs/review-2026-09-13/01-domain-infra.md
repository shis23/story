# 审查域1：domain + 基础 infra（domain / infra-util / infra-vector / infra-regex）

- 任务：task-1（revision 2，owner=review-domain）
- 基线：git HEAD `ab894c6`，工作树干净，日期 2026-09-13
- 审查方式：**只读**。未修改/创建/删除任何源码、配置、测试、文档；仅写入本报告文件。
  未运行 cargo / npm / 任何构建或测试命令。工具：read / grep / glob / Select-String / git log。
- 结论口径：每条发现都给 `文件:行号` + 代码摘录（≤15 行）。**文档描述不作为证据**。
  「已核实」= 我本人打开过该行代码；「疑似」= 证据来自协作者初筛，我未逐行复核或需要运行时/数据侧确认。

---

## 1. 范围与覆盖率

### 1.1 审查对象与规模

| 子域 | 文件数 | 行数（read 工具实际行数） | 覆盖方式 |
| --- | --- | --- | --- |
| `crates/domain/**` | 24 | ≈16.5k（card_studio 2373、chronicle 1625、prompt_module 1232、character 1192、campaign 916、turn 872、conversation 841、agent 814、message_layout 724、card_shell 704、variables 696、mvu_translation 644、agent_profile_config 637、world_info 624、campaign_runtime 572、character_knowledge 538、preset 511、narrative_contract 464、story_task 406、novel_distill 323、generation 216、history 112、lib 63） | 核心 9 文件我逐段自读（campaign / character / campaign_runtime / variables / chronicle / narrative_contract / character_knowledge / story_task / turn 的枚举与尾部）；world_info / preset / prompt_module / message_layout / agent / agent_profile_config / conversation / history / novel_distill / mvu_translation / card_shell / card_studio 由 3 个子代理全文初筛，**我逐条复核了其中 20 条关键发现的代码行**（见 §3 每条置信度标注） |
| `crates/infra-util/**`（lib.rs / secret_store.rs / write_fence.rs） | 3 | 154 / 286 / 124 | 我全文自读 |
| `crates/infra-vector/**`（lib.rs） | 1 | 833 | 我自读 1–620 + 421 之后关键段；测试段抽读 |
| `crates/infra-regex/**`（lib.rs） | 1 | 961 | 我自读 1–450 与测试段；核心引擎逐函数核对 |

未覆盖/未深挖（明确声明）：`crates/domain/**` 中 `generation.rs` 我未亲自逐行复核（仅子代理结论）；`turn.rs` 中段（130–780）我未逐行复核；`infra-regex` 的 ST placement 语义按 task-1 约定不评（域6）；其余 15 个 crate 不在本域范围。

### 1.2 范围外但被引用的证据文件

为判定影响半径，只读引用了 `crates/tauri-app/**`、`crates/app-pipeline/**`、`crates/app-memory/**`、`crates/app-agent/**` 的**调用点行号**，未对这些 crate 做审查，也**未据此下域外结论**；涉及处均标注「跨域」。

---

## 2. 结论摘要

- **P0：1 条（D-01）；P1：3 条（D-02～D-04）；P2：12 条（D-05～D-16）；P3：10 条（D-17～D-26，含合并观察）。合计 26 条。**
- domain 层整体结构完整、纯度达标（无 tauri / 网络 / IO 依赖），CLAUDE.md 与 DATA_MODEL.md 中关于 `CharacterInstance`、`CharacterDefinition`、`resolved_persona/behavior`、`CampaignRuntimeContext`、`AgentProfileConfig`、Chronicle A/B/C、`TurnStatus/AttemptStatus` 的「Current Code Facts」条目**逐条与代码一致**，未发现重大声明造假；主要问题集中在**边界/静默降级**与**双权威默认值**。
- 最严重的是 `card_shell.rs` 的 UTF-8 边界 panic（可由中文卡 JSON 直接触发，落在 Tauri 命令里），其次是世界书 v3 `enabled` 双写导致导出→重导入后开关被静默回滚，以及向量库损坏后缺少写栅栏保护而可能以空集覆盖原文件。

---

## 3. 发现清单（按严重度降序）

### D-01 · P0 · 类别 D（panic） · `crates/domain/src/card_shell.rs:428`、`:441`

**`capture_es_module_urls` 缺少 UTF-8 边界推进，中文卡内容可直接触发 panic（同文件兄弟函数有该守卫）**

```rust
421: fn capture_es_module_urls(text: &str) -> Vec<String> {
424:     for (pat_start, _pat) in [("import ", true), ("from ", true), ("import(", true)] {
425:         let lower = text.to_ascii_lowercase();
426:         let mut search_from = 0;
427:         let needle = pat_start;
428:         while let Some(rel) = lower[search_from..].find(needle) {
429:             let abs = search_from + rel + needle.len();
430:             let rest = text[abs..].trim_start();
...
441:             search_from = abs + 1;
```

`abs` 落在 ASCII 匹配之后（保证是 char 边界），但 `abs + 1` 只有在 `abs` 处字节是 ASCII 时才是 char 边界。当 needle 紧跟多字节字符（例如卡内 JS 注释 `// 移植 from 原作者`、`import 模块`）时，下一轮 `lower[search_from..]` 在非边界切片 → panic（`byte index N is not a char boundary`）。同文件 `capture_jquery_load_urls` 在 `:351-354` 与 `capture_http_urls`（`:359-419`，按 `char.len_utf8()` 推进）都有守卫，本函数独缺。

**可达性（已核实）**：`extract_from_tavern_helper`（`card_shell.rs:228` 调用）← `extract_card_shell_manifest`（`card_shell.rs:101`）← Tauri 命令 `crates/tauri-app/src/commands/card_shell.rs:36`、`:91`。输入是卡 JSON `extensions.tavern_helper.scripts[].content`，即不可信外部数据。

**影响**：导入/打开含中文 TH 脚本的卡时命令 panic，卡片 shell manifest（及 inline JS 获取）不可用；`[profile.release]` 未设 `panic="abort"`，为 unwind 到命令边界，用户体验为功能不可用 + 错误日志。

**建议**：把 `:352-354` 的 `is_char_boundary` 推进循环复制进来（并把 `to_ascii_lowercase()` 提到 3 个 pattern 循环之外）；补测试 `capture_es_module_urls("// 移植 from 原作者")`。

**置信度：高（已核实）；E 缺口：全 workspace 无任何测试调用该函数**（现有 UTF-8 回归测试 `:690-703` 只覆盖 `capture_http_urls`/`capture_jquery_load_urls`）。

---

### D-02 · P1 · 类别 B（不变量/往返保真） · `crates/domain/src/world_info.rs:216-221`、`:94-104`、`:287`、`:291`

**ST v3 `enabled:false` 世界书条目：用户启用后导出→重导入会被静默重新禁用**

```rust
216:         let v3_enabled = st
217:             .extra
218:             .get("enabled")
219:             .and_then(serde_json::Value::as_bool)
220:             .unwrap_or(true);
221:         let disabled = st.disable.unwrap_or(false) || !v3_enabled;
```
```rust
 94:     pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
 95:         if enabled && matches!(self.route, LoreRoute::Disabled) {
...
100:             self.route = restored;
101:         }
102:         self.disabled = !enabled;
103:         Ok(())
104:     }
```
```rust
287:             disable: Some(self.disabled),
...
291:             extra: self.extra.clone(),
```

`extra` 是 `StWorldInfoEntry` 上的 `#[serde(default, flatten)]`（`crates/domain/src/character.rs:146-148`），因此 v3 的 `enabled` 键作为**同一语义的第二个真相源**被原样保留（`from_st` 里 `extra: st.extra`，`:261`）。`set_enabled(true)` 只写 `disabled`，不改 `extra["enabled"]`；导出时同时写出 `disable:false` 与 `enabled:false`，重导入按 `:221` 计算 `false || !false == true` → 条目又是禁用的。

**可达性（已核实）**：启用开关的生产入口 `crates/tauri-app/src/campaign_store.rs:896-910 set_world_info_entry_enabled`（经 `storage_backend.rs:1754-1775`、命令 `crates/tauri-app/src/commands/world_info.rs:322`）；导出走 `WorldInfoBook::to_st_book`（`world_info.rs:171-176`）。MVU/DLC 卡大量把 `[InitVar]`、事件数据放在禁用条目里，正是这条路径的目标用户。`card_studio` 的 `export_gate_checks` 只校验 content/keys/insertion_order（`card_studio.rs:1367/1399/1417`），不覆盖该位。

**影响**：用户显式启用的世界书条目在导出后重新导入（或卡工作室回环）后静默回到禁用态；无任何错误、无日志。属"用户意图被静默回滚"。

**建议**：`set_enabled` 内同步 `self.extra`：`if self.extra.contains_key("enabled") { self.extra.insert("enabled".into(), Value::Bool(enabled)); }`；或在 `to_st_entry` 写出 `disable` 时移除 `enabled`。补一条 `set_enabled(true)` → `to_st_book()` → `from_st()` 的往返断言。

**置信度：高（已核实）；E 缺口：无任何测试断言 `disable` 与 flatten 的 `extra["enabled"]` 在往返后一致。**

---

### D-03 · P1 · 类别 D/B（静默失败/数据丢失） · `crates/infra-vector/src/lib.rs`（`with_persistence` 的损坏分支）

**向量库主文件不可恢复损坏时不冻结写入，且 `.corrupt` 备份失败被忽略 → 下一次 upsert 可能以空集覆盖原文件**

```rust
 69:    fn recovery(path: &Path) -> (HashMap<Id, VectorRecord>, Option<PathBuf>) {
 70:        if !path.exists() {
 71:            return (HashMap::new(), None);
 72:        }
 73:        let attempt = |p: &Path| -> Result<HashMap<Id, VectorRecord>, String> {
 74:            let raw = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
 75:            serde_json::from_str(&raw).map_err(|e| e.to_string())
 76:        };
 77:        if let Ok(records) = attempt(path) {
 78:            return (records, None);
 79:        }
 80:        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
 81:        if let Ok(records) = attempt(&tmp) {
 82:            warn!(...);
 83:            return (records, None);
 84:        }
 85:        warn!(... "copies to .corrupt and starts empty");
 86:        let backup = path.with_extension("json.corrupt");
 87:        let _ = std::fs::copy(path, &backup);   // ← 结果被丢弃
 88:        (HashMap::new(), Some(backup))
 89:    }
```

对比项目既定的 V4 硬化约定（`crates/infra-util/src/write_fence.rs:1-9` 的动机注释、`crates/tauri-app/src/storage_health.rs:60-71 record_unrecoverable`：主文件损坏且 `.tmp` 不可用 → `write_fence::freeze(path)` + 登记阻断事件），本 store **完全不接入**该机制：损坏时返回空 map，任何后续 `upsert/delete` 都会 `persist_records`（`:404`）把"空集 + 新记录"原子写到主文件；而第 87 行的备份拷贝失败被 `let _ =` 吞掉，此时唯一的原始数据被彻底覆盖且没有任何提示。生产确实使用该 store：`crates/tauri-app/src/lib.rs:841 BruteForceStore::with_persistence(...)`。

**影响**：向量记忆（`VectorKind::ArchivedSummary` / `CharacterKnowledge` 等）在损坏场景下静默归零；与其它 JSON store 的保护级别不一致，且失败路径无事件、无日志（`warn!` 只说"copies to .corrupt"，不校验是否成功）。

**建议**：`std::fs::copy` 失败时返回 `Err`/记录 `tracing::error!` 并调用 `storyforge_infra_util::write_fence::freeze(path)`（或让 store 走统一的 `json_store::load_json_with_tmp_backup_or_default` + `storage_health`）。

**置信度：高（已核实代码路径与生产调用点）；影响面限于向量库（非 Campaign 真相源），故定 P1 而非 P0。**

---

### D-04 · P1 · 类别 B（契约不匹配，跨域） · `crates/domain/src/story_task.rs:149-154` + `crates/app-pipeline/src/turn_dossier.rs:284-285`

**`StoryTime` 触发器只做严格字符串相等；生产调用方传入空 story_clock，导致"故事时钟"类伏笔在 writer 路径永不注入**

```rust
149:                 TaskTrigger::StoryTime { target } => {
150:                     // 大小写不敏感比较（M-30），并去除首尾空白
151:                     if story_clock.trim().eq_ignore_ascii_case(target.trim()) {
152:                         return TriggerCheck::Satisfied;
153:                     }
154:                 }
```
```rust
284:     let pending_tasks =
285:         storyforge_domain::story_task::render_tasks_for_injection(pending_tasks, runtime.turn, "");
```

`render_tasks_for_injection`（`story_task.rs:238-247`）用 `check_trigger` 过滤；另一条调用点 `crates/app-pipeline/src/lib.rs:3333-3337` 传的是真实 `ctx.story_clock`，而 `compile_turn_dossier` 传 `""`。`CompiledTurnDossier.pending_tasks` 会经 `render_for_writer()` 的 `## 未了任务与伏笔` 区块进入 writer tail（`turn_dossier.rs:128-130`、`lib.rs:3645`），调用点 `lib.rs:1029`、`:1219` 均在写作主链路上。于是任何非空 `StoryTime` target 在该路径恒不命中（`""` 永不等于非空串），且无任何警告。

**影响**：用户按故事时钟规划（"到第2年6月触发"）的伏笔任务，在 writer dossier 路径静默消失——正是该模块声称要解决的"导演忘记三个月前的伏笔"。此外严格相等语义本身意味着时钟一旦越过 target 就永久失配（除非用户手改）。

**建议**：domain 侧把空/空白 clock 视为显式错误或 `Option`（不要静默当作可比较值）；`turn_dossier` 改用真实 story clock；并评估 StoryTime 是否需要"越过即触发"语义 + 缺测。

**置信度：高（已核实 domain 与两个调用点）；跨域：建议域3确认 `turn_dossier` 与 `build_director_tail` 两条路径在写作主链路的实际使用关系（若 dossier 为唯一正文注入源，本条按 P0/P1 上限处理）。**

---

### D-05 · P2 · 类别 B（确定性/契约） · `crates/infra-vector/src/lib.rs:341-378`

**关键词检索无排序，直接截断 HashMap 迭代序 → 远记忆召回结果进程间不稳定**

```rust
341:    fn search_by_keywords(&self, keywords: &[String], limit: usize) -> Result<Vec<VectorHit>, VectorError> {
...
358:        let mut hits: Vec<VectorHit> = records
359:            .values()
...
374:            .collect();
375:        hits.truncate(limit);
376:        Ok(hits)
```

`records` 是 `HashMap<Id, VectorRecord>`，迭代顺序由随机种子决定（每次进程启动不同），代码既没有按 score/时间排序，也没有稳定 tie-break。生产消费方按 `limit` 取值：`crates/app-memory/src/recall.rs:97`（`recall_by_keywords`）、`:157`（`fetch = limit*8`）→ `filter_archived_hits`（`recall.rs:177-205`）**按 store 返回顺序取前 `limit` 条**，同样不排序。

**影响**：同一 query 在不同启动之间召回到不同的远记忆片段（当候选数 > limit 时确定发生）→ 注入 prompt 内容不稳定（prompt 缓存失效）与召回行为不可复现；调参/复现问题困难。

**建议**：定义稳定排序（如 `score` 降序 + `id` 升序 tie-break），或让 store 返回全部候选由调用方排序。

**置信度：高（已核实）；E 缺口：无任何测试覆盖"limit < 候选数"下的顺序稳定性。**

---

### D-06 · P2 · 类别 B/F（双权威默认值不一致，跨域） · `crates/domain/src/campaign.rs:60-62` vs `crates/domain/src/variables.rs:105`

**`story_clock` 顶层默认值与 variables schema 默认值不同 → 新建 Campaign 天生处于"双表示分歧"状态**

```rust
 60: fn default_story_clock() -> String {
 61:     "Day 1".into()
 62: }
```
```rust
103: pub fn default_campaign_variables() -> Vec<VariableField> {
104:     vec![
105:         VariableField::string("story_clock", "故事时间", "第1天", "全局"),
```

`Campaign::new_with_variable_schema` 同时写入 `story_clock: default_story_clock()`（`campaign.rs:100`）与由 schema 初始化的 `variables`（`variables.rs` 里的 `"第1天"`）。而 `story_clock_diverged()`（`campaign.rs:181-191`）在两者不等时返回 true，`repair_story_clock_authority()`（`:201-220`）以 variables 为权威覆盖顶层字段并返回 `FieldRepaired`，加载路径会为此发 `tracing::warn!`（`crates/tauri-app/src/campaign_store.rs:52-57`；另有 `infra-sqlite/src/production.rs:1541/1563`、`commands/import_export.rs:150`）。

**影响**：(a) 每个新建 Campaign 在首次加载时都会被判定为"历史分歧"并触发一次 warn + 就地改写字段，"FieldRepaired 表示历史残留"这一审计信号被稀释成噪声；(b) `import/export` 的字节级比对被迫显式调用 repair 归一（`import_export.rs:141-151` 的注释即为此绕行），把不一致固化成实现约定。

**建议**：统一为单一默认常量（domain 导出 `DEFAULT_STORY_CLOCK`，两处引用）；或新 Campaign 直接由 variables 派生顶层字段。

**置信度：高（已核实两处常量与 repair 调用链）；跨域：改写动作发生在域2/域4 的加载路径。**

---

### D-07 · P2 · 类别 B（未处理边界） · `crates/domain/src/chronicle.rs:371-384`

**`select_anchor_turns(h_anchor = 0)` 会 panic（切片 start > end）**

```rust
371: pub fn select_anchor_turns(committed: &[CommittedTurnRef], epoch_start_head: Option<&Id>, h_anchor: u32) -> Vec<CommittedTurnRef> {
...
376:     let take = (h_anchor as usize).min(head_idx + 1);
377:     let start = head_idx + 1 - take;
378:     committed[start..=head_idx]
```

`h_anchor = 0` → `take = 0` → `start = head_idx + 1` → `committed[head_idx+1..=head_idx]` 触发 `slice index starts at N but ends at N-1` panic。同函数 `select_band_turns` 对 `s == 0` 有显式分支（`:415`），`max_near_raw_turns` 用 `saturating_add`（`:358`），只有 `h_anchor` 缺守卫。

**影响**：`ContextWindowParams.h_anchor` 是 pub 可配置字段（文件头注释明确"实验默认参数（可配置）"）。当前所有生产/测试调用点都用 `ContextWindowParams::default()`（`tauri-app/src/runtime_support.rs:1245`、`harness-real-llm/src/*`），且测试只出现 `h_anchor: 2/3/5`（`chronicle.rs:1168/1196/1224/1534/1601`），因此**当前不可达**；一旦加入配置入口/调参即 panic。

**建议**：`if h_anchor == 0 { return Vec::new(); }`（或 `start` 用 `take` 反推并防越界）；补 h_anchor=0 测试。

**置信度：高（切片条件已核实）；可达性：中（无现网调用点）。**

---

### D-08 · P2 · 类别 B（静默覆盖用户数据） · `crates/domain/src/card_studio.rs:1618-1623`

**`apply_stage_json(STAGE_BASIC)` 对空 `description`/`scenario` 不加守卫，LLM 返回空串会清空用户已有内容（`name` 有守卫）**

```rust
1613:             if let Some(s) = value.get("name").and_then(|v| v.as_str())
1614:                 && !s.trim().is_empty()
1615:             {
1616:                 artifacts.name = s.trim().to_string();
1617:             }
1618:             if let Some(s) = value.get("description").and_then(|v| v.as_str()) {
1619:                 artifacts.description = s.to_string();
1620:             }
1621:             if let Some(s) = value.get("scenario").and_then(|v| v.as_str()) {
1622:                 artifacts.scenario = s.to_string();
1623:             }
```

JSON-mode 模型返回 `"description": ""` 是常见失败形态；同文件 B 路径（`:489-498` 一带）对 description/scenario 是空值守卫的，行为不一致。

**影响**：单次"基础信息"生成即可静默清空用户手写的简介/场景，无错误、无回滚点。

**建议**：与 `name` 一致加 `!s.trim().is_empty()` 守卫（或空值返回 `Err`）；补空串回归测试。

**置信度：高（已核实）。**

---

### D-09 · P2 · 类别 B（静默降级） · `crates/domain/src/conversation.rs:413-422`

**`collect_active_variants` 遇到未知 `before_node_id` 时返回"全部节点"，把正在被重写的草稿也交给模型**

```rust
413:     /// 共享：收集 before_node 截止的活跃非空非 Discarded 变体（时间正序）。
414:     fn collect_active_variants(&self, before_node_id: Option<&Id>) -> Vec<&MessageVariant> {
415:         let end_idx = if let Some(bid) = before_node_id {
416:             self.nodes
417:                 .iter()
418:                 .position(|node| &node.id == bid)
419:                 .unwrap_or(self.nodes.len())
420:         } else {
421:             self.nodes.len()
422:         };
```

契约（`:356` 注释："before_node 截止"）在 id 过期/不匹配时应为"空前缀"或报错，`unwrap_or(self.nodes.len())` 却退化成"全量历史"。调用方 `crates/app-pipeline/src/lib.rs:1072`（regenerate）传入被重生成的节点 id。

**影响**：重生成时模型看到自己即将被替换的草稿 → 输出与上一版高度雷同/自我复述；且该降级完全静默（无日志）。属"特定时序下主路径错误"。

**建议**：改 `Result`/`Option`，或未知 id 视为空前缀并在调用方记录 warning。

**置信度：高（已核实）；跨域：建议域3确认 regenerate 传入 id 的生命周期（是否可能早于 nodes 重建）。**

---

### D-10 · P2 · 类别 B（启发式误判） · `crates/domain/src/mvu_translation.rs:257-283`、`:330-347`

**`depth_prompt.prompt` 的自然语言正文被计入 `script_bytes`，纯文本卡可被判为 `Heavy`**

```rust
330: /// 把 assets.js + extensions 里可能的 JS 源拼到一起（粗略，给打分用）
331: fn collect_js_blob(assets: Option<&RenderableAssets>, extensions: &serde_json::Value) -> String {
...
339:     // depth_prompt 可能内嵌 script
340:     if let Some(dp) = extensions
341:         .get("depth_prompt")
342:         .and_then(|v| v.get("prompt"))
343:         .and_then(|v| v.as_str())
344:     {
345:         blob.push_str(dp);
346:         blob.push('\n');
347:     }
```
```rust
261:     let script_bytes = js_blob.len();
...
283:     let script_heavy = script_bytes >= THRESHOLD_SCRIPT_BYTES_HEAVY;   // = 5_000（:237）
285:     let (classification, reasoning) = if dom_heavy || script_heavy {
```

~1667 个中文字符（3 字节/字）的纯指令正文即越过 5000 字节阈值，`classification = Heavy`，`reasoning` 写"重 DOM（script 5000 字节），需共享 WebView 兜底"；该分类与 reasoning 会注入分析器提示词（`crates/app-meta/src/prompts/mvu_analyzer.rs:133-149`）作为判定依据。

**影响**：无 JS 的纯文字卡被系统性推向 Hybrid/WebView 路由，偏置分析结果与实际渲染需求。

**建议**：`depth_prompt` 正文只参与占位符探测，不计入 `script_bytes`/DOM 计数阈值；或单独区分 prose_bytes 与 js_bytes。

**置信度：高（已核实）。**

---

### D-11 · P2 · 类别 B（模板渲染错误） · `crates/domain/src/prompt_module.rs:338-352`、`:436-438`

**嵌套宏在第一个 `}}` 处被截断：`{{setvar::a::{{getvar::b}}}}` 产出错误文本，且内联 `getvar` 替换分支实际不可达**

```rust
338:     while let Some(start) = rest.find("{{") {
339:         let (before, after_start) = rest.split_at(start);
340:         rendered.push_str(before);
341:         let macro_body_start = &after_start[2..];
342:         if let Some(end) = macro_body_start.find("}}") {
343:             let (body, after_body) = macro_body_start.split_at(end);
```
```rust
436:     for (key, value) in &state.variables {
437:         rendered = rendered.replace(&format!("{{{{getvar::{key}}}}}"), value);
438:     }
```

body 取到第一个 `}}` 为止，嵌套宏的内层被切断（存进变量的是 `{{getvar::b`），剩余 `}}` 作为字面文本输出；由于值文本总在自身 `}}` 之前被切掉，`:436-438` 的整段替换永远匹配不到其目标形态（作者意图的不可达分支）。

**影响**：卡/preset 使用嵌套宏时提示词静默产出错文本（LLM 指令被破坏），且无告警。

**建议**：按 `{{`/`}}` 计数配对（或递归解析 body）后再渲染；补嵌套宏测试。

**置信度：高（已核实）。**

---

### D-12 · P2 · 类别 B（静默截断） · `crates/domain/src/card_studio.rs:1670`、`:542`

**LLM 产出的 `order` 用 `as i32` 无检查转换，越界值静默回绕后写入 ST 插入顺序**

```rust
1670:                 let order = item.get("order").and_then(|v| v.as_i64()).unwrap_or(100) as i32;
```
```rust
539:             let order = item
540:                 .get("order")
541:                 .and_then(|v| v.as_i64())
542:                 .map(|n| n as i32)
543:                 .unwrap_or(((i as i32) + 1) * 10);
```

`4294967297 as i32 == 1`、`2147483648 as i32 == -2147483648`；该值直接进入 `WorldInfoEntry.order`（`:1049` 一带），决定世界书注入顺序。同文件对同类 LLM 数值已有正确先例（`:949-951` 用 `u32::try_from(...).unwrap_or(u32::MAX)`）。

**影响**：条目前后顺序错乱（可能整段世界书注入次序翻转）且无任何错误提示；`gate.insertion_order_roundtrip`（`:1417`）只对比"作品值与自身往返值"，检不出这种越界。

**建议**：`i32::try_from(n).unwrap_or(100)`；`:1163` 的 `max_order + 10` 同样改 saturating。

**置信度：高（已核实）。**

---

### D-13 · P2 · 类别 B/C（契约未实现） · `crates/domain/src/prompt_module.rs:189-197`

**`PromptModule::Exclusivity::Single` 从未被执行，同类别多个互斥模块会同时注入 system**

```rust
189:             let ids = profile.selected_ids(role, cat);
190:             for mid in ids {
191:                 if let Some(m) = modules
192:                     .iter()
193:                     .find(|m| &m.id == mid && role_applicable(&m.applicable_roles, role))
194:                 {
195:                     parts.push(m.content.clone());
196:                 }
197:             }
```

`exclusivity` 在全 workspace 唯一读取点是展示 DTO（`crates/tauri-app/src/module_store.rs:51` 的 `format!("{:?}", m.exclusivity)`），前端 grep 无引用；而 `save_profile` 接受任意 `PromptProfile` JSON（`crates/tauri-app/src/commands/profiles.rs:53-70`）。

**影响**：一个 Single 类别（视角/文风/语气）可配置多个 id，全部拼进 system prompt → 互相矛盾的指令同时生效；无校验、无告警。

**建议**：装配时对 Single 类别只取首个 id（或校验唯一），或删除该枚举以免误导；补测试。

**置信度：高（枚举未被执行已核实；具体 profile 数据是否已有重复项为疑似）。**

---

### D-14 · P2 · 类别 B/文档漂移 · `crates/domain/src/agent_profile_config.rs:238-250`

**`migrate_to` 注释称"未知版本不做任何修改"，代码却无条件覆写 `config_version`；且会把更高版本降级**

```rust
238:     /// 版本迁移入口。将配置迁移到 `target` 版本。
239:     ///
240:     /// 当前只有 v1，v1→v1 是 no-op。返回是否发生过迁移。
241:     /// 未知版本不报错，保留数据不变（向前兼容）。
242:     pub fn migrate_to(&mut self, target: u32) -> bool {
243:         if self.config_version == target {
244:             return false; // 已是目标版本，no-op
245:         }
246:         // 当前只有 v1；预留未来版本迁移分支。
247:         // 未知版本不做任何修改，保留数据。
248:         self.config_version = target;
249:         true
250:     }
```

`:247` 的注释与 `:248` 的代码直接矛盾（CLAUDE.md/DATA_MODEL.md 记录的是"unknown versions update config_version but preserve data"，即以代码为准）。副作用：未来版本（如 `config_version: 2`）的配置被本版本加载时会被**降级为 1**，且所有加载路径都调用它（`module_store.rs:504-507` 等），版本标记被销毁、`bool` 返回值全 workspace 无人消费。

**影响**：今天无实害（只有 v1）；但一旦引入 v2，"未来数据被旧客户端静默改标记"会让迁移逻辑重跑或漏跑，属埋雷。

**建议**：`if target <= self.config_version { return false; }`；三态返回（NoOp/Upgraded/Unsupported）；修注释。

**置信度：高（已核实）；E 缺口：无降级/更高版本保留的测试。**

---

### D-15 · P2 · 类别 B（serde 兼容） · `crates/domain/src/card_studio.rs:150-155`、`:191-192`

**`CardProject` 五个字段缺少 `#[serde(default)]`，而 store 一次性反序列化整个 `Vec<CardProject>`**

```rust
150: #[derive(Debug, Clone, Serialize, Deserialize)]
151: pub struct CardProject {
152:     pub id: String,
153:     pub name: String,
154:     pub mode: CardProjectMode,
155:     #[serde(default)]
156:     pub brief: String,
...
191:     pub created_at: String,
192:     pub updated_at: String,
```

同结构其它字段与 `CardArtifacts`/`WorldviewDraftEntry` 都带 `#[serde(default)]`，`CardProjectMode` 甚至 derive 了 `Default`（`#[default] FromScratch`）但因字段无 default 而无法从反序列化生效。`crates/tauri-app/src/card_studio_store.rs:16` 是 `let projects: Vec<CardProject> = json_store::load_json_with_tmp_backup_or_default(...)`——单条老/残缺记录即让**整个** `card_projects.json` 解析失败，回退空列表（原文件会被 `.corrupt` 备份保留，但 UI 上项目列表消失，下一次保存会写回新文件）。

**影响**：一条脏记录 = 全部卡工作室项目在 UI 中消失（可恢复但用户可见数据丢失体验）。

**建议**：五字段补 `#[serde(default)]`；并改为逐条解析（`Vec<Value>` 后 `from_value`），跳过坏条目并告警。

**置信度：机制高（已核实）；"历史 JSON 确实缺这些键"为疑似（未见到样本数据）。**

---

### D-16 · P2 · 类别 D（门禁可被输入操纵） · `crates/domain/src/card_studio.rs:914-924`

**LLM 自选的 `code` 子串决定硬阻断等级：命中 `required`/`keys`/`empty` 保持 Error，其余降级 Warning**

```rust
914:             // LLM cannot alone invent hard blockers for missing core fields; demote unknown errors to warning
915:             // unless code is clearly aligned with methodology.
916:             let severity = if matches!(severity, CheckSeverity::Error)
917:                 && !code.contains("required")
918:                 && !code.contains("keys")
919:                 && !code.contains("empty")
920:             {
921:                 CheckSeverity::Warning
922:             } else {
923:                 severity
924:             };
```

是否构成硬错误取决于模型自己填的字符串。下游影响（`ok`/评分 clamp/`last_error`）由子代理给出的行号 `:946-956` 与 `crates/tauri-app/src/card_studio_api.rs:354-362` 支撑，**我未复核这两处**。

**影响**：模型只要把严重问题命名成不含上述词根的 code 即可自行降级（或反向：给无害问题取名含 `empty` 即变硬阻断）。它是审查建议面而非编译闸门（`compile_artifacts` 只走 `run_checks`），故 P2。

**建议**：用 `run_checks` 的规则码白名单判定，或要求 LLM 提供 `rule_ref`。

**置信度：代码事实高（已核实 `:914-924`）；下游门禁强度：中（未复核）。**

---

### D-17 · P3 · 类别 B/E · `crates/domain/src/novel_distill.rs:88-90`、`:147-173`

空文本小说生成"永不可推进"的作业，且 `hard_cap` 可溢出。`chunk_novel("")` 返回空 vec（自有测试 `:262` 断言），`next_pending_chunk()` 为 None 但 `stage` 停在 `Chunks`，`apply_chunk_result` 因无合法 index 必失败、`apply_style_formula` 又因阶段不符被拒（`:181-183`）→ 无合法路径到 `StyleFormula/Ledgers/Done`；`progress()` 返回 `(0,0)`（`:201-204`）。`let hard_cap = target * 2;`（`:90`）对接近 `usize::MAX` 的入参溢出（debug panic / release 回绕为 0 → 每字符一块）。

```rust
 89:     let target = target_chars.max(200);
 90:     let hard_cap = target * 2;
```
缓解：该模块目前**无生产调用者**（全 workspace grep 只有 `domain/src/lib.rs:17 pub mod novel_distill;` 与本文件测试），故不升级。建议 `new()`/`advance()` 在 `chunks.is_empty()` 时直接推进阶段；`saturating_mul` + 上限钳制。**置信度：高（已核实）。**

### D-18 · P3 · 类别 E/C · `crates/domain/src/history.rs`（全文 112 行）

`truncate_uncommitted` / `require_committed_head` 是"删除持久化对话"的对账函数，**整个文件零测试**（无 `#[cfg(test)]`，我通读确认）。未覆盖不变量：受影响轮次的 cut 前移（`:49-55 cut = cut.min(input)`）、已提交内容守卫（`:40-48`、`:57-65`）、轮次就地改写（`:66-77`）、水位钳制（`:80-89`）。另 `:36 attempt.variant_id == node.id` 把 node id 存进名为 `variant_id` 的字段（经 `infra-sqlite/src/preaccept.rs:247-261` 确认是 node id，行为无害），是给下一个改代码的人的陷阱。建议按分支补单测并重命名/加注释。**置信度：高（已核实，我通读全文）。**

### D-19 · P3 · 类别 B/C · `crates/domain/src/message_layout.rs:122-126`、`:136-140`、`:400-403`

- 分段哈希无长度前缀：`["a","b"]` 与 `["a\nb"]` 摘要相同（都拼成 `a\nb\n`），而 `into_messages()` 语义不同 → `full_request_fingerprint()`（`:114`）对不同请求可能给出同一指纹；`SegmentFingerprint` 因额外带 `tail_parts` 计数而部分免疫（`:147`）。该 API 当前只有测试调用，故 P3。
- builder 不是注释宣称的"编译期强制三段"：`system()`/`history()` 可缺、可重复（后写覆盖），缺失即 `unwrap_or_default()`（`:401-402`），且 `into_messages()` 总会 push 一条空 system（`:88-89`）。
建议：长度前缀/带 part 索引；缺失段改用 `Result` 或 typestate。**置信度：高（已核实）。**

### D-20 · P3 · 类别 F（代码注释/文档漂移） · `crates/domain/src/agent.rs:41-42`、`:688-695`；`crates/domain/src/story_task.rs:232`；`crates/domain/src/mvu_translation.rs:242`

- `agent.rs:41-42` 注释称裸 `"Subagent"` 会被解析为 `Subagent("")`，但 `from_str`（`:90-101`）无该分支 → 会返回 `unknown AgentRole variant: Subagent`。
- `agent.rs:688-695`：`chronicle_level()` 对非法 `level` 回退 A（`unwrap_or(A)`），`is_leaf_a()` 却比较 `level == 0`；`level = 7` 时两个访问器结论矛盾。
- `story_task.rs:232` 注释"LikelyCompleted(>0.8) 的不自动注入"暗示高置信度会自动注入，但 `:240` 只放行 `is_injectable()`（Pending/Active），任何置信度都不注入；`TaskStatus::Completed` 的"后处理高置信度"也无代码路径。
- `mvu_translation.rs:242` 文档称 `getElementById` 超阈值即 Heavy，`:281-282` 只判定 `document_calls`/`innerhtml_calls`；`get_by_id_calls`、`placeholder_calls`、`jq_calls`、`script_blocks` 收集进 `counts` 后不参与判定。
**置信度：高（`agent.rs:688-695`、`story_task.rs`、`mvu_translation.rs` 已核实；`agent.rs:41-42/90-101` 由子代理证据，标记疑似-中）。建议对齐注释或实现。**

### D-21 · P3 · 类别 C/D · `crates/domain/src/agent.rs:122-131`；`crates/domain/src/llm.rs:11-37`、`:40-46`

- `AgentProfile`（`agent.rs:122-131`）全 workspace 无调用者，与 `AgentRunConfig`/`AgentProfileConfig` 语义重复。
- `LlmConnection.api_key: String`（`llm.rs:17`）只在手写 `Debug` 里脱敏（`:24-37 .field("api_key", &"***")`），`#[derive(Serialize)]` 未加 `skip_serializing`——今天靠"IPC 只走 `LlmConnectionSummary`"兜住，任何未来 `-> LlmConnection` 命令即明文外泄。
- `LlmProtocol::Custom(String)`（`:45`）用外部标签，同一字段可能序列化为 `"OpenAi"` 或 `{"Custom":"x"}`，而详情 DTO 是扁平字符串（`crates/tauri-app/src/commands/connections.rs:432-437`）。
建议：`#[serde(skip_serializing)]` + 专用持久化形态；自定义 `Serialize` 统一为字符串。**置信度：代码事实高（llm.rs 部分为子代理证据，标记疑似-中；未亲自读该文件行）。**

### D-22 · P3 · 类别 C · 死代码/不可达分支（全 workspace grep 证据）

- `crates/domain/src/card_studio.rs:1759 drafts_to_st_book`（`#[allow(dead_code)]`，零调用者；compile 走 `world_info::WorldInfoBook::to_st_book` `:1106`）——被替代的第二条出卡路径未删。
- `card_studio.rs:1136/1143 apply_mvu_bootstrap_entry` + `MVU_INITVAR_MARKER`：只有自身测试调用，无 app/tauri 入口。
- `card_studio.rs:1191 extra_definitions_from_st_extensions`：无生产调用者，而模块注释 `:97-98` 声称导入侧会接它。
- `card_studio.rs:1021-1025`：不可达告警分支（`compile_artifacts` 在 `:982-991` 已因 `run_checks` 报错返回 Err，而 `run_checks` 对 `!constant && keys 全空` 已有硬错误 `:814-822`），且文案"已跳过 keys 校验"与事实相反。
- `card_shell.rs:22 CardShellKind::Other`：全 workspace 未构造。
- `prompt_module.rs:143-148 AgentBinding`：全 workspace 无引用。
- `preset.rs:207 enabled_system_prompts`、`world_info.rs:106 matches_query`、`world_info.rs:203 search_by_keywords`（纯别名）、`message_layout.rs:101/114 prefix_fingerprint/full_request_fingerprint`（仅测试）、`novel_distill.rs` 整模块（②D-17）、`agent.rs:724 to_chronicle_a`（仅测试）。
**置信度：中（子代理全 workspace grep 证据；我抽查了 `card_studio.rs:572-597/908-924/1605-1684`、`card_shell.rs`、`agent.rs:660-781` 等相邻区域，未逐条复核每一处 grep）。建议 Lead 侧用 `cargo +nightly -Zunused` 或按清单逐条 `grep -rn` 终判后再删。**

### D-23 · P3 · 类别 B · `crates/domain/src/world_info.rs:97-99`、`:223-231`、`:238/244`

- `set_enabled` 的 `Err` 分支不可达：`default_route()`（`:82-89`）对 `(bool,bool)` 穷尽且只返回 `Both|Constant|Selective`，永不为 `Disabled`，故 `:97-99` 死代码（调用方的 `?` 是装饰）。
- `from_st`（`:223-231`）与 `default_route()`（`:82-89`）重复编码同一套蓝绿→路由映射，需手工同步。
- `insertion_order`/`extensions.depth` 用 `as i32` 有损截断（`:238`、`:244`），第三方卡的越界值静默回绕。
建议：删死分支或改 `Option<LoreRoute>`；抽公共映射函数；`i32::try_from(...).ok()`。**置信度：高（我亲自读过 `:60-104`、`:200-293`）。**

### D-24 · P3 · 类别 B/D · `crates/domain/src/card_studio.rs:572-591`、`card_shell.rs:167-178`

- `reverse_parse_character` 用 `..WorldviewDraftEntry::default()` 重建条目（`:587`），丢掉 `probability`/`exclude_recursion`/`group`（compile 侧在 `:1027-1037` 写入 extensions），"从已有卡反解析"的 Mode-C 修订会静默丢失触发概率/递归排除/分组；`export_gate_checks` 不校验这些字段。
- 内联 HTML 壳的阈值单位不一致：`replace.chars().count() > 80`（字符）与 `replace.len() > 8_192`（字节）混用（`:171`、`:174`），约 3000 字中文 viewer 壳（≈9KB）被清空为 `InlineHtml{html:""}`；tauri 侧对 inline_js 有 `deferred/byte_len` 延后取回通道（`crates/tauri-app/src/commands/card_shell.rs:44-55`），InlineHtml 无等价通道且 `get_card_shell_inline_js` 只返回 InlineJs。因前端目前把 InlineHtml 当触发锚点用，故 P3。
**置信度：高（我亲自读过 `card_studio.rs:572-597`、`card_shell.rs:167-191`）。**

### D-25 · P3 · 类别 C/B · `crates/domain/src/card_shell.rs:293-315` vs `frontend/src/utils/cardShellDisplay.js:43-50`

Rust `classify_shell_kind` 与 JS `classifyShellUrl` 是两份关键词表，JS 注释明确声称与后端一致，但 JS 只镜像了 URL 分支（缺 `首页`/`状态栏`/`statusplaceholder`），且优先级相反（Rust 先 home 后 status，JS 先 status 后 home）→ 含 `/home/` 与 `/status/` 的同一 URL 两侧分类不同。另 CLAUDE.md 声称的 `matchesAnyInlineShellTrigger` **只存在于 JS**（`cardShellDisplay.js:257`），Rust 侧只透传 `trigger: find`（`card_shell.rs:158/184`）——这不是代码缺陷，但文档若被读成"Rust 有匹配实现"会误导。
**置信度：中（子代理全文件证据；我未逐行复核 JS 文件）。跨域：建议域5/域6 决定单一真相源。**

### D-26 · P3 · 类别 B/E · `crates/domain/src/preset.rs:43-69`

`RegexScript` 的 `#[serde(default)]` 不对称：`placement_codes`/`source`/`trim_strings` 有默认，`disabled: bool`、`flags: String`、`placement`、`find_regex` 等没有；`Preset` 是从 `presets.json` 整体反序列化的（`crates/tauri-app/src/preset_store.rs:26`），任一脚本缺键即整份列表解析失败并回退 `.tmp`/默认。结构不对称已核实；**是否存在此类历史数据未确认（疑似）**，故 P3。建议补齐 default 或逐条解析。**置信度：结构高 / 影响低-中。**

---

## 4. 目标完成度核对表（声明 → 代码证据 → 判定）

| # | 声明（来源） | 代码证据 | 判定 |
| --- | --- | --- | --- |
| 1 | `CharacterInstance` 字段集 = id/campaign_id/definition_id/name/persona_override/behavior_override/variables/is_temporary（CLAUDE.md、DATA_MODEL §118-129） | `campaign.rs:293-307`（字段列表）；构造器 `:326-334` | **达成** |
| 2 | `CharacterInstance` 无 `backstory_override`/`variable_schema` | 同上，字段集合中不存在 | **达成** |
| 3 | `CharacterDefinition` owns persona_prompt/behavior_rules/base_backstory: Vec<String>/variable_schema（+group/role_type） | `character.rs:536-559` | **达成** |
| 4 | `resolved_persona(definition)`/`resolved_behavior(definition)` 接 `Option<&CharacterDefinition>`，override 优先、缺失回退 definition | `campaign.rs:384-401` | **达成** |
| 5 | `CampaignRuntimeContext` 是纯域快照（无 store/lock/Tauri），字段 campaign/instances/definitions_by_id/knowledge/tasks/turn | `campaign_runtime.rs:24-33`；crate 依赖表无 tauri | **达成** |
| 6 | 辅助方法 `find_instance_by_id_or_name`/`definition_for_instance`/`resolved_*_for`/`with_temporaries_for` | `campaign_runtime.rs:37-141` | **达成** |
| 7 | `with_temporaries_for` 返回 `Vec<CharacterInstance>` 供调用方落库，并对同批重复未匹配角色去重 | `campaign_runtime.rs:97-141`（小写 id/name 去重集合） | **达成** |
| 8 | `CharacterInstance::temporary_with_overrides(campaign_id,name,persona?,behavior?)` | `campaign.rs:326-345`（`temporary` 转发）+ 测试 `:812-870` | **达成** |
| 9 | `persist_temporary_instances_to` A.1 后仅单测使用；`persist_temporary_instances_async` 已删除 | `tauri-app/src/runtime_support.rs:1507`；调用者只有 `lib_tests_writing.rs:1575/1611/1644/1667/1689`；async 版本全 workspace 零匹配 | **达成** |
| 10 | Campaign 双 `story_clock`：顶层字段 + variables，语义以 variables 为准 | `campaign.rs:31-35` + `variables.rs:105` | **部分达成**：双权威存在且已接 repair，但两处默认值不一致（D-06）；DATA_MODEL §97 "未来可通过数据迁移统一"已滞后于现有 repair 实现 |
| 11 | Chronicle A/B/C：`ChronicleLevel`/`ChronicleCode`/`ContextEpochSnapshot`/`covered_by` 折叠/压缩发布纯函数 | `chronicle.rs:79-141`、`:448-554`、`:911-1013` | **达成** |
| 12 | `RoundSummary` 是 A/B/C 统一存储形态，字段 id/campaign/conversation/turn/content + code/headline/lineage_id/covered_by/level/turn_end/covers，转换 `to_chronicle_a`/`from_chronicle_entry`（DATA_MODEL §30-33） | `agent.rs:661-781`（字段与两个转换均有） | **达成**（但转换为有损，见 P3-未单列：`from_chronicle_entry` 丢 full/source_*/origin_*/invalidated_at；今天无生产者填这些字段） |
| 13 | Turn/Attempt 双状态机正交，`is_terminal` 覆盖 Committed/Degraded/Failed/Abandoned，`draft_hash` + variant 精确匹配决定 accept | `turn.rs:28-110`、`MutationBatch::new`（target=expected+1）与 `TurnAttempt{variant_id,draft_hash,pending_temporary_instances}` | **达成** |
| 14 | `AgentRunConfig` 三字段全 Option；`AgentProfileConfig` 六字段；`effective_max_concurrent_subagents()` clamp 0→1；内置 ID `builtin-default-agent-v1` 不可删；`ProfileConfigError` 三变体；`validate()` 三项检查且不校验工具名；无 temperature | `agent_profile_config.rs:47-68`、`:76-100`、`:172-174`、`:118`、`:13-20`、`:216-236`；`temperature` 在该文件零匹配 | **达成** |
| 15 | `migrate_to` v1→v1 no-op、未知版本更新版本号保留数据 | `agent_profile_config.rs:242-250` | **部分达成**：no-op 正确，但注释与代码矛盾且会降级更高版本（D-14） |
| 16 | MVU 键归一化 canonical（点记法/无 `stat_data.` 前缀/模板段 `{}`）+ 前端镜像 `mvuKey.js` | `variables.rs:170-191`；`frontend/src/utils/mvuKey.js:7-22` | **达成**：我逐条比对两侧算法（含 `trim_start_matches('/')` vs `replace(/^\/+/)`、`while strip_prefix` vs `while slice`、段 `<>`→`{}` 条件），语义等价，未见漂移 |
| 17 | domain 层无 tauri/网络/IO 依赖 | `crates/domain/Cargo.toml`（serde/serde_json/serde_yaml/chrono/uuid/thiserror/tracing/sha2）；全 crate grep `tauri|reqwest|std::fs|std::net|tokio` 仅命中注释里的 "tauri-app" 字样；唯一非确定性来源为 `Utc::now()`/`Uuid::new_v4()`/`std::process::id()` | **达成** |
| 18 | card-studio 出卡闸门含三维保真：content/keys/insertion_order roundtrip | `card_studio.rs:1352`（CRLF+trim 归一）、`:1367`、`:1399`、`:1417`（均有测试 `:1914-1955`） | **达成** |
| 19 | card-shell `classify_shell_kind` 含 开场/intro//intro/ → OpeningCustom，状态判定先于开场 | `card_shell.rs:296-315` | **达成**（细节：`intro` 仅在 label 命中，`开场` 在 find/label 命中，`/intro/` 在 URL 命中） |
| 20 | `matchesAnyInlineShellTrigger`（内联壳触发匹配） | 只存在于前端 `frontend/src/utils/cardShellDisplay.js:257`，Rust 仅透传 trigger | **达成/不适用**：CLAUDE.md 只描述前端行为，未见 Rust 侧同类声明 |
| 21 | `export_gate_checks` 中 `expected_keys` 与 `actual_keys` 的归一一致性 | —— | **无法判定**：子代理发现两侧归一不同（expected 做了 trim/空过滤，actual 原始比较），我未复核该段 |
| 22 | ARCHITECTURE-AUDIT.md 的"阶段 1-6"叙述 | 文档头部自标注为 2026-06-17 历史快照 | **不作为事实来源**（文档已自我废弃；其"临场角色 postprocess 前落盘"表述已被 A.1 取代） |

---

## 5. 未发现问题与低风险观察

### 5.1 未发现问题（已核对范围）

- `crates/domain/src/narrative_contract.rs` — `extract_gate_probes`（`:80-118`）的字节扫描：needle 为 ASCII，token 结束位置只会停在非 ASCII/非 `[A-Za-z0-9_-]` 字节，`&t[start..i]` 始终落在 char 边界（**UTF-8 安全，已核实**）；`owner_of_secret`/`binding_for_probe` 的空串误归属风险被调用方守卫住（`app-pipeline/src/quality_gate.rs:281-296` 先做 `chars().count() < 4` 过滤且要求 `text.contains(token)`）。— 未发现问题。
- `crates/domain/src/character_knowledge.rs` — `PropagationPolicy` 的 `#[serde(default, skip_serializing_if = "is_open")]` 与旧 `knowledge.json` 兼容、`into_entry` 传播字段一致性：`character_knowledge.rs:32-45`、`:77-82`、测试 `:523-537`。— 未发现问题。
- `crates/domain/src/variables.rs` — `normalize_mvu_key`/`normalize_schema_keys` 的幂等性与冲突处理：我逐例推演（`/世界/时间`、`stat_data.stat_data.hp`、`<a>.<b>`、`hp.`、`a/b`、`/`）确认二次调用不变；`normalize_schema_keys` 对空键保留 label 的行为有测试（`:670-683`）。— 未发现问题（唯一观察：大写 `STAT_DATA.` 前缀不收敛，见 5.2）。
- `crates/infra-util/src/write_fence.rs` — 全文核对：`freeze` 在持锁区间内写 `ANY_FROZEN`，`unfreeze` 同理，`is_frozen` 的"先读原子布尔再查集合"不会产生"集合非空但返回 false 且已生效"的窗口（只会读到 freeze 生效前的旧值）；`lock().unwrap_or_else(|p| p.into_inner())` 对 poison 的处理安全；键用 `to_string_lossy`（同进程同构造方式，注释已说明）。— 未发现问题。
- `crates/infra-util/src/lib.rs` — `atomic_write` 的 `.tmp` + `sync_all` + rename 重试 + 保留 `.tmp` 语义与注释一致；`write_fence` 前置检查在写之前。— 未发现问题。
- `crates/infra-util/src/secret_store.rs` — 全文核对：只接受 `storyforge-secret:v1:` 引用；`ensure_with_init` 只缓存成功（Android 失败可重试，`catch_unwind` 包裹）；`delete_secret` 把 `NoEntry` 视为成功（幂等）；明文透传是**有测试的有意降级**（`:210-233`）。— 未发现问题（风险提示见 5.2）。
- `crates/infra-regex/src/lib.rs` — 核心引擎：`find_ascii_case_insensitive` 用同长 `to_ascii_lowercase` 保边界（`:282-290`），tag 扫描按 `open_end`/`close_end` 推进边界（`:300-340`），`MAX_REGEX_INPUT_LEN` 上限先于执行（`:398-407`），超时线程 detach + `TIMED_OUT_SPECS` 防重复泄漏（`:296-345`），`parse_st_regex_spec` 斜杠字面量与 flags 合并有测试。— 未发现问题（两处观察见 5.2）。
- `crates/domain/src/chronicle.rs` 压缩纯函数 — `partition_compress_groups`/`validate_compress_covers`/`publish_compress_batch`/`next_code_seq`：分组连续性、覆盖校验、`covers` 与 `covered_by` 一致性、序号推进均有实现与测试；`ChronicleCode` 只能由 `new`/`parse` 构造（ASCII 前缀+数字），`next_code_seq` 的 `[1..]` 切片被 `level()` 短路保护。— 未发现问题。
- `crates/domain/src/story_task.rs` — `check_trigger` 的两遍扫描顺序（确定性触发器先于 Event）符合其注释动机（`:137-167`）。— 未发现问题（契约问题见 D-04）。
- `crates/domain/src/history.rs` — 逻辑本身：未知节点报错、轮次-对话范围校验、生成/提交中拒绝、已提交内容守卫、水位钳制均正确。— 未发现问题（**但零测试**，见 D-18）。
- `crates/domain/src/generation.rs` — 子代理全文核对（explicit_mode 短路、4+/3+大场景/3+2 任务 → SequentialCrew、dual 评分与 `==2` 门槛、EconomyDefault 回退、wire 值）**未发现缺陷，仅测试缺口**（actor_count 0/1、explicit BigScene 无测试）。**置信度：中（子代理证据，我未逐行复核）。**

### 5.2 低风险观察（不构成发现，但值得知道）

1. `narrative_contract.rs:227-233 owner_of_secret("")` 会返回第一个 binding（`b.secret.contains("")` 恒真）；今天的调用方有长度守卫，属"API 对空输入不安全"的潜在陷阱，建议加 `if secret.trim().is_empty() { return None }`。
2. `secret_store.rs:165-174 resolve_secret_value` 在 keyring 读取失败时返回 `Err`——调用方**不得**把 ref 文本当 API key 回退使用（会把 `storyforge-secret:v1:...` 当密钥发给服务端）。属跨域契约，建议域4/域2 核查 `infra-llm`/`connection_store` 的 fallback 路径。
3. `infra-regex`：超长输入（>1MB）返回的是 `RegexError::Compile`（`:401-407`），语义上更像 `Replace`/新变体，调用方若按"正则语法错误"提示用户会误导；`TIMED_OUT_SPECS` 是进程级永久短路（一次超时后同 spec 本进程内不再执行），对卡内合法但慢的正则是"整进程禁用"，建议至少在错误文案里说明。
4. `infra-vector`：崩溃恢复里 `.tmp` 主备份路径有一个良性缺口——主文件损坏但 `.tmp` 可解析时，`.tmp` 的内容被载入内存但 `.tmp` 文件保留、下一次 `persist_records` 用主路径原子写覆盖 `.tmp`（`atomic_write` 语义），不会丢数据但会留下陈旧 `.tmp`。
5. `variables.rs:170-191` 归一化对 `STAT_DATA.`/`Stat_Data.` 大写前缀不收敛（大小写敏感），若 LLM 输出大写容器前缀会生成与既有键不同的新键；分析器提示词已钉死记法，风险低。
6. `chronicle.rs:53-55 PendingCompressPublication.child_ids` 为"仅反序列化、新写入不再使用"的兼容字段——保留是刻意的，但若未来清理需先确认无历史数据依赖。
7. `message_layout.rs` 的 `prefix_fingerprint`/`full_request_fingerprint` 无生产调用者（生产用 `segment_fingerprint` + harness 侧 observability），存在"观察指标与实际口径不一致"的沟通成本。
8. `card_shell.rs:167-178` 内联 HTML 被清空时**不留标记**（对比 tauri 侧 `deferred/byte_len` 通道），前端只能看到"没有 html"。

---

## 6. 需要 Lead 重点复核的结论

1. **D-01 的定级**：panic 在 Tauri 命令内（unwind 到 IPC 边界）。我按"可由用户卡数据触发的未处理 panic = P0"定级；若项目约定把命令级 panic 视为"可恢复错误"，可降为 P1——但无论定级如何都应修，且修复只需复制 3 行守卫。
2. **D-03 的定级**：向量库不是 Campaign 真相源（远记忆可重建），我定 P1；若产品把 `CharacterKnowledge` 向量视为不可再生，则应升 P0 并优先接入 `write_fence`/`storage_health`。
3. **D-04 跨域裁决**：需要域3确认 `turn_dossier::compile_turn_dossier`（`lib.rs:1029/1219`）与 `build_director_tail`（`lib.rs:3333`）在写作主链路的实际分工；若 dossier 是唯一正文注入源，StoryTime 类任务等于完全不可用（P0/P1 上限）。
4. **D-06 跨域影响**：新建 Campaign 恒为"分歧"状态，域2/域4 的加载路径会因此每次发 warn 并改写字段；统一默认值前，`import/export` 的字节比对会继续依赖 `import_export.rs:141-151` 的显式 repair 绕行。
5. **子代理证据、我未逐行复核的条目**：D-21 的 `llm.rs` 两项、D-22 的死代码清单（多为全 workspace grep 结论）、D-25 的前端 JS 对比、D-26 的 `preset_store` 行为、§5.1 的 `generation.rs`。这些在采纳前建议 Lead（或对应域）二次确认行号与调用者。
6. **D-15 / D-26 的"影响"**依赖历史数据是否存在缺键记录，我无样本可证（标注疑似）；若 Lead 能访问用户数据目录（`data/card_projects.json`、`presets.json`），一次 grep 缺字段即可定论。
7. **测试总缺口（E 维度汇总，供 Lead 排工）**：`history.rs` 零测试；`capture_es_module_urls` 零测试；`switch_to_nearest_active` 零测试；`h_anchor = 0` 零测试；`migrate_to` 降级零测试；世界书 `enabled` 往返零测试；向量库关键词顺序稳定性零测试；`summary`/`story_clock` 新档不分歧零测试；嵌套宏、Single 互斥、空 description 覆盖、`order` 越界均零测试。
