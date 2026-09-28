# R4 复检报告：域6（Meta/MVU/插件/Card Shell/导入导出）+ 域7（文档/脚本/证据）

- 复审人：**review-frontend**（task-21，第二遍审查）
- 日期：2026-09-13
- 复审对象：`fixes/06-meta-plugin-fixes.md`、`fixes/07-goals-docs-fixes.md`、`fixes/09-goals-scripts-fixes.md`、`fixes/GATE-REPORT.md`
- 代码基线：工作树（未提交，冻结中）；`HEAD = ab894c6`
- **约束遵守**：本报告是本次唯一写入（外加 Lead 另派的 task-33）；**未修改任何源码/文档/测试**。

## 0 方法与可复现性声明（先读这一条）

| 手段 | 说明 | 本次是否可用 |
| --- | --- | --- |
| `cargo test -p ... --lib -- <filter>` | 真实 Rust 测试 | **可用**（本报告 §3/§4/§5 的 Rust 证据均为实跑） |
| `node <tests/*.test.mjs>` | node:test 单进程 | **可用** |
| 单进程 vitest 等价 harness（`@vue/compiler-sfc` + happy-dom + `@vue/test-utils` + 自写 `vitest` shim；脚本在 `%TEMP%`，**未入库**） | 可挂载真实 `.vue`、派发 MessageEvent、跑断言；**不支持 `vi.mock`** | **可用**（M-24/M-31a 的独立复核就靠它） |
| Node WHATWG `URL` 复算 | 与 Rust `url` crate 同标准；照抄守卫逻辑逐条判定 | 可用（§4 明确标注"等价复算，非 Rust 运行时"） |
| `npm run test:ui`（vitest 本体） | 沙箱 `spawn EPERM` | **不可用**（沿用 Lead 门禁结论，不重复声称） |
| Pester | 见 §6.5：本机两套环境**都跑不出记录里那组数**，原因是 PowerShell 版本 / Pester 版本组合，非代码 | **不可复现** |

**本报告中任何"通过"都注明了度量工具；harness 与等价复算的结论都不得当作 vitest/Rust 门禁结论使用。**

---

## 1 门禁期三条新缺陷：修复是否真的闭合（Lead 指定重点）

### 1.1 M-24 握手令牌时序（插件桥整体失效）

**代码复核**（`frontend/src/components/PluginHost.vue`）：

| 不变量 | 实现位置 | 复核结论 |
| --- | --- | --- |
| 令牌**先于**文档组装生成 | `:269` `const token = newPluginHandshakeToken()` → `:270` `composeShellDoc(props.plugin, token)` | ✅ 顺序正确；`composeShellDoc(plugin, token)` 是纯函数（`:229-242`），令牌由参数注入 |
| watch 源与令牌无关 | `:246-250` `handshakeSourceKey = plugin.id + '\u0000' + entry_html` | ✅ 令牌不是响应式依赖 ⇒ 无自激重注册 |
| 每次源变化恰好注册一次 | `:262-304`；`registerShellDoc` 只在回调内调用 | ✅ 静态成立，且被独立实测（见下表 3 条） |
| 注册文档里的令牌 == 随后被接受的令牌 | `newPluginHandshakeToken()` 返回值即 `handshakeToken`，同时嵌进文档 | ✅ `isPluginBridgeHandshakeValid(handshakeToken, data.handshake)`（`:424`）比较的就是同一变量 |

**独立实测**（我自己的 harness 文件 `%TEMP%\r4-verify.test.mjs`，绕过被测仓库测试、用 `__TAURI_INTERNALS__.invoke` 桩 + 真实 `@tauri-apps/api` 路径）：

| 用例 | 断言 | 结果 |
| --- | --- | --- |
| 初挂载恰好注册一次 + 注册文档带当前令牌 + 只有该令牌被接受 | `registerDocCalls.length === 1`；`handshake` 正则命中；空令牌/`sfh_forged` 不放行；正确令牌放行并渲染槽内容 | ✅ |
| 源变化（改 `entry_html`） | 恰好再注册 1 次（总数 2）；新令牌 ≠ 旧令牌；旧令牌放行失败、新令牌放行成功 | ✅ |
| 帧自导航（额外 `load`） | 已信任 → 额外 load → 重放旧令牌 → 槽位不出现 | ✅ |
| 时序快照 | 注册文档正文包含 `handshake: "<token>"`，且该 token 是随后唯一被接受的值 | ✅ |

**结论：M-24 的两条不变量（"每次源变化恰好注册一次"、注册=被接受）在逻辑层与单进程 DOM 层均已闭合。** 测试文件本身也把两条不变量写成了硬断言（`tests/components-v2/plugin-host-handshake.test.mjs:190-232`：`:194` 断言 1 次、`:201` 断言 2 次、`:204` 断言令牌轮换、`:209-229` 断言旧令牌拒绝 / 新令牌放行），**断言不是恒真**。

**残余（低，不构成缺陷）**：`onIframeLoad` 用 `expectedFrameLoad` 消费"宿主自己那次 load"（`:317-325`）。若 `registerShellDoc` 返回**与上一次完全相同**的 URL，`:src` 不变 ⇒ 不产生 load ⇒ 标记残留；此后第一次自导航 load 会被误当作"自己的 load"。实测路径上 URL 由后端一次性 token 组成（`utils/shellDocUrl.js` `TOKEN_RE = /^[a-f0-9]{64}$/`，每次 `card_shell_register_doc` 生成新 token ⇒ URL 必变），**因此当前不可达**，仅作为实现细节记录。

### 1.2 `expectedFrameLoad`：自导航后旧令牌不可重放

