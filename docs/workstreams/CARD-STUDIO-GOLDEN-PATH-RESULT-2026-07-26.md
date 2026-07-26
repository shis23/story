# Card Studio 实机 Golden Path 验收结果（2026-07-26）

> Branch: `feat/card-studio-phase1`（已 rebase 到 main `61b80bb`）
> 端点: `https://cli.2529985.xyz/v1` · 模型: `deepseek-v4-pro`（key 只经环境变量 `LLM_API_KEY`，不落盘）
> Runner: `crates/tauri-app/tests/card_studio_golden_path_real_llm.rs`（`#[ignore]`，真实 LLM）
> 结论：**A 从零 / C 修订双路 golden path 全绿**，出卡质量闸门 JSON+PNG 双 round-trip 9/9 全 PASS。

## 运行方式

```text
$env:LLM_API_KEY='<key>'
cargo test -p storyforge --test card_studio_golden_path_real_llm -- --ignored --nocapture --test-threads=1
```

驱动测试镜像 `cardstudio_run_stage` / `cardstudio_run_review` 的完整命令层逻辑
（prompt 装配 → chat → `extract_json_object` → `apply_stage_json` → 阶段推进；
审查为规则+LLM hybrid，解析失败回退规则报告），in-memory 驱动不落 app data。
闸门与 `cardstudio_export_gate` 相同：compile → ST JSON / PNG 字节 → 真实
`infra-import::import_character` round-trip → `export_gate_checks`。

## A 从零（明月秋青 v1 stage pack）

brief：雨夜末班车站守灯人（低魔都市怪谈，单主角旅程卡）。

| 阶段 | 结果 | 耗时 | 输出 |
| --- | --- | --- | --- |
| basic | 一次通过 | 87.4s | 954 字符 |
| personality | 一次通过 | 55.4s | 802 字符 |
| worldview | 一次通过 | 29.4s | 400 字符 |
| opening | 一次通过 | 16.1s | 327 字符 |

- 审查：ok=true，score 80，source=hybrid，4 条非阻断 issue（性格调色盘建议手写充实、
  world_type 微调等）。
- 闸门：JSON 9/9 PASS + PNG 9/9 PASS（name/first_mes round-trip、世界书总数/全启用/
  常驻数、无死路由、keys 保留、组件 0/0、spec 3.0）。
- 产物：`artifacts/card-studio/golden-a-card.png`（可直接 GUI 导入）+
  `golden-a-evidence.json`（含完整 st_card_json）。

## C 修订（reverse_parse → 局部重跑 → 另存）

源卡「灯下客」确定性编译产生（不耗 LLM），`new_from_existing_character`
reverse_parse 正确回填基础字段与 2 条世界书，落检查阶段。

- 开场白局部重跑（带修订指令「更有动作感与画面感」）：一次通过，120.6s。
  修订前「他把灯往你这边挪了半寸：……末班车已经走了。」→ 修订后
  「他把灯挪了半寸，纸罩上的水珠滚下一粒。火苗缩了缩，又亮起来。…」——
  保留设定、动作感明显增强，符合修订指令。
- 审查：ok=true，score 72，hybrid。
- 另存语义：修订编译 domain id `f55b5113…` ≠ 源卡 id `088eb03d…`，原卡不受影响。
- 闸门：JSON 9/9 + PNG 9/9 全 PASS。
- 产物：`golden-c-card.png` + `golden-c-evidence.json`。

## 总耗时

432s（两路合计，`--test-threads=1` 顺序执行；单路 A ≈ 分钟级，满足「20 分钟做出可玩卡」
的时间预算，且全部阶段一次通过、无重试）。

## 边界与残留

1. **这是命令层语义的 headless 验收**，不是 GUI 点击流。GUI 补验时只需在写卡工作室
   重复同样操作（阶段生成 ×4 → 方法论审查 → 出卡质量闸门 → 导入）。
2. A 卡 name 沿用了项目名「golden-a-守灯人」——basic 模板尊重用户已给名字，brief 里
   没有单独起名时会以项目名为准。实际使用时在 brief 或 basic 阶段给出角色名即可。
3. A 卡世界书仅 1 条（brief 要求「规模小而完整」）；量产时可在 worldview 阶段
   user_note 里要求条目数下限。
4. 证据 JSON 已确认不含任何凭证。
