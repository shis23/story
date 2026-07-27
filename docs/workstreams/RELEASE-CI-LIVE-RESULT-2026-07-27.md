# Release CI — 合并复核结果（2026-07-27）

## 结论

分支 `codex/release-ci-live`（head `fa32093`）已通过 merge commit `14f5977` 合入
本地 `main`。合并复核修完了原报告中未收口的 RunnerReadiness 合同问题，并补上
下载完整性与 Pester 版本契约。

本地门禁为 **PASS**；本分支未 push、没有对应 head 的远端 run，Windows runner 也
尚未注册，所以远端状态仍为 **PARTIAL/BLOCKED**。

## 合并后复核修正

### 工作流元数据与静态合同

- YAML 解析器现在返回每一个 workflow job 的元数据，不再只返回
  `windows-host-evidence`/`android-host-evidence`。
- host evidence 严格结果只覆盖对应两个 job；`frontend-gate`、`secret-scan`、
  `pester-release-tests` 保留通用解析结果供各自合同校验。
- 修复 top-level permissions、host env、producer `-OutputDir`、缺失 workflow
  失败关闭和错误消息等边界。
- 对抗性 fixture 显式按 UTF-8 读写，避免 Windows PowerShell 5 把中文注释误解为
  换行并破坏 YAML。

### 下载与测试运行器

- 三个 Linux job 的 Node 固定为 22.12.0；下载 tar 后先用 nodejs.org 同版本
  `SHASUMS256.txt` 校验，再解压到 toolcache。
- Windows gate 固定安装 Pester 4.10.1。
- `run-release-build-tests.ps1` 只选择 Pester 3.x/4.x。若机器只有 Pester 5，会明确
  失败并提示安装兼容版本，不再把 legacy `Should Be` 语法送给 Pester 5 后产生全红
  假回归。
- PyYAML 继续固定为 6.0.2，解析合同要求真实 YAML parser。

## 本地验证

- `.gitea/workflows/*.yml`：真实 PyYAML parser 全部通过。
- `ReleaseBuild.Tests.ps1`：44 passed。
- `ReleaseBuild.Pipeline.Tests.ps1`：14 passed。
- `ReleaseBuild.CI.Tests.ps1`：34 passed。
- `ReleaseBuild.RunnerReadiness.Tests.ps1`：99 passed。
- 合计：191 passed / 0 failed。
- repository secret scan：PASS。
- Windows release dry-run：PASS。
- Android host dry-run：PASS。
- `git diff --check`：PASS。

## 远端证据边界

- 历史 Gitea run #19 只覆盖旧 `origin/main@d9bc4e5`：四个 Linux job 成功，三个
  Windows job 因无 runner 长期 blocked。它不是本次代码的证据。
- Windows gates 已拆到 `windows-gates.yml`，普通 Linux push/PR run 不再被无
  Windows runner 的 job 卡成非终态。
- 本轮按用户要求只合并和本地提交，未 push，故没有当前 head 的远端 run。
- `release-host-evidence` 未触发，没有新的远端 artifact/provenance/SBOM。
- 注册 Windows runner、push 当前 head、观察新 Linux run 终态和可选 dispatch
  Windows gates，仍是后续远端验收项。
