# Campaign 全局变量

StoryForge 的变量分为两个持久作用域：

- **全局变量**：存放在 `Campaign.variables`，由整个故事活动共享。后处理写回时使用 `instance_id: null`。
- **角色变量**：存放在 `CharacterInstance.variables`，每个角色实例各自持有。后处理写回时必须提供对应的 `instance_id`。

普通角色卡 `stat_data`、`extensions.mvu.initvar` 等 MVU 字段只会进入角色变量 schema，不会自动提升为全局变量。

## 卡片声明全局变量

卡片只有在扩展中显式使用以下任一路径时，导入器才会把字段识别为 Campaign 全局 schema：

- `extensions.storyforge.campaign_variables`
- `extensions.campaign_variables`

推荐使用带命名空间的第一种格式：

```json
{
  "extensions": {
    "storyforge": {
      "campaign_variables": {
        "faction_tension": {
          "label": "阵营紧张度",
          "type": "int",
          "default": 12,
          "description": "整局共享的阵营冲突强度",
          "group": "世界状态"
        }
      }
    }
  }
}
```

支持的类型为 `int`、`float`、`string`、`bool` 和 `json`。也可以直接写标量默认值，例如 `"is_wartime": false`。

## 生命周期

新建 Campaign 时，系统会合并三项内置全局字段与卡片声明的全局 schema，并立即写入各字段的默认值：

- `story_clock`：故事时间
- `weather`：天气
- `world_state`：大势

卡片后来增加全局字段时，旧 Campaign 不会被静默改变。用户可在“Campaign → 变量 → 全局变量”中点击“同步卡片”。同步只补 schema 和缺失值，不覆盖故事运行期间已经变化的当前值。

“新增变量”只修改当前 Campaign，不反向修改角色卡模板。旧档若已有同名值但缺少 schema，补定义时会保留原值。

## 后处理约束

PostProcessor 收到的变量目录包含键名、中文标签、类型和作用域。它必须：

- 只输出目录中存在的键；
- 全局字段使用 `instance_id: null`；
- 角色字段使用当场角色实例 ID；
- 不把角色字段写入全局，也不把全局字段写入角色实例。

最终变量更新仍随 Turn 的结构化 mutation batch 一起提交，不绕过现有的接受屏障。