- 实现：`:320-323` "宿主 load ⇒ 消费标记、保留令牌"；`:324` "额外 load ⇒ `newPluginHandshakeToken()` 轮换"。同时 `:319` 无条件 `handshakeValid.value = false`，`:318` `iframeReady=false`。
- 我独立复现的三步链路（自导航后重放旧令牌）✅ 通过（§1.1 第 3 行）。
- **语义边界（写在记录里的口径我认同）**：令牌是"当前文档新鲜度"锚，不是插件身份认证；被导航走的新文档若与壳同源且能读到旧令牌，本机制不新增信任（它本就在同一个 `sandbox="allow-scripts"` 空源帧里）。真正被挡的是**拿不到令牌的文档**（跨源页、外部窗口、伪造 `event.source`）。
- 复核 Lead 记录 §15.2 的措辞"自导航后的新文档永不取得信任"：严格说是"**永不取得信任，直到宿主再次为它注册带新令牌的文档**"——`watch` 再次触发（源变化）后新文档会有新令牌并正常握手，这是**期望行为**。建议把 §15.2 的"永不"改为"本轮注册周期内永不"，避免被读成"该帧永久拉黑"。（N-R4-07，P3）

### 1.3 M-31a `<style>` 正文残留（`stripStyleElementsFromSlotHtml`）

**代码复核**（`PluginHost.vue:90-112`，`:439-442` 在 sanitize 之前调用）：

| 不变量 | 实测（harness / 我的探针） | 结论 |
| --- | --- | --- |
| 完整 `<style>…</style>` 整块删除（含正文） | `strip('<style>body{display:none}</style><button class="sf-x" style="color:red">ok</button>')` → `'<button class="sf-x" style="color:red">ok</button>'` | ✅ 精确相等 |
| **无 `<style` 时逐字节不变** | `strip('<button class="sf-x" style="color:red">ok</button>')` → 原样 | ✅ 逐字节相同 |
| 大小写 / 属性 / 多个块 | `<STYLE media="x">a</STYLE >` → `''`；`a<style>b</style>c<style>d</style>e` → `'ace'` | ✅ |
| 非目标字符串不被误伤 | `<stylefoo>x</stylefoo>` 原样；`undefined/null` → `''` | ✅ |
| sanitize 之后 DOM 里无 `<style>`、无 `display:none` 文本 | 我的实测：`sanitize(strip(输入), {FORBID_TAGS:['style'],FORBID_CONTENTS:['style']})` → `'ok'` | ✅ |

**结论：闭合**（完整块 + 常见畸形形态都被拦下，且"无 style ⇒ 零改写"这条不变量成立）。

**残余缺陷（N-R4-02，P3，真实浏览器才有）**：**未闭合/自闭合的 `<style` 不在删除范围**：
- `strip('<style/>body{display:none}')` → 原样（正则要求成套 `</style>`）；
- `strip('<style>body{display:none}')` → 原样。
- 这两条在 **happy-dom** 下 sanitize 返回 `''`（看着安全，见 §2 的环境事实），但在**真实浏览器**里 `<style/>`/未闭合 `<style>` 会把后续内容当 CSS 文本，DOMPurify 的 `KEEP_CONTENT` 语义照样把正文留成裸文本 ⇒ 正是本次要消灭的"宿主界面污染"。
- 建议（不要求本轮改，代码冻结）：把预处理改为"`<style…>` 起到 `</style>`（若无则到串尾）整段删除"，或在记录里显式登记该残余。

---

## 2 域6 记录 §14/§15 措辞与事实的逐条核对

| 记录声明 | 我的独立核对 | 判定 |
| --- | --- | --- |
| §14.1 旧实现"注册文档永远带上一轮令牌、握手永不成功、自激重注册" | 现实现已按"先生成→再组装→再注册"重构；不变量实测通过（§1.1） | ✅ 与代码一致 |
| §14.2 `TavernHelper无脚本重新执行` 是选错标记（假阳性） | 记录主动自我更正、改用"出站回复 `__sf_th_bridge_res`"作受理副产物；我**无法**运行该 vitest 文件（其 `vi.mock`/iframe 语义超出 harness 能力），仅确认逻辑描述与代码注释自洽 | ⚠️ 未独立验证（能力边界，非"否定"） |
| §15.1 "31/31 文件通过，exit 0"，日志 `artifacts/review-2026-09-13-round2/fe-vitest.log` | 我未运行 vitest；Lead 门禁报告同口径；日志文件存在 | ⚠️ 采信 Lead 门禁（我无独立证据） |
| §15.2 `expectedFrameLoad` 收口修复 | 实测通过（§1.2）；仅"永不取得信任"措辞建议收紧（N-R4-07） | ✅（措辞 P3） |
| §15.2 `FORBID_CONTENTS:['style']` 在 3.4.12+happy-dom 下无效 | **我的实测完全一致**：`sanitize('<style>body{display:none}</style><button>ok</button>', {FORBID_TAGS:['style'],FORBID_CONTENTS:['style']})` → `body{display:none}<button>ok</button>`（CSS 正文残留）；单 `FORBID_TAGS` 同样残留 | ✅ 事实正确 |
| §15.3 **"happy-dom 下 DOMPurify 3.4.12 的默认白名单解析为空"** | **部分不成立/表述过宽**。我实测（`dompurify@3.4.12`，repo 内依赖）：`isSupported === true`；`sanitize('<p>hi</p>') === 'hi'`；`sanitize('<button …>ok</button>') === 'ok'`（这两条与记录一致）**但**：`sanitize('<div><p>hi</p></div>') === '<p>hi</p>'`、`sanitize('<div><span>a</span><b>b</b></div>') === '<span>a</span><b>b</b>'`、`sanitize('<button>ok</button><style>x{}</style>') === 'ok<style>x{}</style>'`（**`<style>` 元素本身被保留**）。⇒ 白名单不是"空"，而是**随输入变化的半初始化状态** | ⚠️ **N-R4-01，P3**（结论方向不变，但推论 2 的"任何元素保留断言都不可靠"应改为"输入相关、不可预测"） |
| §15.3 推论 1：M-31a 用例"`<style>` 不存在"部分靠环境成立，真正钉住行为的是纯函数断言 | 与我实测一致（条目 1）；且**补充一条**：控件组"行内 `style` 属性保留"在 happy-dom 下确实不可断言（我实测 `sanitize('<button class="sf-x" style="color:red">ok</button>') === 'ok'`，属性与元素一起没了） | ✅ |
| §15.3 推论 3："早期单进程 harness 下看起来通过却掩盖了 CSS 正文残留" | 合理且与我的实测相符（单看 DOM 断言在 happy-dom 下可能因环境副作用为真） | ✅ |

