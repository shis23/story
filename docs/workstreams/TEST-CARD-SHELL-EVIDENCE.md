# test-card 壳路径证据清单（Phase 7）

> 金标：`test-card.png`（命定之诗与黄昏之歌 v4.1）  
> 复跑：`scripts/run-card-shell-evidence.ps1`

## 自动断言（CI/本地）

| # | 检查 | 命令 / 位置 |
| --- | --- | --- |
| 1 | 提取器：home/custom_start/status + ≥5 TH | `cargo test -p storyforge-domain card_shell` |
| 2 | allowlist 拒非白名单 host | `cargo test -p storyforge --lib card_shell_cache` |
| 3 | display `body.load` 分流 | `node --test frontend/tests/card-shell-display.test.mjs` |
| 4 | TH 顺序 / 按钮解析 | `node --test frontend/tests/tavern-helper-scripts.test.mjs` |
| 5 | 变量出站 campaign/instance | `node --test frontend/tests/shell-variable-outbox.test.mjs` |
| 6 | 世界书活动隔离 | `cargo test -p storyforge --lib campaign_store::tests::test_campaign_world_info` |

## 手工 UI 证据（Tauri）

1. 导入 `test-card.png` → 开活动  
2. 写作主屏应出现：  
   - 状态栏壳（CardShellHost compact）  
   - 开场壳（消息很少时）  
   - TavernHelper 状态条（6 脚本顺序 + 可见按钮：重新读取初始变量 / 重新处理变量 / 命定创意工坊）  
3. 日志中可检索：`shell_var_write`（变量出站）与 `card_shell_fetch` 失败时非静默  
4. 非 allowlist URL → 明确错误条，非纯文本“成功”

## 库存 URL

见 `docs/workstreams/test-card-shell-inventory.json`。
