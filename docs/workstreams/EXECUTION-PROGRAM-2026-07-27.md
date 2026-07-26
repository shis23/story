# 执行纲领（2026-07-27 晚，自主推进会话的完整状态快照）

> 本文档是上下文压缩前的状态固化。恢复工作时以此为唯一入口：
> 已拍板决策 + 执行队列 + 每项的精确修法都在这里。
> 全量问题清单（105 条，6 路并行扫描产出）：`OPEN-ISSUES-SWEEP-2026-07-27.json`。

## 用户已拍板的决策（全部有效，不要再问）

| 决策点 | 结论 |
|---|---|
| L7 重型卡（bgm/图鉴/cg）可见挂载 | **方案 A：消息区内嵌**（折叠面板默认收起、展开即挂载=用户手势、一次一个活跃） |
| L4 后端门禁 | **严格模式**（已完成，commit 0f87b24） |
| card-studio | **全做**：先收尾 Phase 1 金标（PNG 实机证据 + 导入后自动 extract 绑定），再起 Phase 2 蒸馏与 Phase 3 系统卡骨架 |
| git push 长期解 | **搁置**（继续用京东云隧道，见 memory push-via-jd-tunnel） |
| 发布方向 | **双线并行**：桌面收尾照走，Android 约束到 AND-2（系统选择器文件导入）/AND-3（数据目录+迁移）先行 |
| 发布 CI | **批准在京东云部署 act_runner** 注册 Gitea 实跑 workflow；本机跑 tauri build 出 Windows 安装包证据 |
| 战略实验 | **都做**：多 Agent 盲测（用中继，先出证据）+ 写作流水线重设计（按已定稿方案起骨架） |
| V4 损坏恢复 | **启动拦截 + 恢复提示**（检测到主文件损坏不再静默空集：前端弹恢复引导，用户确认前不写盘） |

## 执行队列与状态

### P0 已核实数据丢失 bug（来源 ARCHITECTURE-REVIEW-2026-07-26.md，均对抗核实确认）

- [x] **V1** regenerate present_chars 恒空 → **已修**（commit cd821d1）：
  `run_editor_and_commit`（app-pipeline，所有重 roll 粒度共同收尾）按
  start_writing 同语义填充 self.session；回归测试在 test_regenerate_full_with_hint。
- [x] **V2** 临时角色 postprocess 永久跳过 → **已修**：`apply_outcome` 从加载的
  TurnRecord 取本 attempt 的 `pending_temporary_instances` 传入批构建；
  JSON 路径经 `normalize_knowledge_update_for_postprocess_with_extras` +
  `find_instance_by_name_or_id_with_extras`（原签名保留委托，~30 个测试调用点不动）；
  SQLite 路径 `build_runtime_mutation_batch` 构建有效实例集（快照 + pending temps，
  按 id 去重、跨 campaign 拒绝）。同名收紧与广播受众均计入 temps。
  回归测试：`pending_temporary_instances_resolve_in_json_batch`（apply_outcome 全链路）
  + `pending_temporary_instances_resolve_in_runtime_batch`（正反对照）。
  CLAUDE.md Phase 6 过期事实已回写。
- [x] **V4** atomic_write 硬化 + 损坏启动拦截 → **已修**：
  infra-util `atomic_write` temp fsync → rename（3 次退避重试，Unix rename 后
  fsync 父目录），重试耗尽保留 .tmp 返回硬错误（去掉直写回退）；
  新增 `write_fence` 模块（路径级写栅栏，无 UI 依赖）。
  tauri-app `storage_health` 登记簿 + json_store 加载器接线：主文件损坏
  .tmp 恢复 → 提示事件；不可恢复 / IO 读失败 → `.corrupt` 备份 + 冻结写入
  + 阻断事件。命令 `storage_health_report` / `storage_health_acknowledge`；
  前端 `StorageHealthGate.vue`（AppV2 顶层）：阻断事件全屏拦截，
  「从空白开始（解冻）」或「稍后手动修复（保持只读保护）」；
  tmp 恢复事件仅提示条。注意：m5 `hard_deadline_covers_final_context_fill_boundary`
  因 fsync 每轮成本上升改为宽 deadline + 动态睡过期限（时序脆弱性根除）。
- [x] **V7** 三件 → **已修**：①infra-sqlite 新增 typed
  `SqliteError::RevisionConflict {campaign, turn_base, expected, target}`，
  accept_turn 的 revision CAS 用它；sqlite_runtime 适配层按类型匹配分类
  （integrity 分歧不再被子串 "revision" 误报成回合过期），RevisionConflict
  现携带真实 base/current。②重复 accept 统一幂等 Ok：JSON 路径
  `get_committed_turn_by_variant`（turn_store，只命中终态+accepted attempt
  的 variant）回退查找 + accept_by_variant 幂等重放分支（复用 attempt 上保留的
  pending_state_changes 重建 AcceptOutcome，零盘上副作用）；测试
  `duplicate_accept_replays_idempotently_after_commit` 断言重放输出与首次一致
  且 revision 只 bump 一次。③SQLite 内联质量门改调 domain
  `quality_accept_decision`（与 JSON 路径同源）。完整共享状态机抽取排后（周级）。

### P1 发布门禁（2026-07-27 全部完成）