**建议的措辞修正（供 Lead 决定，我不改文档）**：把"默认白名单解析为空"改成"默认白名单在 happy-dom 下**未可靠初始化**：简单文本块被去标签，但嵌套块与含 `<style>` 的输入会被部分保留——**不要基于 happy-dom 断言元素/属性是否保留**"。

---

## 3 M-01（P0-2）：子帧可达 Tauri IPC —— **仍然开放**

**记录 §3 的划分是诚实的**：标题即"暂缓（主体）"，正文写明"本轮**没有**把子帧可达 IPC 这件事关掉，任何'已修复'的表述都是假的"，并单列"仍需 Windows 运行时 PoC"。

我独立核对四条收敛项：

| # | 记录声明 | 我的核对 | 判定 |
| --- | --- | --- | --- |
| 1 | `SHELL_DOC_CSP.connect-src` 增加 `ipc: http://ipc.localhost`（卫生项，非修复） | `crates/tauri-app/src/shell_doc_protocol.rs:52-87` 实测含 `"ipc: http://ipc.localhost "`（`:77`）；注释显式写"不构成 M-01 的修复" | ✅ 与声明一致 |
| 2 | 4 条静态守卫测试存在且生效 | 存在（`:345`,`:362`,`:381`,`:409`）；**我实跑**：`cargo test -p storyforge --lib -- shell_doc_protocol card_shell_cache` → **30 passed / 0 failed**，其中含 `shell_csp_keeps_tauri_ipc_sources_for_custom_protocol_fetch`、`shell_csp_stays_locked_down`、`capability_grants_only_the_main_window`、`acl_manifest_absence_is_a_known_risk` 全 ok | ✅ |
| 3 | guard 2 断言"无 `https://`、无通配符" | 实跑通过；我另核 CSP 常量确实不含 `*` 与 `https://`，且 `default-src 'none'` / `object-src 'none'` / `form-action 'none'` 齐备 | ✅ |
| 4 | `capabilities/default.json` 只绑 `main`、且仓库无 `permissions/` | `crates/tauri-app/capabilities/default.json`：`"windows": ["main"]`，无 `webviews`/`remote` ✅；`crates/tauri-app/permissions/` **不存在** ✅ ⇒ `has_app_acl_manifest == false` 的前置成立 | ✅ |
| 5 | "新增 app ACL manifest"方案被否决、且不提交 | 仓库确无 manifest；记录给出 `authority.rs:132-134` / `webview/mod.rs:1823` 依据 | ✅ 与我读到的 Tauri 判定链一致（未逐行复核上游源码，属**来源采信**） |

**我的独立结论**：
1. **M-01 仍开放（P0）**，等级不变。已落地的 4 项都是"爆炸半径收敛 + 禁止盲改"，**没有任何一项把子帧的 IPC 可达性关掉**。
2. 记录对"需要运行时 PoC"的标注**是诚实且必要的**：PoC 步骤（插件 iframe 内 `typeof window.__TAURI_INTERNALS__` / `invoke('list_conversations')` 对照主窗）写得可执行；`wry 0.55.1` WebView2 丢弃 `for_main_frame_only` 的源码依据方向正确，但**本环境（含我这次）无法伪证**。
3. 记录给出的三条真修复方向（升级/回移 wry；独立 webview 承载不可信 HTML；命令层主帧凭据）与源码事实一致，第 3 条与我 F-30/R4 对"23 条保留命令"的观察互为补充：**在 M-01 关闭前，这 23 条前端不可达但已注册的命令是同一攻击面的一部分**（域4 的 N-02 已按此口径登记）。
4. 我**不**把 §3 的任何一条记为"M-01 已修"。

---

## 4 M-02（白名单 + IP 字面量绕过）：**不可绕过（已闭合）**

**实现**（`crates/tauri-app/src/card_shell_cache.rs`）：
- 单一解析器：`parse_http_url`（`:453-469`）——`url::Url::parse` → 仅 `http/https` → **拒绝 userinfo**（`username` 非空或 `password` 存在）→ **拒绝显式非默认端口**（`:80/:443` 被 URL 标准归一化后不触发）。
- IP 守卫：`validate_fetch_url`（`:148-162`）——`host_str()` 含 `[` 或去括号后可 `parse::<IpAddr>()` ⇒ 拒绝。注释说明 WHATWG 已把十进制/十六进制/八进制 IPv4 折叠成点分形式（`:154-157`）。
- allowlist：`is_url_allowed`（`:129-143`）小写比较（大小写不可绕过）；**重定向目标同样过 `validate_fetch_url`**（`:145-147` 注释 + 用例）。

