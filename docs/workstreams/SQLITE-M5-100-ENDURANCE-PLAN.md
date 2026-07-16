# SQLite + M5 100 轮真实模型集成计划

> 状态：历史 Full100 已完成；Native 12 + TextFallback 3 专项补测进行中。
> 基线：main @ 75385e8。
> 执行方式：用户已授权直接在 main 工作；分阶段提交、不 push。
> 结果文档：补测完成并离线核验后更新 `SQLITE-M5-100-ENDURANCE-RESULT.md`。

## 1. 为什么不能直接把现有 100 轮改成 SQLite 模式后开跑

当前已有一部分真实 SQLite 接线：启动选择、JSON 到 SQLite 的切换、Conversation
持久化、Turn 的通用保存/更新、Accept、恢复和活动 Turn 屏障在 opt-in SQLite
模式下已有权威路径；默认后端仍是 JSON。

但是这不足以构成“SQLite 写作生命周期已完整接入”，也不足以构成“100 轮全
agent 覆盖”。当前事实如下：

| 项目 | 当前状态 | 为什么不能作为本次验收 |
| --- | --- | --- |
| SqlitePreacceptRepository | repository、migration、故障注入测试已存在 | Tauri 和 harness 尚未调用它；首稿、autofix、postprocess、regenerate、edit-stale 仍是分步持久化 |
| 当前 endurance runner | 可调用真实模型并保存 checkpoint | HarnessEnv 固定使用 JSON CampaignStore、ConversationStore、TurnStore；Accept 走 JSON CommitProbe |
| ScheduledAction | 有 100 轮覆盖表 | 大多数 action 只变成 intent 文本/证据标签；不会真的调用 regenerate、autofix、Meta、cache 失效或模式切换 |
| postprocess/Chronicle | 有共享服务和 deterministic proof | 当前 endurance 记录 synthetic chronicle fixture，且 production_postprocess_complete 为 false |
| Meta | 有一批确定性和 ignored real-LLM 测试 | 多个 Meta 命令仍依赖 legacy CampaignStore；SQLite 模式不能把它们当作已支持功能 |

因此，只设置 STORYFORGE_STORAGE_BACKEND=sqlite 然后执行现有 Full 100，会得到
一个“JSON harness 的普通写作耐久跑”，不能宣称 SQLite、全 agent、Meta、
真实 postprocess 或各项功能已验收。

本计划的核心原则是：先让同一条生产写作路径真正使用 SQLite pre-accept UoW，
再让 endurance runner 调用那条路径；只要某项没有实际调用记录和 SQLite 落盘
后置条件，就不计入覆盖。

本计划中的“真实模型”不是手机或 GUI 真机验收。它不产生 GUI、设备、远端 CI、
签名或发布通过的声明。

## 2. 目标与边界

### 目标

1. 在 opt-in SQLite 模式下，把下列写作前状态机接到生产命令和同源的 eval adapter：
   首稿、autofix、postprocess、regenerate、编辑后 stale、恢复。
2. 每个上述操作以 SQLite 的单事务 UoW 完成，或显式 fail-closed；不得通过
   JSON fallback、双写、影子数据库或事后补写伪造原子性。
3. 将 100-turn runner 从“计划标签”升级成“CoverageRow 实际动作分派 + 可观测
   调用账本 + SQLite 后置条件”。
4. 在独立、可恢复、无敏感内容的 evidence root 下，使用 SQLite 权威路径完成
   新的 3、12、30、100 accepted-turn 分阶段真实模型运行。
5. 为主写作 agent、角色卡提取、Meta 和关键写作功能提供精确覆盖报告；未支持
   的 SQLite Meta 功能必须明确写成 unsupported，不能静默回落 JSON。

### 明确不做

- 不改变默认后端；没有环境选择时仍使用 JSON。
- 不复活、不续跑、不引用历史 45/100 或其他已清理的耐久证据。
- 不把本地私有角色卡正文发送给模型；除非另有明确授权。
- 不记录或输出 API key、原始 prompt、故事正文、私密知识、完整模型响应、绝对
  主机路径或数据库内容。
- 不把 Meta、GUI、手机设备、Android、远端 CI、签名或发布结果混入本次声明。

## 3. 阶段门槛

每一关失败都必须停止在该关，保留诊断和已验证证据；不能跳关运行真实模型。

| Gate | 允许开始前提 | 产出 |
| --- | --- | --- |
| A：SQLite 接线 | 工作区干净；先写失败测试 | 真实生产路径使用 pre-accept UoW 的 command/shared-adapter 集成测试 |
| B：实际覆盖 | Gate A 全绿 | CoverageRow 到实际命令/服务/SQLite 状态的 exact-set 证明 |
| C：真实模型 | Gate A、B 和确定性门禁全绿 | 新 run ID 的 Canary 3、Coverage 12、Stability 30、Full 100 |
| D：结果封存 | Full 100 已通过，或发生诚实 partial | seal、offline verify、secret scan、RESULT 和最终门禁 |

