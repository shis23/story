# R10 收口报告：R6 "无记录"项 W-31（前端 `validGenerationModes` 含 `big_scene`）

- 执行人：**review-frontend**（task-33）
- 日期：2026-09-13
- 来源：`round2/R6-fix-completeness-audit.md` 的 4 条"无记录"之一 **W-31 [P3]**（原始发现：`03-writing-pipeline.md:701-705`；R2 复检补充：`R2-pipeline-recheck.md` N-R2-09）
- 处置：**已修**（前端不再接受已下线的档位；后端兼容面未动）

## 1 事实核对（先复现，再动手）

| # | 声明 | 代码事实（2026-09-13 复核） | 判定 |
| --- | --- | --- | --- |
| 1 | `stores/writing.js` 的 `validGenerationModes` 含 `'big_scene'` | 原 `writing.js:16-21` 为字面量集合，含 `'big_scene'`；`:26-28` 用它过滤 localStorage 载入值；`:67` 用它守 `setGenerationMode` | ✅ 成立 |
| 2 | 档位目录没有 `big_scene`，ComposerBar 只渲染 3 档 | `utils/generationModes.js:1-21` 目录 = `continuation` / `duet` / `sequential_crew`；`design/writing/ComposerBar.vue:10,27,54` 只 `v-for="mode in generationModes"`（=目录） | ✅ 成立 |
| 3 | 后果：旧值被接受为当前档位并参与写作发起 | `writing.js:57-62`（campaign 作用域解析）→ `composables/useWriting.js:141` 作为 `generationMode` 传 `startWriting` → `tauri-api.js:394,406`（`generationMode: generationMode || null`）；`composables/useMessageVariants.js:167,400` 把同一值传给 reroll 请求 | ✅ 成立 |
| 4 | "看不见选中项" | `ComposerBar.vue:59,62` 用 `generationMode === mode.value` 决定高亮；`big_scene` 不在目录里 ⇒ 三个芯片**全不高亮**；`:70` 的成本提示 `find(...)?.callEstimate` 返回 `''` ⇒ 用户既看不到选中项、也看不到成本 | ✅ 成立 |
| 5 | 昂贵路径：`allowPartialReroll` | `adapter/useWritingScreenAdapter.js:89`：`writing.writingMode !== 'campaign' \|\| writing.generationMode === 'big_scene'` ⇒ campaign 会话下陈旧 `big_scene` 会打开 `allowPartialReroll`，再经 `utils/rerollPolicy.js:8`（`editorOnly = allowLegacyPartial && recordedMode === 'big_scene'`）落到 legacy 局部重 roll | ✅ 成立 |
| 6 | `big_scene` 在后端仍是**合法兼容模式** | `crates/domain/src/generation.rs:8-12` 枚举含 `BigScene`；`README.md:19`「旧并行 `big_scene` 保留为后端兼容模式」；`docs/ARCHITECTURE.md:56`「旧 `big_scene` 并行编排保留为显式兼容路径」；`docs/AGENT_INTERFACES.md:12`「`big_scene`：旧并行剧组，**仅保留后端兼容路径**」；`frontend/src/adapter/useWritingScreenAdapter.js` 之外，`rerollPolicy` 以"**上一次产物的** recordedMode"参与判定（后端 provenance），与"当前选择档位"是两码事 | ✅ 成立 |

**结论**：文档口径（仅后端兼容）与前端判定式（合法档位）矛盾，R2 的 N-R2-09 要求"二者取一"——本任务取**前端移出集合**（下方 §2 说明为何不取另一条）。

## 2 处置方案：为什么"移出验证集 + 回退默认档位"

| 方案 | 做法 | 理由 / 代价 | 选择 |
| --- | --- | --- | --- |
| **A（采用）** | 校验集**从目录派生**，`big_scene` 不再是合法档位；解析到它时回退默认档位 `continuation`；读取阶段**不重写、不删除** localStorage | 与产品面（3 档）和文档口径（"仅后端兼容"）一致；不回退旧数据；不引入新的昂贵路径。副作用是 legacy 局部重 roll 在 campaign 会话下不再可达——正是 W-31/N-R2-01 要消灭的"不可见昂贵模式"；非 Campaign 会话（`writingMode !== 'campaign'`）行为不变，符合 CLAUDE.md"不得破坏非 Campaign 路径" | ✅ |
| B | 保留 `big_scene` 为合法档位，但在 ComposerBar 里新增一档（映射为可见档位） | 会让产品面从 3 档变 4 档，与 `README.md:19`/`ARCHITECTURE.md:56`/`AGENT_INTERFACES.md:12` 的"仅兼容路径"定位冲突；且要重新定义它的成本提示与 reroll 语义（属产品决策，不是本 P3 的收口范围） | ❌ 不选 |
| C | 保留合法但把陈旧值**静默映射**到某个可见档位（如 `sequential_crew`） | 静默改写用户意图：`big_scene` 是并行全剧组、成本与顺序剧组不同，映射会让"用户以为在 A 模式、实际发出 B 模式"——比回退默认档位更糟 | ❌ 不选 |

