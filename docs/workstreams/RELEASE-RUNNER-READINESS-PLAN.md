# Release Runner Readiness 工作计划

> 分支：`codex/release-runner-readiness`
> 基线：`8732e21`
> 类型：Gitea/host release 运行前就绪度与离线验证；不部署远端基础设施

## 目标

把现有 release workflow 与 host-side evidence runner 提升为“部署 runner 后可明确预检、可离线复核、失败关闭”的状态。解决当前“workflow 已提交但远端 Gitea runner 未实跑”的证据缺口，但不把本线写成远端 CI 已执行、已签名或 GUI/真机已验收。

## 当前事实

- Windows/Android host runner、manifest/provenance/subject sidecar、hash 和 workflow/Pester 基础已存在。
- 远端 Gitea runner、完整 Tauri bundle、Android APK、签名、GUI 和真机仍未验证。
- 现有 runner 默认 host-only；bundle/APK 必须显式请求并 fail closed。

## 交付范围

1. 审计两个 workflow 与 release 脚本的 runner 前置条件：OS、PowerShell、Rust target、Node/npm、Python/YAML parser、Tauri CLI、Android SDK/NDK、GTK/WebKit 等。
2. 新增 fail-closed 的本地预检工具与机器可读 report，区分：可运行、缺依赖、需要显式 bundle/APK、不可证明 GUI/设备。
3. 为 Gitea runner 准备不含凭证的操作说明/配置模板：标签、权限、缓存、artifact retention、并发取消、最小 host 权限与手工触发方式。
4. 补离线 artifact verifier：验证 manifest、provenance、subjects、sidecar SHA-256、dependency inventory 与路径脱敏；缺 subject/sidecar、hash 不符或未知 schema 必须失败。
5. 加强 workflow 静态测试，保证真 YAML parser、actions 固定版本、`npm ci`、secret scan 与 host-only/bundle 显式意图不会退化。

## 明确不做

- 不 SSH 部署/注册/修改 Gitea runner、VPS 或远端 workflow；这需要用户单独授权和可见的基础设施操作。
- 不签名、不发布、不上传真实产物、不调用真实模型。
- 不把 host-side/dry-run 写成 GUI、Android 真机或远端 CI 已通过。
- 不改 SQLite、写作 pipeline、插件、`HANDOFF.md` 或 `RELEASE-CHECKLIST.md`。

## 冲突隔离

- 本线拥有 `.gitea/workflows/**`、`scripts/release-build/**`、相关 Pester、`docs/operations/**` 与本线 RESULT。
- 不修改 harness、Tauri 写作命令、SQLite 或前端业务组件。
- 如发现需调整现有正式 release 语义，先以测试和文档提出，不删除 fail-closed 检查来求通过。

## TDD 与验收

新增或扩展 Pester。至少覆盖：

1. runner 缺少每一种关键依赖时 fail closed，report 不泄露路径/凭证。
2. host-only 与显式 bundle/APK 的意图严格区分；未显式授权时不安装/构建发布产物。
3. artifact verifier 对缺 subject、hash 篡改、BOM sidecar、路径逃逸、schema 漂移、敏感 warning/note 全部拒绝。
4. workflow 静态校验必须使用真 YAML parser，并断言固定 action、`npm ci`、target/SDK 预检和 artifact retention。
5. 本地 dry-run 产出可以离线重新验 hash；不把 dry-run 当作 remote run。

最低门禁：

```powershell
Invoke-Pester .\scripts\tests -CI
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-host-pipeline.ps1 -DryRun
git diff --check
```

按受影响范围运行已有 release Pester 和 workflow syntax gate。不要用远端网络可达替代 runner 实跑证据。

## 交付与提交

- 建议提交：预检/红测 → verifier → workflow/readiness docs → RESULT。
- 新增 `docs/workstreams/RELEASE-RUNNER-READINESS-RESULT.md`，明确本线只证明本地就绪度与离线验证，不证明远端 runner、bundle、APK、签名或 GUI。
- 不 push、不 merge main；收口时保持 worktree 干净。