## 4. Gate A：SQLite 写作前生命周期接线

### 4.1 必须接通的事务

不得只在 infra-sqlite 测试中调用 repository。Tauri 真实命令和 M5 adapter 必须
共同调用同一个生产拥有的生命周期边界；不要分别复制一套 SQLite 写法。

| 操作 | 必须原子写入 | 必须拒绝的情况 |
| --- | --- | --- |
| 首稿 | conversation variant、Turn、Attempt、DraftReady outbox | campaign/conversation/attempt 不匹配、Committing、持久化故障 |
| autofix | 更新后的正文、draft hash、完整 canonical QualityReport、outbox | stale、superseded、重复但 payload 不同、scope 错误 |
| postprocess | derivation、MutationBatch、AwaitingAcceptance、outbox | cancel、late completion、missing attempt、scope 错误 |
| regenerate | 旧 Attempt supersede、新 variant、新 Attempt、outbox | Committing、错误 previous variant、scope 错误 |
| 编辑 | 编辑后的正文与对应 Attempt=Stale、outbox | terminal 或 Committing Attempt、scope 错误 |
| 重启恢复 | preaccept outbox 与 Turn/Attempt 状态一致 | schema drift、半写、重复 replay |

实现可在 sqlite_runtime 中增加 pre-accept gateway，或抽取 Tauri 与 harness 同用的
应用服务；但它必须在同一 SQLite 权威库上工作。若 pipeline 目前在 UoW 之前就
写入 Conversation，先拆开“生成结果”和“持久化落点”，否则无法事后声称原子。

### 4.2 必须新增的命令级测试

每条测试都必须从真实 command/shared production adapter 进入，而不是直接调用
repository 单元 API：

1. JSON 初始数据完成 cutover 后，移走或禁止读取 legacy JSON 源。
2. 首稿到 Accept：draft -> autofix -> postprocess -> AwaitingAcceptance -> Accept。
3. regenerate、edit-stale、restart recovery 的独立分支。
4. 每个 UoW 的故障注入：conversation、Turn、Attempt、outbox 均不得留下部分写入。
5. scope mismatch、attempt missing、cancel、late completion、supersede、Committing：
   新 Attempt、旧 Attempt、conversation、failure_reason 都保持零误写。
6. 从 pre-accept state 进入 SQLite Accept，关闭后重开数据库验证一致性。
7. 验证 JSON 无 I/O、无内容变化；默认 JSON 路径回归仍绿。

Gate A 通过后，先提交一个可独立审查的 main commit，例如：

~~~text
feat(sqlite): wire preaccept lifecycle through production writing path
~~~

## 5. Gate B：把 100 轮从“计划标签”变成真实覆盖

### 5.1 同源 SQLite endurance adapter

现有 HarnessEnv 和 CommitProbe 不能直接复用为 SQLite 验收，因为它们固定构造
JSON store。新增或重构后的 adapter 必须：

- 启动与 Tauri 相同的 opt-in SQLite 选择、cutover、runtime 激活和恢复；
- 通过 Gate A 的同一生产写作生命周期服务执行首稿、postprocess、regenerate、
  edit 和 Accept；
- 不手工构造 Final Turn、synthetic summary 或 JSON CommitProbe 来伪造成功；
- 以安全摘要证明 sqlite_authoritative=true 与 json_fallback=false；
- 在每个关键 action 后能重开或查询 SQLite 验证状态，而不暴露正文。

### 5.2 覆盖账本合约

每一个 CoverageRow 必须同时有：

1. 计划动作 row_id；
2. 实际执行的 command/service path；
3. 实际 agent/tool 事件；
4. 关联 operation/correlation、Turn、Attempt、Variant 的短 hash；
5. SQLite 后置条件和状态；
6. 预期集合与观察集合的 exact-set 比对。

只写 ScheduledAction、intent、role=pipeline 或“已注入 probe”都不算观察证据。
任何 required row 缺失、重复、额外、只计划未执行或后置条件失败，都使 Full 失败。

每个模型调用或本地 agent 事件只记录角色、计数、枚举、成功/失败、短 hash 和
安全指标；不得记录正文。

### 5.3 主卡：可审计的合成多角色矩阵卡

主 100 不使用用户私人角色卡，也不使用临时本地 PNG。新增一个提交到仓库、
版本固定、完全合成且脱敏的 fixture，例如：

~~~text
crates/harness-real-llm/fixtures/m5_sqlite_endurance_v1.json
~~~

它至少需要：