**实跑证据**：
- `cargo test -p storyforge --lib -- card_shell_cache`（含在 §3 的 30 passed 内）→ `authority_smuggling_cannot_bypass_allowlist_or_ip_guard` **ok**、`rejects_ip_literal_redirect_targets_and_limits_unknown_length_bodies` **ok**、`host_parser` **ok**。

**我自己构造的 37 组绕过用例**（Node WHATWG `URL` + 照抄上述守卫逻辑的等价复算；**非 Rust 运行时**，故与实跑测试互补）：

| 类别 | 样例 | 结果 |
| --- | --- | --- |
| 反斜杠 authority 走私（原始缺陷） | `https://evil.example\@cdn.jsdelivr.net/x` | 拒绝（实际 host `evil.example`） |
| userinfo 伪装 | `https://cdn.jsdelivr.net@evil.example/x`、`http://user:pw@cdn.jsdelivr.net/x`、`http://cdn.jsdelivr.net%2f@evil.example/x` | 拒绝（credentials 规则） |
| IP 字面量家族 | `127.0.0.1`、`[::1]`、`[::ffff:127.0.0.1]`、`2130706433`、`0x7f.0.0.1`、`0177.0.0.1`、`127.1`、`0`、`169.254.169.254`（云元数据） | 全部拒绝 |
| IPv6 zone id | `https://[fe80::1%25eth0]/x` | 拒绝（解析失败 ⇒ fail closed） |
| 端口 | `https://cdn.jsdelivr.net:8443/x` | 拒绝；`:443`/`:80` 归一化后放行（同一主机，正确） |
| 制表/换行走私 | `http://evil.example\t@cdn.jsdelivr.net/x`、`http://cdn.jsdelivr.net\t.evil.example/x`、`http://cdn.jsdelivr.net\\\tx/x` | 前两条拒绝；第三条实际 host 仍是 `cdn.jsdelivr.net`（`\`→`/`、`\t` 被移除），属**正确放行** |
| 片段/查询伪装 | `http://cdn.jsdelivr.net#@evil.example/x`、`...?@evil.example/x` | 实际 host = allowlist 主机（正确放行） |
| 其他 SSRF 面 | `http://localhost/x`、`http://metadata.google.internal/x`、`javascript:`/`file:`/`data:` | 全部拒绝（allowlist / scheme） |
| 子域不继承 | `https://sub.cdn.jsdelivr.net/x`、`https://cdn.jsdelivr.net./x`（尾点） | 拒绝（fail closed，非白名单精确匹配） |

**结论**：**0 组绕过**（37/37 判定与守卫语义一致）。原始 `\@` 走私与 IP 字面量两类都闭合。

**建议（不阻塞）**：把下列 5 条补进 `authority_smuggling_cannot_bypass_allowlist_or_ip_guard` 的用例列表（成本极低、防未来回归）：`https://[::ffff:127.0.0.1]/x`、`https://127.1/x`、`https://0/x`、`http://169.254.169.254/latest/meta-data/`、`https://cdn.jsdelivr.net./x`。

---

## 5 M-03 / M-04 / M-08：逐条实跑核对

| ID | 记录声明 | 我的实跑 / 核对 | 判定 |
| --- | --- | --- | --- |
| M-03 跨 Campaign 写入被拒 | `TypedPatch.campaign_id/revision` + `stamp_patch_scope`；preview 绑不符 ⇒ `Stale`；accept 先 `mark_typed_patch_stale` 再报错（**写盘前**） | `cargo test -p storyforge --lib -- test_meta_typed_patch_is_bound_to_its_campaign` → **1 passed / 0 failed**；代码侧 `crates/app-meta/src/typed_patch.rs:81-91`（字段）、`:347+`（盖章）存在；`None` fail-closed 的口径在 `patch_campaign_binding_ok` 注释中一致 | ✅ 已修复 |
| M-04 `__storyforge*` 保留命名空间 | 前端 `utils/mvuExecuteResult.js` 丢弃保留键；Rust `app-pipeline::push_mvu_js_variable_updates` 同口径 | 前端 `node frontend/tests/mvu-execute-result.test.mjs` → **3 pass / 0 fail**；Rust `cargo test -p storyforge-app-pipeline --lib -- mvu_js_fallback_drops_reserved_namespace_keys` → **1 passed / 0 failed** | ✅ 已修复(降级：作用域仍一律 campaign，见 M-18) |
| M-08 prompt hook 需 plugin_id + 权限 | `plugin_prompt_hook_result` 可选 `plugin_id`/`modifier_plugin_ids`；未声明/未注册/无 `ModifyPrompt`/已禁用 ⇒ 丢弃 messages 且仍结算 pending | `cargo test -p storyforge --lib -- prompt_hook_mutation_requires_a_permissioned_modifier_plugin` → **1 passed / 0 failed**；代码 `crates/tauri-app/src/commands/plugins.rs:255-335`（`ensure_permission(..., Permission::ModifyPrompt)`、`:325` warn、`:331` 未知 request_id） | ✅ 已修复(降级) |
| M-08 的降级点声明是否诚实 | "`request_id` 未与签发时的插件集合绑定 ⇒ 持 `ModifyPrompt` 的插件 A 仍可抢占 B 的 `request_id`" | 代码未做 request_id ↔ 插件集合绑定（`plugins.rs` 只按传入 id 复核），记录已显式登记为已知残余 | ✅ 声明诚实 |
| M-04 的"前端不是安全边界"口径 | 命令层 T-06 守卫在 `crates/tauri-app/src/commands/variables.rs:172+`（`__storyforge*` 拒绝），但 accept 路径不经该守卫 | 与记录一致；两处口径（前端丢弃 + Rust 丢弃）均已落地 | ✅ |

