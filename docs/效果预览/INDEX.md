# 效果预览索引

> 有图后按行追加。状态：`inbox` | `selected` | `rejected`。

| 日期 | 文件 | screen | 端 | 状态 | 备注 |
| --- | --- | --- | --- | --- | --- |
| 2026-07-21 | `selected/20260721-board-desktop-v1-editorial-paper.png` | board（empty/writing/written/campaign 四联） | desktop | selected | **主方向 A「纸上编辑部」主参考**：暖米白纸感 + 金棕点缀 + 衬线正文；布局、侧栏、写作区、变体卡以此为准 |
| 2026-07-21 | `selected/20260721-board-desktop-v2-minimal-ink.png` | board（四联） | desktop | selected | **主方向 A 辅参考**：极浅留白、克制层次；空状态排版、过程回顾时间线、战役管理密度参考 |
| 2026-07-21 | `rejected/20260721-board-desktop-v1-ink-landscape.png` | board（四联） | desktop | rejected | 方向 B「山水墨境」：水彩插画依赖重，含蓝图中不存在的“模板中心/智能体广场”，不采用 |
| 2026-07-21 | `rejected/20260721-board-desktop-v1-dark-atelier.png` | board（四联） | desktop | rejected | 方向 C「暗夜书阁」：深棕金铜偏奇幻游戏感，管理页易滑回调试台气质，不采用（其“夜读”氛围仅供深色变体情绪参考，不作规范） |

## 使用约束

- 落地实现只参考 `selected/` 两张；`rejected/` 仅用于避免重复生成。
- selected 图中出现的非蓝图能力（模板中心、智能体广场、世界观设定等）**不落地**，以 `docs/FRONTEND-COMPONENTS.md` 能力地图为准。
- 待补画面（生成后先入 `inbox/`）：`writing` 停止/错误态、`written` 的 ProcessReview 展开态、`campaign` 四 tab 细节、`meta` health+patch preview、`mobile-empty` / `mobile-writing`、夜读深色一帧。