- 主角、支援角色、临时角色，以及两个显示名相同但 ID 不同的角色定义；
- 角色关系、任务、变量和可验证的 instance ID；
- constant、selective、both 三种世界书路由；
- owner-only private knowledge 与 must_not_reveal 规则；
- 三条安全 probe 事实，能被 Summary/Chronicle 实际查回；
- 最小 MVU schema；需要时可含受控 TavernHelper/Regex 兼容字段；
- 版本、fixture hash、definition/knowledge/world-info 计数。

真实本地卡只可作为独立的 import/extract 或 MVU smoke。若日后要把真实卡文本送至
外部模型，必须先取得单独授权；证据只能保存 fixture hash、大小和计数。

### 5.4 写作覆盖矩阵

下表是最低要求。普通写作轮填充连续性；特殊操作必须真正调用目标 API，不能仅把
同一句说明塞进 user intent。

| 行 | 触发 | 真实要求 | 最低 SQLite/证据后置条件 |
| --- | --- | --- | --- |
| 基线写作 | 第 1 轮及其余普通轮 | Director、实际 1/2/3 个 Subagent、Editor、QualityGate、真实 Summarizer、真实 PostProcessor | 每个角色有 observed event；proof、batch digest、Attempt 与 Accept 均落 SQLite |
| 多角色身份 | 至少 2 行 | supporting、temporary、同名不同 ID 实际参与 | instance ID short hashes 可区分，不能靠显示名判断 |
| CharacterExtractor | setup 独立 probe | 从主 fixture 导出/验证定义，而非 fallback_from_character | definition/role 计数与 hash；无原文 |
| reasoning | 三个正常写作行 | Disabled、Native、Prompted 真正进入请求配置 | observed request mode 与计划值相等 |
| tools | 至少 Native 与 TextFallback 各一行 | 真正切换可支持的工具模式 | actual mode 与 tool event；不支持则 fail-closed/unsupported，不可伪造 |
| world-info | Constant、Selective、Both 各一行 | 编译器实际选择相应路由 | route、选中数、输入 hash |
| regenerate | 11/37/73，18/54/90，25/61/97 可调整 | Overall、Editor-only、Subagent-only 真正调用 | target、variant lineage、未误伤其他 Attempt |
| QualityGate | 正常路径和受控负向 probe | 至少一条真实 autofix；一条非修复型错误在 100 外或不计入 accepted | complete QualityReport、autofix count、失败不污染主链 |
| 私密知识 | owner、non-owner、narration、must-not-reveal | 真正安装规则并检查输出 | 仅安全 pass/fail/hash；泄漏直接失败 |
| Chronicle/事实 | 注入 3、8、13；查询 35、65、95 | 通过 SQLite Summary/Chronicle 实际检索 | expected probe exact-set 与 checked-pass exact-set 相等 |
| cache/epoch | 稳定轮与明确失效轮、跨 epoch | 真正的 request fingerprint/cache 指标变化 | 原因、epoch、稳定/失效观察；CacheStable 必须实际调度 |
| restart | 至少一次受控 checkpoint/restart | 恢复后继续同一 run，不重放 accepted Turn | integrity baseline、run ID、SQLite counts、revision 单调 |

当前全局工具/模型连接若只能在进程启动时设定，不能用“调度标签”冒充切换。此时应当
通过独立、明确的子运行或可注入配置实现每个模式，并在 RESULT 说明运行边界。

## 6. Meta 覆盖：要测，但不能混充为 100 个写作 Accept

Meta 是独立命令家族，部分操作会变更 Campaign；它们在活跃 Turn 期间本来就应受
屏障保护。正确做法是用同一 SQLite 数据库的安静点或受控克隆运行有界 Meta matrix，
并把结果与 100 个写作 accepted turns 分开报告。

| 类别 | 时机 | 最低测试 | SQLite 要求 |
| --- | --- | --- | --- |
| 只读 health | 第 10、60 轮完成后的安静点 | healthy 和 broken snapshot | 读取 live SQLite，不读 legacy JSON |
| generation explain | 至少一个已 Accept variant | explain/provenance 可返回 | 关联 SQLite conversation/variant |
| typed patch | 独立 DB clone | propose、preview、accept、dismiss、stale、活跃 Turn 拒绝 | 先实现 SQLite-native adapter；否则明确 unsupported |
| Meta chat | 独立测试 campaign | 1 至 2 个有预算的真实模型回合与工具事件 | 真实 Meta role/command 记录，不能只测普通 pipeline |
| MVU | 独立 clone | analyze、preview、apply 和变量保留 | SQLite-native 实现后才可标 PASS；否则 unsupported |
| 其他 Meta 命令 | 文档化清单 | 从 Tauri handler 精确枚举 | 每项要么有 SQLite 测试，要么显式 unsupported |