---

## 6 域7：声明真实性 + 门禁有效性

### 6.1 `RETAINED_NO_FRONTEND_CALLER` 23 条

- 我实跑 `node scripts/architecture/backend-baseline.mjs` → **exit 0**，末行 `[backend-baseline] 门禁通过`，统计行：
  `定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿命令 23`，并**逐个列出** 23 个命令名。
- 逐个与我 F-30 的清单比对：`abandon_turn`、`archive_conversation`、`soft_delete_variant`（既有 3 条）+ 我删除 wrapper 的 20 条（含 `card_shell_allow_host`、`delete_character`、`get_active_agent_profile_config` 等）——**完全吻合，无第 24 条**。
- `cd frontend && node tests/tauri-command-contract.test.mjs` → **`pass 11 / fail 0`**（Gate 0 的"注册集 = 前端可达集 ∪ 声明保留集"成立）。
- **真实性**：23 条声明不是"文档里写着而代码不存在"——它们是同一份 `commandAttributes`/注册表推导出来的，且我核过 `delete_character` 的依据链（`CLAUDE.md` 级联语义 ← 域4 已引用）。
- **是否削弱门禁**：没有。这是把**静默**变成**显式声明 + 违规 exit 1**；域4 保留了"是否连删（175→155）"的产品决策权，并把这个未关闭的安全面（N-02）单列。

### 6.2 untracked 密钥扫描规则收窄（09 §4）

**逐条核对实现**（`scripts/release-build/ReleaseBuild.Common.ps1`）：

| 记录声明 | 代码事实 | 判定 |
| --- | --- | --- |
| 新增开关 `-SkipUnquotedAssignment`，仅跳"无引号赋值"一条规则 | `:519` 参数、`:562-563` `if ($SkipUnquotedAssignment -and $rule.Name -eq 'unquoted secret assignment') { continue }` | ✅ 只跳这一条 |
| 其余规则对所有文件照旧 | `:526-559` 共 9 条规则：私钥块、`AKIA`、`sk-`（长/短）、Slack、`Authorization/X-Api-Key`、裸 `Bearer`、**带引号** secret assignment、无引号 assignment（被跳过的那条） ⇒ 8 条仍在 | ✅ |
| 单一事实源 + tracked 扫描改为派生（顺带补 `.psd1`） | `:486-491` 扩展名表；`:756-758` `$configOnlyPaths` 由该表派生 | ✅ 消除了"两处手工同步" |
| `.env` 等无扩展名 dotfile 仍算配置文件 | `:493-505`：无扩展名 ⇒ `fileName.StartsWith('.')` | ✅ |
| untracked 分支按路径传开关；evidence roots 分支同样处理 | `:856-857`（相对路径）、`:905-906`（`$file.Name`） | ✅（`$file.Name` 只取扩展名，等价） |
| 防回退断言 | Pester `ReleaseBuild.CI.Tests.ps1:612-615` 四条 `Should Be`（`.env`/`.ps1` = true、`.md`/`.rs` = false）**逐条与我读到的一致** | ✅ |
| fail-closed 语义未变 | 读取失败/超 2 MiB/路径消失仍 throw（记录声明；我未逐行复核所有分支） | ⚠️ 采信 + 部分核对 |

**门禁有效性判断**：**未发现削弱**。收窄的只是"非配置/脚本文件里、形如 `key: 长串` 的无引号赋值"这一条**低置信度**规则；真实密钥最常落点（`.env`/`*.json`/`*.ps1`/`*.yml`…）保持全规则，高置信度形态（`sk-`/`AKIA`/私钥/Bearer/authorization/带引号凭据）在**所有**文件上仍然拦。记录 §4.5 也主动写出残余与回退条件（"回退与保留评审产物不可兼得"），属**诚实收窄**。
**唯一建议**：把 §4.5 第 1 条的残余风险同步进 `GATE-REPORT`/门禁脚本的自述（现在只在域7 记录里），这样下一位维护者不会误以为"untracked 扫描 = 全规则"。

### 6.3 G-12 校验和覆盖方向

| 记录声明（09 §5） | 我的独立核对 | 判定 |
| --- | --- | --- |
| 同名冲突成立，实际留存的是 **Windows** 那份、缺的是 Android（后补传） | `git log -1 --format=%B ab894c6` 原文：`… so the release upload no longer silently overwrites one platform's checksums with the other's (v0.1.1/v0.1.2 both shipped only the Windows sums; v0.1.2 got SHA256SUMS-android.txt uploaded manually as remediation)` | ✅ 与结论一致（库内权威陈述） |
| 补救动作指向 Android | 同上引文；且 `README.md:42` 现文案"…（APK 哈希，**发布后补传**）" | ✅ |
| release.yml 已不再用裸名 | `.github/workflows/release.yml:97` `SHA256SUMS-windows.txt`、`:213` `SHA256SUMS-android.txt` | ✅ |
| 196 B / 88 B 的字节算术 | 我未取回 Release 原文（离线环境），仅复核算术自洽（CRLF 两行 98×2=196；`sha256sum` 单行 88） | ⚠️ 未独立复算（记录方法可复现，采信） |
| **行号引用** | 记录 §5.1/§5.2 写 "`release.yml:90-94`"（实际 **L97**）、":204-205"（实际 **L213**）、§5 注 "`release.yml:247`"（实际 **L258**） | ⚠️ **N-R4-04（P3）行号漂移，结论不受影响** |

### 6.4 RELEASE-STATUS / REGRESSION-COVERAGE 的"行号重算"

