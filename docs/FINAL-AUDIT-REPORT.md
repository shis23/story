# 当前审计报告

审计日期：2026-06-16

## 结论

StoryForge 的代码已经包含 Campaign、多角色、后处理、Meta Agent 和对话树等关键能力，但主写作链路尚未以 Campaign 为中心。当前最大风险不是缺功能，而是数据权威分裂：

- `CharacterStore`/扁平 `Character` 仍在支撑写作。
- `CampaignStore` 已保存更正确的角色实例、变量、知识和任务。
- `app-pipeline` 还没有把 `CharacterInstance` 作为写作身份核心。

## 已核对事实

- Rust workspace 当前包含 14 个 crate。
- Tauri 命令数为 87。
- `CampaignStore` 位于 `crates/tauri-app/src/campaign_store.rs`。
- `WritingContext` 位于 `crates/app-pipeline/src/lib.rs`，当前仍包含 `characters: Vec<Arc<Character>>`。
- `ToolContext` 位于 `crates/app-agent/src/tools.rs`，当前工具仍按扁平角色卡查角色。
- `CharacterInstance::resolved_persona()` 当前只返回 override。
- `start_writing` 和 `regenerate` 是主要流式写作入口。

## 主要风险

1. 同名角色会导致知识和变量归属风险。
2. Campaign 状态无法稳定影响下一轮写作。
3. 后处理如果继续按名字落盘，会放大数据污染。
4. Meta Agent 如果先扩展聊天能力，会偏离维护层定位。
5. 前端如果继续堆面板，会让主流程更难收束。

## 建议

按 `docs/ROADMAP.md` 推进。第一优先级是 Campaign 写作主线统一；在这之前，不建议继续投入大型外围功能。