现有若干 Meta 命令通过 legacy CampaignStore 读取状态。在 SQLite 模式下，这个 store
是 disabled sentinel；不能把 JSON 运行的 Meta 测试移花接木为 SQLite 覆盖。若 Meta
没有移植，本计划的正确结果是“Meta SQLite unsupported”，而不是“全功能通过”。

## 7. Gate C：真实模型运行协议

1. 先完成所有确定性门禁，再按 Canary 3 -> Coverage 12 -> Stability 30 -> Full 100
   顺序运行。每个 stage 都是新的或已完整验证的 controlled run。
2. 使用显式 STORYFORGE_STORAGE_BACKEND=sqlite 和隔离、gitignored、durable evidence
   root；不得使用用户正常数据目录。
3. 真实模型配置只从环境读取。记录非敏感 model label 和参数模式，但不记录 endpoint
   凭证或请求正文。
4. 不增加人为的 max_tokens=4096 或其他小硬上限。若生产耐久路径的架构语义是
   max_tokens 省略/None，就保持它；证据只记录 omitted/None 或非敏感架构值。
5. 在 Canary 和 12 轮阶段按真实角色调用数、retry 和 timeout 数据计算 Full 的明确
   硬 call/time budget。预算必须有上限，不能为了凑 100 无限重试；也不能因为旧的
   700 数字不足就静默吞掉 agent 调用。
6. 一次短暂中断可在同一 run ID 下恢复，但先做 evidence tree safety、hash、
   checkpoint integrity 和 exact-set 校验。不得把历史 run 拼入新 run。
7. 任一 SQLite authority、JSON fallback、secret scan、postprocess proof、required
   coverage row 或 data-integrity 失败都立即 fail-closed。模型临时错误仅可按已声明的
   有界重试策略处理。

Full 只有同时满足下列条件才是 PASS：

- 100/100 intended turns 到达 accepted terminal state；
- 所有 required CoverageRow 的实际观察集合精确覆盖；
- 所有三条 early fact 实际查回，不可用“曾注入”替代；
- 每轮的 SQLite preaccept、postprocess proof、Accept 与恢复后置条件成立；
- SQLite reopen/authority audit、evidence seal、offline verify 和全树 secret scan 均通过；
- Meta 结果按支持/unsupported 分开记录，未把 JSON 或 synthetic 测试误报为 SQLite；
- RESULT 明确 gui_device_claimed=false。

未达到以上任一条件时只能报告 Partial Evidence 或 Failed，不能把 M5 Full、SQLite
全接线或 Meta 全覆盖标成完成。

## 8. 确定性门禁与提交点

实现者应依据实际 Cargo package 名称运行所有受影响 crate，最低包括：

~~~powershell
cargo fmt --all -- --check
cargo test -p storyforge-infra-sqlite
cargo test -p storyforge --test sqlite_optin_lifecycle
cargo test -p storyforge --lib
cargo test -p storyforge-app-pipeline --lib
cargo test -p storyforge-app-agent --lib
cargo test -p storyforge-app-meta --lib
cargo test -p harness-real-llm --lib
cargo test -p harness-real-llm --test endurance_deterministic
cargo test -p harness-real-llm --test evidence_retention_deterministic
cargo test -p harness-real-llm --test m5_production_evidence
cargo clippy -p storyforge-infra-sqlite -p storyforge -p harness-real-llm --all-targets -- -D warnings
git diff --check
~~~

若代码改动涉及其他 app crate、Tauri command adapter 或 Meta，应补充相应测试和
clippy gate。先提交 Gate A/B 的代码与确定性测试；真实证据文件保持 gitignored，
随后只提交脚本、测试和 RESULT 文档。禁止 push。

建议提交点：

~~~text
feat(sqlite): wire preaccept lifecycle through production writing path
test(eval): make M5 coverage rows execute and attest SQLite actions
docs(eval): record SQLite-backed M5 100-turn evidence
~~~

## 9. RESULT 必填字段

结果文档必须如实给出：

- main HEAD、提交列表、工作区状态；
- 哪些 Tauri/shared production commands 已转到 preaccept UoW；
- 每个 SQLite 生命周期测试与故障注入的结果；
- role-card fixture 的版本/hash/安全计数；
- CoverageRow 计划数、观察数、缺失/重复数；
- Meta matrix 的 PASS、unsupported 与原因；
- run ID、stage、真实调用数、accepted 轮数、重试和恢复次数；
- sqlite_authoritative、json_fallback、postprocess_complete、seal/verify/secret scan 的结果；
- 所有剩余风险与未做的声明。

不得把 deterministic Phase B fixture、synthetic Chronicle、JSON harness、旧 checkpoint、
GUI/设备或未执行的 Meta 路径写成真实 SQLite 证据。