| 抽查项 | 结果 |
| --- | --- |
| `docs/RELEASE-STATUS.md:34`（G-01：HEAD=ab894c6 + 三轮门禁 + 未提交修复的表述） | ✅ 实际 L34 正是"停止位置：…仓库 HEAD 为 `ab894c6`…" |
| `docs/RELEASE-STATUS.md:90`（G-12 定案文案） | ✅ 实际 L90 正是"…**2026-09-13 复核定案：留存的是 Windows 那一份，缺的是 Android…**" |
| `RELEASE-STATUS:36/93`（三轮门禁 + "后两轮未写入退出码行"） | ✅ 两处都存在，且**显式写明口径纪律**（不把"日志以 `Release gate passed.` 结束"写成"退出码 0"）——这是本轮最值得肯定的诚实点之一 |
| `docs/REGRESSION-COVERAGE.md`（task-28 重算） | ✅ L5 自述"原有 73 处里 55 处已漂移，现已全部改为真实 `fn` 行号"；L12 已消除旧矛盾（改为"行号会漂移、以函数名检索为准"）。**逐点验证样例**：L22 写 `crates/domain/src/campaign.rs:798` → 我实查 `campaign.rs:798` = `fn resolved_persona_override_takes_priority()` ✅ 精确命中；L23/24/25/26/27/28（810/823/833/845/857/867）与 `:35-43`（894/912/926/938/724/746/755/769/880）同批重算值格式一致 |
| `REGRESSION-COVERAGE` 旧建议值 vs 现在 | 09 §7.2 曾建议 `campaign.rs:389 → :719`；**HEAD 实际** `resolved_persona` 本体在 **:399**、`resolved_behavior` 在 **:409**、测试在 798+ ⇒ task-28 按 HEAD 重算是对的，09 §7.2 的建议值已过期（记录本身已标"转交"，不算错误，但读者需知道它已过时 → N-R4-06 同族） |

### 6.5 Pester：**我无法复现"198 全绿"，且发现门禁对 PowerShell 版本敏感**（N-R4-05，P2）

Lead 允许我复跑 Pester（只跑一次并贴计数），我实际做了三件事（后两次为同一问题的最小诊断）：

| # | 命令 | 结果 | 归因 |
| --- | --- | --- | --- |
| A | `powershell -NoProfile -ExecutionPolicy Bypass -Command "Invoke-Pester scripts/tests"` | **Tests Passed: 0, Failed: 198, Container failed: 4**；栈顶 `New-RandomTempRegistry` → `RegistryWrapper.OpenSubKey` 访问拒绝 | 该写法**绕过了 wrapper 的版本选择**：PS 5.1 解析到用户级 **Pester 5.7.1**（`G:\document\WindowsPowerShell\Modules`），5.x 初始化测试注册表驱动器被本会话沙箱拒绝 ⇒ **不是代码失败**（也说明不能用裸 `Invoke-Pester` 复现门禁） |
| B | `pwsh -NoProfile -File scripts/tests/run-release-build-tests.ps1`（Lead 给入口，**PS 7**） | 第 1 套 `ReleaseBuild.Tests.ps1`：**Passed 43 / Failed 8**，wrapper fail-fast 退出码 1 | wrapper 正确选中 **Pester 3.4.0**（其内会打印 `Using Pester 3.4.0`），但 **Pester 3.4.0 的 `Should Throw` 在 PowerShell 7 下语义失效** ⇒ 8 条"期望抛错但未抛"（`fails when a required tool is missing`、`fails closed on non-zero process exit code`、`rejects zero executed/failed/skipped/pending/inconclusive results` 等） |
| C | Lead 门禁日志 `artifacts/review-2026-09-13-round2/pester.log` | 头部 `Using Pester 3.4.0`；末套 `Passed: 99 Failed: 0`，末行 `Release build tests passed.` | Lead 用的是 **Windows PowerShell 5.1 + Pester 3.4.0**（GATE-REPORT §6 的命令形态），与记录 09 §3 的口径一致 |

**结论**：
1. **我的复跑不能作为"198 绿"的独立证据**，也**不能**用我这 8 条失败去否定它——两边是不同 PowerShell 版本下的不同结果。
2. 但这是一条**真实的门禁脆弱性**：wrapper `run-release-build-tests.ps1:77-89` 只校验 **Pester 3.x/4.x**（注释说明 Pester 5 拒绝 `Should Be` 语法），**没有校验 PowerShell edition**；在 PS 7 下它会"选择到正确版本、却给出 8 条假红"。任何用 `pwsh` 跑门禁的人（CI、别的成员）都会看到红。
3. **建议**：wrapper 增加 `if ($PSVersionTable.PSVersion.Major -ne 5) { throw "run this suite under Windows PowerShell 5.1 (Pester 3.4.0)" }`，并在 `GATE-REPORT` 的复现命令处注明 `powershell`（5.1）而非 `pwsh`。这样"Pester 198/0"才是**可复现**的声明。

---

## 7 文档复检：`README.md` / `RELEASE-STATUS.md` / `ARCHITECTURE.md`

**结论：未发现"把未验证的东西写成已验证"的实质性夸大。**抽查证据：

