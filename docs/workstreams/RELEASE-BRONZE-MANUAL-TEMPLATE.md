# Bronze 人工 GUI / 真实模型证据模板

> 用途：自动化无法证明的桌面 GUI 与真实模型路径。
> 约束：使用独立 dev-data，不读取/覆盖用户真实 Campaign。
> 证据目录建议：`artifacts/bronze/B#-YYYY-MM-DD/`
> 2026-07-27 的完整双人执行顺序、停止条件与证据纪律见
> `DESKTOP-BRONZE-LIVE-ACCEPTANCE-2026-07-27.md`；本文继续作为 B1-B6 记录表。

## 公共前置

- 候选 SHA：
- 日期：
- 执行人：
- 平台：Windows 桌面 / 其他：
- 数据目录：独立 dev-data 路径（填绝对路径）
- 模型连接：无 / mock / 真实（模型名、endpoint，不要写 key）
- 输入材料：角色卡文件名

## B1 单角色首轮

- [ ] 启动应用
- [ ] 导入角色卡
- [ ] 角色列表刷新
- [ ] 创建 Campaign 并设为 active
- [ ] 写第一轮
- 预期：Director/Subagent/Editor trace 可见；正文落到当前 Campaign；无 legacy character 写作路径
- 证据：截图路径 / app 日志 / `log_export_bundle`
- 结果：待跑 / 通过 / 失败
- 备注：

## B2 多角色三轮

- [ ] 导入多角色卡并抽取 definitions
- [ ] 创建多 instance Campaign
- [ ] 连续 T1/T2/T3 写作
- [ ] 每轮检查 Pipeline + Campaign 面板
- 预期：分角色输出；三轮后 summaries/knowledge/variables/tasks 至少一类影响下一轮
- 证据：
- 结果：
- 备注：

## B3 同名隔离

- [ ] 两个同 display name、不同 instance id
- [ ] 给其中一个写私有知识/变量
- [ ] postprocess 后检查 knowledge/variables
- 预期：按 instance id 落盘；不串写；name 歧义不静默写错
- 证据：
- 结果：
- 备注：

## B4 后处理写回与重启恢复

- [ ] 写作后等待 postprocess
- [ ] 检查 summaries/knowledge/variables/tasks
- [ ] 重启 app 再检查 active Campaign 与状态
- 预期：摘要/知识/变量/任务可见；重启后恢复
- 证据：
- 结果：
- 备注：

## B5 Meta 解释与修复（GUI）

- 自动子项：`scripts/run-meta-smoke.ps1`（后端）
- [ ] 打开 Meta 面板
- [ ] health check
- [ ] 解释本轮生成
- [ ] patch preview / dismiss / accept
- 预期：解释引用真实 trace/provenance；preview 不直接写；dismiss 不改状态；accept 后 tab 刷新
- 证据：
- 结果：
- 备注：

## B6 排障与恢复（GUI）

- [ ] 触发失败（断网/取消生成）
- [ ] UI/日志可见错误
- [ ] 导出排障 bundle
- [ ] 重启 app
- 预期：失败不伪装成功；active state 不损坏；bundle 含诊断摘要且无真实 API key
- 证据：
- 结果：
- 备注：

## regenerate / Editor-only / 草稿编辑

- [ ] regenerate 后旧 Attempt 不复活
- [ ] 编辑草稿后 draft_hash 失效，不能直接 accept
- [ ] 重新推导后可 accept
- 证据：
- 结果：
- 备注：

## 结论

- 本轮可合并的自动化证据：
- 仍阻塞发布的人工项：
- 严重 bug：
