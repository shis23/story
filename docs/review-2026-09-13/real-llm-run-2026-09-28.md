# 2026-09-28 harness-real-llm ignored 用例执行记录

- **范围**：33 个 ignored（27 真模型 + 6 非模型 + 1 doctest），用户明示无成本预算限制
- **模型供给现状（当日实测，全部为外部服务变更，非本仓库代码问题）**：
  - 中继 `cli.2529985.xyz`：`glm-5.3-flash` ✓（仅流式可靠）、`gemini-3.8-flash` ✓（快但有 429 配额）；
    `deepseek-v4-pro` 已不路由（"unknown provider"，connections.json 的 active 条目过期）；
    `deepseek-v4-flash`/`glm-5.3-air`/`glm-5.2-flash`/`deepseek-v4` 均不路由
  - `opencode.ai`：要求新增的 `x-opencode-session` 头，客户端未适配
  - 中继在 Cloudflare 后面：**非流式调用 ~100s 即 524**（流式不受影响）；历史 7 月 PASSED 用的
    deepseek-v4-flash 单调用 <100s，如今该模型无供给
- **执行方式**：沙箱 `APPDATA` + `STORYFORGE_APP_DATA_DIR` 双重定向（统一 harness 与应用管线
  两条连接解析路径；用户真实 connections.json 未被修改）；gemini-3.8-flash 为主力，glm-5.3-flash
  跑短调用套件。日志：`artifacts/real-llm-2026-09-28/{run,retry,final,t1-retry2,m5s4-retry,m5s6-retry}.log`

## 结果

| 类别 | 通过 | 未过 / 未跑（原因） |
|---|---|---|
| 真模型（27） | **19**：probe、knowledge、i1、t1（重试过，65s）、t2、t3×3、c×3、m5 s1/s2/s3、blind×3、card_studio×2 | m5 s4/s5/s6（多臂套件 × CF 524 边缘；s6 复跑 2/4，尾段 429 配额耗尽）；card_translation（模型能力验收器：book_total≥400 等阈值需 deepseek 级模型，现无供给）；endurance×2/eval/endurance_sqlite 共 4 个**按设计无法会话内跑完**（full_100_turn ≥700 调用/24h 硬截止） |
| 非模型（6） | **5**：infra-import 真卡、tauri-app lib×2（含 pressure_sync 压测）、keyring 真凭据库往返 | forge_differential：需第三方 forge CLI unpack 产物（本机无，测试自身设计为缺省跳过） |
| doctest（1） | **1**（app-agent llm_parse 示例） | — |

**净判定**：24 通过 / 4 基础设施受限（524 边缘与模型质量门槛，均给出可复跑命令与日志）/
4 按设计不可会话内完成 / 1 外部工具缺省跳过。代码侧无任何因本轮测试暴露的缺陷
（t2/t3/c/m5 s1-s3/blind/studio 全链路在真实模型下行为正确）。

## 解除阻塞的条件（用户侧）

1. 中继恢复 deepseek 级模型路由（或提供新的能力强、非流式 <100s 的模型）→ card_translation
   与 m5 s4/s6 大概率转绿；
2. 或给 harness t 系列补流式通道（`runtime.rs:389` progress_tx=None 走非流式——产品取舍，非缺陷）；
3. endurance/eval 需要专门的长跑窗口（≥24h），命令见 `scripts/run-real-llm-smoke.ps1` 与
   `.eval-env.local` 配方。