| 文档 | 抽查点 | 事实 | 判定 |
| --- | --- | --- | --- |
| `README.md:25` | "11 步门禁已三轮全绿且计数完全一致（…前端 504+119+9+28+2=662…）" + 四项"已闭合" + 明确的"仍未闭合"清单（EXE 代码签名、任意历史回滚、ST 99 事件全集…） | 与 `RELEASE-STATUS:20-32` 一致；且保留"调试包或浏览器测试不能替代真机与真实产物验证" | ✅ 诚实 |
| `README.md:41` | Android 资产"`app-arm64-release.apk`（匹配模式 `*arm64-release.apk`）" | 与 07 §3 的 GitHub API 资产名事实一致（我未查 API，采信+自洽；workflow 侧 `*arm64-release.apk` 见 T-10） | ✅ |
| `README.md:42` | 校验和现状（v0.1.2 资产是 `SHA256SUMS.txt` + `-android`；平台命名自下一版本生效） | 与 `ab894c6` 提交说明、`release.yml:97/213` 一致 | ✅ 未夸大 |
| `RELEASE-STATUS:21` | Pester 198 项 + 工作流合同"读取的是**已停用**的 `.gitea/workflows/*`，不等于 GitHub 侧存在门禁" | `.gitea/workflows/` 三份文件仍在库中；`.github/workflows/release.yml` 全文无 `verify-release`/`cargo test`/clippy/fmt/Pester 调用（我核了关键行） | ✅ 范围说明是**主动降低**承诺，不是夸大 |
| `RELEASE-STATUS:34/36/93` | 三轮门禁 + "第二/三轮日志未写入退出码行" | 我核 L34/L36/L93 原文一致；`artifacts/release-gate-2026-09-06*/` 三个目录存在（记录 §G-11 的依据） | ✅ 口径纪律明确 |
| `ARCHITECTURE.md:3/180/185/192/198` | 内容（插件验收覆盖、Gitea 停用改 GitHub-hosted、APK 签名缺 `apksigner` 证据、技术债改写） | **内容全部存在且与记录一致** | ✅ |
| `07 §5.1` 的 `ARCHITECTURE.md` **行号** | 记录写 `:169`（插件仍需 GUI 验收）、`:174`（Windows runner 未验证）、`:180-181`、`:187`、`:188`（技术债） | 实际：`:169` 是 JSON 存储段落（无关内容）、`:174` 是空行；所述内容分别在 **L180 / L185 / L192 / L198（技术债节）** | ⚠️ **N-R4-03（P3）**：6 处行号全部漂移，其中 2 处指向无关内容 |
| `GATE-REPORT §4` | 单列 6 项"未验证"（CI 改动未真实运行、真 Chromium 行为、签名、真模型用例、未提交工作树…） | 与我本次能观察到的边界一致（我的 harness 也明确不是真 Chromium） | ✅ 本章是"防止后人误判"的关键，写得到位 |

---

## 8 新发现（N-R4 系列）

| ID | 严重度 | 位置 | 现象 | 建议 |
| --- | --- | --- | --- | --- |
| **N-R4-01** | P3 | `fixes/06-meta-plugin-fixes.md:347-352`、`GATE-REPORT:66`、`plugin-host-handshake.test.mjs:366-372` | "happy-dom 下 DOMPurify 默认白名单解析为**空**"表述过宽。实测 `dompurify@3.4.12`：`isSupported=true`；`<p>hi</p>`→`hi`、`<button>ok</button>`→`ok`，**但** `<div><p>hi</p></div>`→`<p>hi</p>`、`<div><span>a</span><b>b</b></div>`→`<span>a</span><b>b</b>`、`<button>ok</button><style>x{}</style>`→`ok<style>x{}</style>`（style 元素被保留） | 改为"白名单**未可靠初始化／随输入变化**，不可据此断言元素/属性保留"；M-31a 的结论与测试设计不受影响 |
| **N-R4-02** | P3 | `frontend/src/components/PluginHost.vue:90-112`（`stripStyleElementsFromSlotHtml`） | 只删**成套** `<style>…</style>`：`<style/>body{display:none}`、`<style>body{display:none}`（未闭合）原样通过；happy-dom 下 sanitize 恰好返回空（测不出），**真实浏览器**里 `KEEP_CONTENT` 仍会把 CSS 正文留成裸文本 ⇒ 本次要消灭的"宿主界面污染"在这两种形态下仍存在 | 预处理改为"从 `<style…>` 删到 `</style>`，无闭合则删到串尾"；或把该残余写进域6 记录的"已知残余" |
| **N-R4-03** | P3 | `fixes/07-goals-docs-fixes.md` §5.1（`ARCHITECTURE.md` 行号列） | 6 处行号全部漂移（`:169→L180`、`:174→L185`、`:180-181→L192`、`:187→L198`、`:188→L199`），其中 `:169`/`:174` 指向无关内容/空行 | 记录里改用"短语/函数名检索"锚点，或在文档头部固定一段"行号以 `rg` 检索为准" |
| **N-R4-04** | P3 | `fixes/09-goals-scripts-fixes.md` §5.1/§5.2/§5 注 | `release.yml` 行号漂移：`:90-94`→**L97**、`:204-205`→**L213**、`:247`→**L258** | 同上（结论 G-12 不受影响） |
| **N-R4-05** | **P2** | `scripts/tests/run-release-build-tests.ps1:77-89` | 门禁只校验 Pester 3.x/4.x，不校验 PowerShell edition：**PS 7 + Pester 3.4.0 ⇒ 第 1 套 43/8（8 条 `Should Throw` 假红）**；裸 `Invoke-Pester` 在 PS 5.1 下会解析到 Pester 5.7.1 ⇒ 全部容器初始化失败 | wrapper 断言 `$PSVersionTable.PSVersion.Major -eq 5`；`GATE-REPORT` 复现命令注明用 `powershell`(5.1)；若希望支持 PS 7，需把 `Should Throw` 语法升级为 Pester 5 兼容写法 |
| **N-R4-06** | P3 | `fixes/07-goals-docs-fixes.md` §8（G-12 行）vs `fixes/09-goals-scripts-fixes.md` §5.3 | 07 §8 陈述的是"**方向未能确认**"版本，09 §5.3 后来定案（Windows 留存）；最终文档是定案版，但 07 记录未加"已被 09 §5.3 取代"指引 ⇒ 单独读 07 会误判文档现状 | 07 §8 加一句交叉引用 |
| **N-R4-07** | P3 | `fixes/06-meta-plugin-fixes.md` §15.2 / `GATE-REPORT:63` | "自导航后的新文档**永不**取得信任"过强：源变化后宿主会为新文档注册带新令牌的文档，届时握手正常（期望行为） | 改为"本轮注册周期内永不；源变化重新注册后按新令牌正常握手" |
| **N-R4-08** | P3 | `frontend/src/components/PluginHost.vue:317-325` | `expectedFrameLoad` 仅在发生 `load` 时被消费；若 `registerShellDoc` 返回同一 URL（`src` 不变、无 load）标记会残留。当前后端每次返回新随机 token ⇒ **不可达** | 可加一条断言/注释："`registerShellDoc` 必须返回新 URL"（或改为按注册序号比较） |

