# 0.1.1 收尾验收

执行时间：2026-09-05 至 2026-09-06，Asia/Shanghai。

结论：本地 11 步确定性门禁通过；四类修复完成回归和限定原生验证。正式发布、真实模型和完整第三方插件门禁仍未全部闭合。

## 修复内容

1. 系统凭据直接使用显式初始化的 `keyring_core`，移除在 Android/iOS 独立拒绝默认存储的包装层。新密钥不得以明文降级保存。
2. ST 导出使用当前 Campaign 世界书；PNG 保留本局世界书，附带 lorebook 合并已获取知识并避开条目 ID 冲突。底层转换的 `None` 表示不覆盖，显式空世界书表示清空。
3. SQLite 打开前设置 busy timeout；首次启用 WAL 遇到 BUSY 时做有界重试。已发布 V001-V008 只兼容精确 LF/CRLF 校验和，迁移账本不重写，修改过的 SQL 仍失败。未来 Git 检出固定 SQL 为 LF；反向导出使用相同校验政策。
4. 统一发布门禁纳入发布脚本、组件、CSP、移动布局和应用壳套件，共 11 步。UI smoke 从任意工作目录定位配置，并直接持有/结束自己的 Vite 进程。

## 确定性门禁

命令：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

本轮环境：Windows PowerShell、Pester 3.4.0、Python/PyYAML 6.0.2、Node 24、锁定的前端依赖及 Chromium。设置 `CARGO_BUILD_JOBS=2` 限制编译并发；没有设置 `RUST_TEST_THREADS=1`。

| 检查 | 结果 |
| --- | --- |
| Secret scan / fmt / Clippy `-D warnings` | 通过 |
| Rust workspace | 1980 通过，0 失败，33 忽略 |
| 前端 Node / Vitest | 499 / 119 通过 |
| Chromium CSP / 移动布局动效配色 / 应用壳 | 9 / 28 / 2 通过 |
| Pester 四套发布合同 | 51 + 14 + 34 + 99 = 198 通过，无跳过 |
| 前端生产构建 | 通过，保留包体和混合静态/动态导入警告 |

不声称覆盖率达到某个百分比，也不将需要真实模型或平台条件的 33 个忽略用例计为通过。UI smoke 另从仓库根目录直接执行通过，专用端口 41763 在结束后无监听。

## 原生专项

- Windows 实际系统凭据写/读/删测试通过：`cargo test -p storyforge-infra-util system_keyring_write_read_delete_roundtrip -- --ignored --nocapture`。
- Windows 调试候选以独立 AppData、SQLite 和 WebView 运行。13 项真实 Tauri IPC 检查通过：三种生成模式、采纳、未采纳稿件删除、已采纳历史保护、末尾分支、v3 快照恢复以及恢复后的继续生成。LLM 为仅监听本机的协议夹具，没有付费请求。IPC 驱动中的前端 prompt-hook 超时警告仍有记录，不能据此宣称真实插件主流程通过。
- Android 设备为 Redmi 23117RK66C，Android 16 / API 36。新旧调试 APK 证书相同，`adb install -r` 成功；未卸载、未清库。
- 真机六项检查通过：既有 Campaign 可读、Keystore 保存和 DTO 脱敏、重启后解密、删除后重新挂回旧 SecretRef 仍无法解密、旧明文配置迁移并重启读取、ST PNG 导出和原生后端重新导入保留两条世界书。
- 升级前后 SQLite 快照逐表对比：20 张表的行内容完全一致，包括 8 条迁移账本；两份数据库 `integrity_check=ok`。原故事、角色、世界书和 authority 没有被验收清空或改写。
- 原生截图已检查。上述 IPC/专项不能替代真实文件选择器、真实模型、后台生成、断网恢复及第三方插件的完整验收。

## 候选身份

这些是基于 `1ca37a2` 加本轮修复构建的调试候选；修复随后提交为 `bb39b43`。它们不是干净发布输入或生产签名的声明。构建后的调整只涉及测试夹具和门禁/验收文档。

| 产物 | SHA-256 |
| --- | --- |
| Windows debug EXE | `1e1cf441e5e0fb7f53dde324c10382b3d8687ba80c3ca9b9e47251641fee63dc` |
| Android arm64 debug APK | `27a1184de03c7056614829835a0e5729c0741bcc0e3f5c551484db4c574ede12` |
| Android 调试证书 | `04ba8572f716899d3f32cc477d53fc3f46cfd602a1cb6ba00958eb882a39d4f1` |

## 失败与边界

- 初轮全量 Rust 测试暴露并发首次打开锁竞争；修复后新增竞争回归和既有并发导入测试通过。
- 新增测试曾因未初始化独立 SQLite runtime、反向导出夹具目标落入活动数据根而失败。修正测试夹具后重新执行完整门禁，没有跳过失败项。
- ADB 旧管道写法曾中断合成连接配置，触发应用写入保护；恢复了本轮验收前已确认的空连接配置，改用文件推送和读回校验后，完整凭据专项通过。没有真实密钥或故事数据库修复操作。
- 真机有一局旧 v2 导入数据缺失源角色卡。当前版本拒绝导出不完整 v3 快照；失败已保留，原数据未改写。新格式测试通过不代表这份历史缺失数据已修复。
- 真实模型与预算、第三方插件样本、当前候选正式分发安装验收，以及推送后的远端 CI 仍需闭合。未执行推送、打 tag 或发布。
- 修复提交后尝试了 Windows 正式发布流程，干净输入检查通过，但在 `npm ci` 阶段因 `lightningcss.win32-x64-msvc.node` 占用（EPERM）失败；没有到 Rust release 或安装器构建。依赖目录因此不完整，追加 Android 三模式写作检查因找不到 Playwright 而未启动，不能计为通过。
- 用户随后要求停止测试和构建。本轮调试 App、模型夹具和 Android 构建残留进程已关闭，没有重装依赖或继续构建。恢复开发时须先处理本地文件占用并重新安装锁定依赖；此前完整门禁结果仍是停止前的已执行证据，不代表目前依赖目录可直接运行。

## 本地证据

原始日志和合成夹具在忽略目录 `artifacts/release-closure-2026-09-05/`，目录日期为任务开始日。

- `gate-verified.log`：完整 11 步成功；`full-gate*.log` 保留此前失败。
- `windows-build.log`、`android-build.log`：原生调试构建日志。
- `windows-release-build.log`：正式发布流程在依赖安装阶段失败的记录。
- `windows/ipc-verification.json`、`windows/fixed-story-native.png`。
- `android/verification.json`、`android/restored-story.png`、`android/legacy-source-failure.json`。
- `android-before.tar`、`android-database-comparison.json`：升级前备份与逐表比较。

这些本地产物不强制加入 Git；公开验收边界和复验命令由本文保留。
