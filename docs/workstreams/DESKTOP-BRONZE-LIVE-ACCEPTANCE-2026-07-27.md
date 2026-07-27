# Windows 桌面 Bronze 真实验收 Runbook（2026-07-27）

> 执行方式：主会话 Codex 与用户共同操作。Codex 负责构建、隔离数据、日志/存档核验；用户负责真实 GUI 点击、视觉判断和必要截图。
> 原则：每个检查点有证据才记 PASS；自动测试、后端真实模型测试和 GUI 实跑三者不得互相冒充。

## 1. 范围

本轮验证当前 Windows 候选 SHA 的用户主路径：

- 首次启动与连接配置；
- 导入卡、创建 Campaign、选择开场；
- 写作模式与调用量提示；
- 真实模型正文、Pipeline trace、回合小票与 Accept；
- 重启恢复、模式记忆、Sequential 后缀重演入口；
- 取消/失败可见性与诊断包脱敏。

不在本轮：多意图质量排名、Android、安装包签名、第三方插件全集、SQLite 默认切换。

## 2. 双方职责

### Codex

- 记录候选 SHA、构建命令、工具版本和测试基线；
- 创建并校验独立数据目录，绝不复用用户正式数据；
- 启动候选应用并持续读取进程、日志和存档状态；
- 每个 GUI 检查点告诉用户只需执行的下一组点击；
- 核对对话、Campaign、turn、receipt、diagnostic 中的结构事实；
- 生成脱敏 RESULT，正文只保留长度/指纹，不入库。

### 用户

- 在真实桌面窗口完成点击、输入和视觉判断；
- 确认是否出现卡顿、遮挡、错误文案、模式费用和 trace；
- 在 Codex 请求时提供截图或简短结果，不需要粘贴 API key；
- 涉及真实付费模型调用前作最后确认。

## 3. 证据目录

使用 gitignored：

```text
artifacts/desktop-bronze/<UTC timestamp>/
  RUN.md
  screenshots/
  logs/
  diagnostic/
  manifests/
```

`RUN.md` 记录 SHA、平台、数据目录、模型名、endpoint host、检查点和结果；不记录 key、完整正文、完整 prompt 或角色私密数据。

## 4. Gate 0：候选与隔离

- [ ] `git status --short` 干净；记录 HEAD。
- [ ] 前端生产构建、Rust workspace 编译门和严格 Clippy 已通过或记录准确复用证据。
- [ ] 使用全新绝对数据目录；目录解析后必须位于仓库下的 gitignored `artifacts/desktop-bronze/`。
- [ ] 现有正式 StoryForge 进程与验收进程可区分；不得让验收实例打开正式数据目录。
- [ ] 连接 secret 来自系统密钥存储或进程环境，日志中不回显。

失败即停止，不进入 GUI。

## 5. Gate 1：启动、导入、Campaign

用户操作：

1. 启动应用，确认主窗口无白屏、无持续 loading。
2. 打开连接配置，确认可识别模型但不显示完整 key。
3. 导入一张已知 PNG/JSON 卡。
4. 创建 Campaign，选择开场并进入写作页。

检查：

- [ ] 导入后卡、角色 definitions 和世界书摘要可见。
- [ ] Campaign 是 active；写作页没有误用旧 legacy 角色。
- [ ] 用户选择的开场落到后端会话，刷新/切页不回退。
- [ ] 截图：Campaign 总览、写作页开场。

## 6. Gate 2：Continuation 首轮与回合小票

用户操作：

1. 选择“续写”，确认显示 `1 次正文 + 2 次廉价记账`。
2. 输入一个短意图，确认后发起真实模型调用。
3. 观察流式输出与 Pipeline trace。
4. 点击 Accept，检查回合小票，取消至少一个可审条目后确认。

检查：

- [ ] 写作状态从 generating 到 awaiting acceptance，再到 committed。
- [ ] 正文流式可见，无重复尾段、永久 spinner 或错误角色名。
- [ ] Summarizer 与 PostProcessor 状态分开显示。
- [ ] 回合小票先于落库出现；取消项不写入，结构性 mutation 仍保留。
- [ ] Codex 核对 Campaign turn、summary/knowledge/variable/task 与 UI 一致。

## 7. Gate 3：模式产品契约

### Duet

- [ ] 使用恰好两个主要角色的 Campaign 或场景。
- [ ] UI 显示 `3–4 次正文编排 + 2 次廉价记账`。
- [ ] 真实运行后 trace 呈现 A-B-A/Editor-lite 语义，人物可区分。

### Sequential Crew

- [ ] 使用三到四名主要角色。
- [ ] UI 显示 `2+N 次顺序编排 + 2 次廉价记账` 和“重点”。
- [ ] 显式选择后真实运行；trace 顺序与公开接戏链一致。
- [ ] 若通过未显式模式的入口触发自动昂贵升档，必须在任何对话写入前提示确认；拒绝后无半轮消息。

真实调用成本较高，每一项由用户在执行前单独确认。本 Gate 可记 PARTIAL，不影响 Gate 1/2 的桌面主路径结论。

## 8. Gate 4：重启恢复与重演

- [ ] 关闭并重新启动同一验收数据目录。
- [ ] active Campaign、会话、已采纳正文、回合小票结果和模式记忆恢复。
- [ ] Sequential 产物显示“从选中角色起重演”入口；非 Sequential 产物不错误开放。
- [ ] 若用户同意额外模型成本，真实执行一次后缀重演，核对前缀不变、Director 未重启、只重放目标及下游。

## 9. Gate 5：失败、取消、诊断

- [ ] 在一次生成中点击取消，确认后端停止且 UI 不继续追加。
- [ ] 使用可恢复方式触发连接失败或超时，确认失败不伪装成功、没有永久 loading。
- [ ] 导出 diagnostic bundle；Codex 扫描其中不含 API key、Authorization、完整 prompt/正文。
- [ ] 重启后没有活跃 Attempt 卡在 Generating/Committing。

## 10. 严重度与停止条件

立即停止并修复：

- 正式数据目录被使用或覆盖；
- key/Authorization 出现在 UI、日志或 diagnostic；
- Accept 前已写入用户取消的状态；
- Campaign/会话跨对象串写；
- 重启后已提交数据消失；
- 取消后模型调用或 token 流持续不可控。

可记录后继续：视觉间距、非阻塞警告、已知 Vite 分包警告、单个非关键 trace 标签问题。

## 11. 结果文档

完成后新增：

`docs/workstreams/DESKTOP-BRONZE-LIVE-RESULT-2026-07-27.md`

每个 Gate 标记 PASS/PARTIAL/FAIL/BLOCKED，附候选 SHA、真实运行层级、截图/日志相对路径、模型调用次数和缺陷。只有用户实际看到 GUI 且 Codex 核对底层状态的条目，才可标记为桌面真实验收通过。