**没有发现 P0/P1 级新缺陷。**

---

## 9 未能验证项（不得当成已验证）

1. **`npm run test:ui`（vitest 本体）**：沙箱 `spawn EPERM`；`plugin-host-handshake.test.mjs` 用 `vi.mock`，我的 harness 明确不支持 ⇒ **该文件我只有"读断言 + 独立等价用例"两层证据**，没有本体运行证据。
2. **Pester 198/0**：见 §6.5，本机两种 PowerShell/Pester 组合均不能复现；采信 Lead 门禁日志（`pester.log` 头部 `Using Pester 3.4.0`、末行 `Release build tests passed.`）。
3. **真 Chromium/WebView 行为**：`stripStyleElementsFromSlotHtml` 的未闭合分支（N-R4-02）、DOMPurify 白名单真实行为、CSP 在真实 WebView 下的执行——本轮全部只有 happy-dom 侧证据。
4. **`ab894c6` Release 资产（196 B/88 B、digest）**：离线无法取回；仅复核了提交说明与 workflow 侧命名。
5. **M-01 的运行时 PoC**：无 Windows/WebView2 运行环境，**维持"未验证、仍开放"**。
6. **09 §4 的"fail-closed 语义未变"**：我只核了规则集与开关路径，未逐行复核所有读取失败分支。

---

## 10 结论汇总

| 对象 | 结论 |
| --- | --- |
| M-24 握手令牌时序 | **闭合**（独立实测 4 项：注册次数/令牌一致/旧令牌失效/自导航重放拒绝） |
| `expectedFrameLoad` 重放防护 | **闭合**（措辞"永不"建议收紧，N-R4-07） |
| M-31a `<style>` 正文残留 | **闭合**（完整块 + 大小写 + 多块 + 无 `<style` 逐字节不变）；**残余**：未闭合/自闭合形态在真实浏览器仍泄漏文本（N-R4-02） |
| 域6 §14/§15 措辞 | 方向正确、自我更正诚实；**一处环境事实表述过宽**（N-R4-01） |
| M-01（P0-2 子帧可达 IPC） | **仍开放（P0）**；记录的"暂缓 + 无法伪证"划分诚实，4 项收敛经实跑确认（30 passed），**不得记为已修** |
| M-02 | **闭合**（Rust 测试实跑 + 我 37 组等价绕过复算 0 命中） |
| M-03 / M-04 / M-08 | **闭合**（3 个 Rust 目标测试实跑各 1/1 通过 + 前端 3/3）；M-04/M-08 的降级点声明诚实 |
| 域7 `RETAINED` 23 条 | **真实性确认**（baseline 门禁通过 + 孤儿 23 逐个列出 + 契约 11/11），**未削弱门禁** |
| 域7 密钥扫描收窄 | **未削弱**：仅停用 1/9 条低置信度规则于非配置/脚本文件；其余 8 条全文件生效；fail-closed 未改；残余风险记录在案 |
| G-12 方向 | **结论与库内权威陈述一致**；记录行号漂移（N-R4-04） |
| RELEASE-STATUS / REGRESSION-COVERAGE 行号重算 | **前者抽查全部命中**；**后者重算真实**（`campaign.rs:798` 逐点验证命中；自述 55/73 漂移） |
| ARCHITECTURE / README 回写 | **内容准确、未夸大**；ARCHITECTURE 行号引用漂移（N-R4-03） |
| Pester 门禁 | **本环境不可复现** + 发现 **PS edition 敏感**（N-R4-05，P2，建议 wrapper 加断言） |
| 新增发现 | 8 条：P2 ×1、P3 ×7；**无 P0/P1 级新缺陷** |

### 给 Lead 的三条优先建议

1. **N-R4-05（P2）**：给 Pester wrapper 加 PowerShell 版本断言，并在 `GATE-REPORT` 复现命令处注明 `powershell`(5.1) —— 否则"198/0"在 PS 7 环境下会变成 8 条假红，任何成员/CI 用 `pwsh` 复跑都会误判门禁红。
2. **N-R4-01 + N-R4-02（P3）**：把"happy-dom 白名单为空"改成"未可靠初始化"，并把 `<style` 未闭合形态登记为已知残余（若要修，只需把预处理正则可选地覆盖"无闭合则删到串尾"）。
3. **N-R4-03/04/06/07（P3）**：这批只是**引用精度**问题：建议在本轮记录里统一加一句"行号以 `rg` 检索为准"（REGRESSION-COVERAGE 已经这么写了），并把 07 §8 指向 09 §5.3。