- [x] vitest 组件层入 `scripts/verify-release.ps1`（新步骤 6/7 `npm run test:ui`）
- [x] Playwright UI smoke **修活**：@playwright/test 入 devDependencies + 本机装
  Chromium；spec 从旧 App.vue 假设重写为 AppV2 走查（role/name 定位：主界面 →
  侧栏 → 调试抽屉开/关 → 插件面板 mock 插件可见 → IPC 调用清单断言）。
  实跑绿：4 张截图 + test-results 无失败（artifacts/ui-smoke/20260727-030939）。
  smoke:ui 不入严格门禁（需 dev server + 浏览器），保持手动/CI 可选步骤。
- [x] CardStudio.vue 挂载冒烟 `tests/components-v2/card-studio.test.mjs`
  （空列表态 / 列表渲染+打开项目阶段条 / 从零创建调用链，3 用例）
- [x] 真卡验收异机复现指南补入 CARD-TRANSLATION-ACCEPTANCE-RESULT
  （env 总表 + 证据目录绝对路径坑位）

### P2 既定队列（今晚早些时候排定）

- [ ] **#19 L7-A**：重型卡内嵌挂载。内联 TH 应用（卿卿 bgm/图鉴/cg，57-99K 脚本）目前在
  0×0 不可见 iframe 执行。改为写作面折叠面板：默认收起 chip、展开才挂载（手势解锁
  autoplay）、一次一个活跃。入口数据：卡 manifest 的 InlineHtml 壳清单。
- [ ] **#21** 评测口径扩展：开场白 `<UpdateVariable>` 种子解析进 forge 差分作者树
  （destiny 剩余"幻觉"里 世界.新闻/事件.莉莉.* 疑似合法种子变量），重算缺口性质。
- [ ] **#22** SQLite 后端 MVU 规则收集补齐：`collect_mvu_update_rules_for_backend`
  SQLite 门跳过 → 实现（后处理【卡片变量更新规则】注入在 SQLite 后端缺失）。
- [ ] **#23** card-studio：P1 金标（导出 PNG 实机证据 + 导入后自动 extract/开 Campaign
  绑定）→ P2 蒸馏骨架（文档切分/文风公式/总结账本/断点）→ P3 系统卡骨架
  （MVU 生成对接现有 analyze/apply、多 CharacterDefinition、世界书高级策略）。
  现有代码入口：domain/card_studio.rs、tauri-app/card_studio_{api,store}.rs、
  components-v2/campaign/CardStudio.vue。
- [ ] **#24** 杂项：cardShellClearCache 按钮入电源模式设置区；vite 500kB chunk 分割；
  cargo/npm 依赖审计。

### P3 新专项（用户今晚拍板）

- [ ] **#30 CI**：SSH 京东云（root@111.228.49.176）装 act_runner，注册到本机 Gitea
  （127.0.0.1:3000，token 在用户全局 CLAUDE.md），让仓库现有 workflow 实跑出首次
  绿证据；本机 `cargo tauri build` 出 Windows 安装包。参考
  RELEASE-CI-EVIDENCE-RESULT.md 的 Unverified Items。
- [ ] **#31 盲测**：单 Agent vs 多 Agent 同提示多轮对比 + 盲评（中继
  https://cli.2529985.xyz/v1，key 用户每会话提供只进环境变量；模型
  deepseek-v4-pro/flash 可用）。产出决定流水线重设计的投入优先级。
- [ ] **#32** 流水线重设计起骨架（先找定稿方案文档：架构评审提到"已定稿未实现"，
  文档位置待查——搜 docs/ 里 pipeline redesign / 流水线重设计）+ Android AND-2/AND-3
  起步（PLAN-ANDROID.md 为准）。

### 周级工程（起步后持续，不指望今天闭环）

V5 应用级 CSP（tauri.conf csp:null）、V6 权限体系统一（L4 已打第一桩）、
V7 完整状态机抽取、V8 lib.rs 巨石拆分、Android Phase 6 全量、Phase 7 收口、
ST 99 事件全集、知识传播真实 LLM 对抗评测、插件直写变量 propose 化。

## 今天已完成并入库（截至本文档）

92d7677 MVU 键记法规范化（解析层+应用层+前端镜像，canonical=点记法/无前缀/{} 占位符）
6913051 harness 临时目录惰性清扫
a8e8b96 card-shell M1/M2/M5+L1/L2/L3/L5/L6（M2 键级补丁协议、M5 Mvu shim 真值桥接）
d4d0cd2 一次性脚本删除 + .gitignore 护栏
167b32c card-studio 出卡闸门 forge 三维保真（正文/keys/顺序）
8296fa6 InitVar 预算修复（50.9% 覆盖率之谜=确定性截断；重测 67.3/65.5%，两模型分数分化验证根因）
a9ab7ef tauri-app 测试目录清扫（360 个残留已清）
69098ce 文档回填（CLAUDE.md 事实 + FORGE 重测数字）
0f87b24 L4 严格后端门禁（plugin_get_conversation + fail closed）
ffee231 模板键 {角色名} 实例展开（状态栏+交互计划，卿卿 32 模板键驱动写作面）
cd821d1 V1 regenerate session 填充

真模型验收 4/4 绿（353s）；workspace 72 目标绿；前端 415+66 绿；已推送至 69098ce
（此后 0f87b24/ffee231/cd821d1 尚未推送——推送走京东云隧道，见 memory）。

## 运行配方备忘

- 真模型验收/差分：见 memory llm-acceptance-recipe（评测证据路径必须绝对路径）
- forge 产物在本会话 scratchpad/forge-out（destiny/qingqing），CLI 在 scratchpad/tavern-cards
- 推送：memory push-via-jd-tunnel