**"不破坏旧数据"的具体含义**：`localStorage` 的 `storyforge:generation-mode-by-campaign` blob **一个字节都不动**（读取时只是不采纳 `big_scene`；用户为别的 Campaign 存的 `duet` 等值照常生效）。用户为那个 Campaign 重新选择任意合法档位后，blob 才被重写，此时陈旧值自然消失（`tests/components-v2/writing-generation-mode-store.test.mjs` 的"用户重新选择档位后，偏好 blob 里不再残留 big_scene"用例钉住这一点）。若未来重新上线该模式，前端恢复目录项即可，用户旧偏好仍在。

## 3 改了什么（4 个文件，3 类改动）

| 文件 | 改动 | 说明 |
| --- | --- | --- |
| `frontend/src/stores/writing.js` | 新增 `import { generationModeCatalog } from '../utils/generationModes.js'`；`validGenerationModes` 由 `new Set(catalog.map(m => m.value))` **派生**，删除 `'big_scene'` 字面量；加 6 行解释注释 | 派生写法让"校验集 == 目录值集"成为**结构性事实**：以后往目录加档位不会再出现"两处漂移"（这正是 W-31 的成因） |
| `frontend/tests/components-v2/writing-generation-mode-store.test.mjs` | **新建**（vitest，6 个用例） | 核心回归护栏；用例见 §4 |
| `frontend/tests/stores/writing.test.mjs` | 新增 1 个 node:test 用例 `retired big_scene in localStorage is not accepted as the current mode (W-31)` | 让 `npm test`（node --test）这一路门禁也能独立覆盖同一不变量 |
| `frontend/tests/composables/useWritingScreenAdapter.test.mjs` | 原 `:70-71` 断言"选中 `big_scene` ⇒ `allowPartialReroll === true`"，改为断言**选中被拒绝**（档位仍是 `sequential_crew`、`allowPartialReroll` 仍为 `false`） | 该断言原先**固化了缺陷行为**（它证明前端能被切进不可见昂贵模式）。这是本轮唯一一处"改既有断言"，属必要的口径更正，不是为了让测试变绿而放宽 |

**未改动（有意）**：
- 后端 `crates/domain/src/generation.rs` / `crates/app-pipeline` 的 `BigScene` 兼容路径 —— 兼容面必须保留（非 Campaign 旧写作与 reroll 仍可能走到它；`03-writing-pipeline.md:755`）。
- `frontend/src/utils/rerollPolicy.js` —— 纯策略函数，判据是"上一次产物记录的 `recordedMode`"，语义未变。
- `frontend/src/adapter/useWritingScreenAdapter.js:89` —— **不在本任务写作用域**。它现在对 campaign 会话恒为 `false`（`generationMode` 再也回不到 `big_scene`），整段 `|| writing.generationMode === 'big_scene'` 变成死分支 ⇒ 见 §6 转交（N-R2-01 / task-32 的清理素材）。
- `docs/AGENT_INTERFACES.md:12` —— **无需改**：本方案让该行文案（"仅保留后端兼容路径"）从"宣称强于实际"变成**与前端一致的事实**（N-R2-09 的"二者取一"取了前端这一侧）。

## 4 测试与失败可控性证据

### 4.1 新增 vitest 用例（`tests/components-v2/writing-generation-mode-store.test.mjs`）

| 用例 | 断言要点 | 修复前会怎样 |
| --- | --- | --- |
| 目录只有三档，且不含 `big_scene` | `catalog.map(v => v.value)` 深等于三档 | 通过（目录本来就没变） |
| 旧 localStorage 里的 `big_scene` 不再被接受为当前档位 | 解析结果 === `'continuation'`；同批 `duet` 正常；原始 blob **逐字节未变** | ❌ 失败（会是 `'big_scene'`） |
| `setGenerationMode` 拒绝 `big_scene` / 未知值，合法三档正常 | 拒绝后仍是 `continuation`，且**没有任何写入**（`getItem === null`）；三档逐个可设且落盘 | ❌ 失败（旧值会写入并成为档位） |
| 用户重新选择档位后 blob 里不再残留 | 选 `duet` 后 blob === `{ 'campaign-a': 'duet' }` | 通过（非回归断言） |
| 合法档位跨 store 重建仍按 Campaign 记忆 | 重建后 `sequential_crew` 保持；另一个 Campaign 的陈旧 `big_scene` 回退 `continuation` | ❌ 失败 |
| 陈旧 `big_scene` 不会让局部 reroll 落到 legacy 昂贵路径 | 同式判定 `allowPartialReroll === false`，`rerollPolicy('continuation','big_scene',false)` → `{editorOnly:false, sequentialSuffix:false}`；并给出对照 `allowLegacyPartial=true` 时 `editorOnly===true` | ❌ 失败 |

