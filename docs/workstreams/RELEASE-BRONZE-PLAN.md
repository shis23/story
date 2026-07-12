# 桌面 Bronze 发布证据线计划

## 基线与边界

- 分支：`codex/release-bronze`
- 基线：`7fb1899`
- 工作目录：`C:\tmp\storyforge-release`
- 本线补桌面真实主流程、恢复和发布证据。
- 不修改 SQLite、M5 harness、Phase B 模型逻辑或 Android worktree。
- 尽量新增测试/脚本；发现产品 bug时先写最小复现，再做有界修复。

## 目标场景

优先覆盖发布清单中尚无真实证据的 Bronze 路径：

1. 启动应用并创建/选择 Campaign。
2. 导入最小角色卡和世界书。
3. 连续完成三轮写作、postprocess、Accept。
4. regenerate/Editor-only，确认旧 Attempt 不复活。
5. 编辑草稿后 hash 失效与重新推导。
6. 应用重启后的 Turn/Attempt/压缩任务恢复。
7. Meta explain/patch 和排障 bundle。
8. 不存在真实模型凭证时，使用 deterministic/mock 路径验证产品状态机。

## 工作方式

1. 先阅读 `docs/RELEASE-CHECKLIST.md`，为每个用例建立编号和明确前置条件。
2. 优先复用已有 Tauri command/测试 helper，不另造第二套业务路径。
3. 自动化无法证明 GUI 行为时，生成可复跑的人工步骤和证据模板。
4. 证据写到本目录或专用 artifacts 目录，不直接改 HANDOFF。
5. 若必须运行 GUI，使用独立 dev-data，不读取或覆盖用户真实数据。

## 可修改区域

- 发布/桌面专用测试和脚本
- `docs/workstreams/**`
- `docs/RELEASE-CHECKLIST.md` 仅在拥有真实新证据时修改
- 为可测试性所需的最小生产改动，必须带回归测试

## 禁止事项

- 不改 SQLite、存储真相源或 schema。
- 不改真实 LLM M5 harness。
- 不把自动化组件测试冒充真实 GUI 证据。
- 不使用或删除用户现有 Campaign 数据。
- 不修改现有 `w11-android` worktree。
- 不执行付费模型调用，除非用户明确授权。

## 验收门槛

- 每个 Bronze 条目明确标记 Automated / Manual / Not Run。
- 自动化用例必须断言最终持久化状态，不只检查命令 exit 0。
- `cargo fmt --all -- --check`
- `cargo test --workspace --quiet`
- `cargo clippy --workspace --all-targets -- -D warnings`
- 前端 Node 测试与生产构建（若涉及前端）
- `git diff --check 7fb1899..HEAD`

## 最终交付

结束时新增 `RELEASE-BRONZE-RESULT.md`，记录：

- 已跑用例编号与证据类型
- commit 和修改文件
- 实际测试结果
- 新发现的产品 bug与严重级别
- 仍需人工 GUI/真实模型验证的条目
- 是否建议合并
