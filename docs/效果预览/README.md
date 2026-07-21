# 前端效果预览（生图稿）

> 用途：存放 **gpt-image / 设计模型** 生成的 StoryForge UI 效果图，供选型与后续强模型落地。  
> 不是线上截图库，也不是 `USER-GUIDE` 的产品截图目录（产品截图仍规划在 `docs/images/`）。

## 目录

```text
docs/效果预览/
  inbox/       新生成、尚未筛选
  selected/    已选定、作为视觉方向或金样参考
  rejected/    明确不用（可留作对比，避免重复生成）
  README.md    本说明
  INDEX.md     图册索引（有图后维护）
```

## 放图规则

1. **先丢 `inbox/`**，选定后再移到 `selected/` 或 `rejected/`。  
2. **不要**把现有 `AppV2` 截图当风格参考放进来冒充新设计。  
3. 文件尽量 **PNG 或 WebP**；单张建议 < 5MB。  
4. 一轮生成可多变体；用文件名区分 `v1/v2/v3`。  
5. 若图中误含密钥、真实私卡正文、账号信息：**不要入库**，本地删掉重来。

## 命名（推荐）

```text
YYYYMMDD-<screen>-<platform>-<variant>-<note>.png
```

| 段 | 含义 | 示例 |
| --- | --- | --- |
| 日期 | 生成日 | `20260721` |
| screen | 画面 | `empty` `writing` `written` `campaign` `meta` `mobile-writing` |
| platform | 端 | `desktop` `mobile` |
| variant | 变体 | `v1` `v2` `v3` |
| note | 短备注 | `editorial` `light` `dark` `sidebar-collapsed` |

示例：

```text
20260721-empty-desktop-v1-editorial.png
20260721-writing-desktop-v2-streaming.png
20260721-written-desktop-v1-variants.png
20260721-campaign-desktop-v1-tabs.png
20260721-mobile-writing-mobile-v1.png
```

### 建议优先覆盖的 screen（与组件蓝图一致）

| screen | 内容 |
| --- | --- |
| `empty` | 首次启动：未导入 / 无 Campaign |
| `writing` | 写作中：流式、停止、主消息区 |
| `written` | 写完：消息、变体、轻量过程回顾 |
| `campaign` | Campaign 管理：卡 / 实例 / 知识 / 任务 |
| `meta` | Meta：health / patch preview（可次要） |
| `mobile-*` | 移动端主流程 |

## 选定标准（移入 selected 前自问）

- 像**创作/叙事写作工具**，不像 IDE、运维台、游戏 HUD  
- **写作区是主角**；调试/插件/trace 不抢戏  
- 中文长文层次清楚（标题 / 正文 / 次要信息）  
- 空状态、生成中状态也成立  
- 与 `docs/FRONTEND-COMPONENTS.md` 的能力不冲突（能想象出对应组件职责）

## 给后续模型怎么用

- **强模型落地**：只把 `selected/` 当视觉参考 + 读 `FRONTEND-COMPONENTS.md` 能力地图  
- **弱模型填组件**：给「选定图路径 + 金样组件路径 + 禁止改契约文件」  
- 未选定的 `inbox/` 不要当最终规范

## 与 Git

大图是否提交由你决定：

- **要进仓库**：方便其他 agent/会话对照 `selected/`  
- **不进仓库**：可只本地预览；或把 `docs/效果预览/**/*.png` 加进 `.gitignore`，只提交本 README / INDEX

当前默认：**目录与说明可提交；你放入的图片按需选择是否 commit。**

## 索引

有图后请更新同目录 `INDEX.md`（模板已给），至少记录：

- 文件名  
- 对应 screen  
- 状态：inbox / selected / rejected  
- 一句话评价或选用原因  