### 4.2 失败可控性（变异测试，实做）

把 `writing.js` **临时改回**旧实现（字面量集合含 `big_scene`）后：

- 等价的 node:test 复算（§4.3）：**6 用例 → pass 1 / fail 5**；
- 仓库 `tests/stores/writing.test.mjs`：**19 → pass 18 / fail 1**（新增用例）；
- 仓库 `tests/composables/useWritingScreenAdapter.test.mjs`：`allowPartialReroll` 断言失败（旧断言与新行为冲突，正是本次被更正的条目）。

随后**已恢复**为派生实现并复跑通过（`git diff` 只含 §3 的 3 个跟踪文件 + 1 个新文件）。⇒ **测试非恒真、有明确失败控制点。**

### 4.3 我实际跑过的（沙箱能力内）

| 验证 | 命令 | 结果 |
| --- | --- | --- |
| 语法 | `node --check` × 4 个文件 | 全部 exit 0 |
| 仓库 node:test 目标文件（**真实运行**） | `node tests/stores/writing.test.mjs` | **19 tests / 19 pass / 0 fail** |
| | `node tests/generation-modes.test.mjs` | 2 / 2 pass |
| | `node tests/reroll-policy.test.mjs` | 2 / 2 pass |
| | `node tests/composables/useWritingScreenAdapter.test.mjs` | **7 / 7 pass**（更正断言后） |
| | `node tests/composables/useMessageVariants-receipt.test.mjs` | 3 / 3 pass |
| | `node tests/composables/useMessageVariants-guards.test.mjs` | 4 / 4 pass |
| vitest 用例等价复算（`%TEMP%` 临时文件，未入库；同一批断言用 node:test 重写） | `node %TEMP%\r10-store-replica.test.mjs` | **6 / 6 pass** |

> **口径**：上面的"等价复算"是我在无 vitest 权限下的替代手段，**不能当作 vitest 结论**。

### 4.4 我**跑不了**的（必须由 Lead 复跑）

| 门禁 | 原因 |
| --- | --- |
| `npm run test:ui`（vitest，**含本次新增的 6 个用例**） | 沙箱 `spawn EPERM` |
| `npm test`（node --test 全套 67 文件） | 同一限制：`node --test` 逐文件 spawn 子进程 → `Error: spawn EPERM`（我改为**逐文件直接运行**了受影响文件，见 §4.3） |
| `npm run build` | 同上 |

**⇒ 记录结论：`frontend/src/stores/writing.js` 的改动已由 node:test 目标文件与等价复算验证；vitest 侧（含新增用例所在文件）**需 Lead 复跑 `npm run test:ui` 确认**，我未声称已绿。**

## 5 回归面评估（会不会踩到别的路径）

- **Campaign 会话**：默认档位仍是 `continuation`；`duet`/`sequential_crew` 逐项复跑通过；陈旧 `big_scene` 现在回退默认（这正是修复目标）。
- **非 Campaign 会话**（legacy）：`generationMode` 仍返回 `'continuation'`（`writing.js:60-61` 未变），`useMessageVariants` 仍传 `generationMode: null`（`campaignStore.activeCampaign ? ... : null`），`allowPartialReroll` 仍为 `true`（第一个操作数短路）⇒ **legacy 局部重 roll 未受影响**，符合 CLAUDE.md 的"不得破坏非 Campaign 路径"。
- **reroll 后端契约**：请求参数与后端 `BigScene` 兼容解析均未改动；`rerollPolicy` 的 `recordedMode` 仍来自产物 provenance。
- **文案一致性**：`ComposerBar` 成本提示、`README.md:19`、`ARCHITECTURE.md:56`、`AGENT_INTERFACES.md:12` 现在与代码一致（本次不需要动文档）。

## 6 转交与残留

1. **`useWritingScreenAdapter.js:89` 已成死分支**（campaign 下 `generationMode === 'big_scene'` 不可达；非 campaign 下第一个操作数已短路）→ 归属 **task-32 / N-R2-01**，建议连 `allowPartialReroll` 的表述一起简化（删除后 `rerollPolicy` 的 `allowLegacyPartial` 仍被 legacy 路径传入 `true`，函数本身不能删）。
2. **`08-docs-sync-fixes.md:123` 的虚假归属**（称"域5 记录称其已在前端侧修复"）不在本任务交付范围 → R2 已登记，需文档任务或 Lead 裁定。
3. **`docs/AGENT_INTERFACES.md:12`** 无需改动；若 Lead 更希望前端保留该档位，则需按方案 B 另立产品决策。
4. W-31 的 R6 状态可置为**已收口**；同批另外 3 条"无记录"项不在本报告范围。
