# Task F：MVU apply patch（提议→预览→接受→写盘）

> 你的 worktree：`C:\tmp\sf-taskF`，分支：`codex/mvu-apply`
> 起始 commit：main `413ac0b`。
> 必读：`docs/tasks/ROUND-4-README.md`（文件重叠约定）。

## 你负责的文件（只动这三个）

- **新建** `crates/app-meta/src/mvu_apply.rs`
- **修改** `crates/app-meta/src/lib.rs` —— **仅追加** `pub mod mvu_apply;` 和 re-export。
- **修改** `crates/tauri-app/src/lib.rs` —— **仅追加** 2 个新 Tauri 命令 + invoke_handler 注册。

**禁止改动**：
- `crates/app-agent`（E 的领地）、`crates/domain`、`crates/app-pipeline`。
- `crates/app-meta/src/meta_conversation.rs`（G 的领地）。
- `frontend`。
- `crates/app-meta/src/lib.rs` 里现有的 mod/re-export（只追加，不改现有）。

## 背景：现状

MVU 分析（`meta_analyze_mvu_card`）产出 `StoredMvuTranslation`（存在 `campaign_store`），含 `translation: MvuTranslation`，其中 `variable_schema: Vec<VariableField>` 是从 ST 卡 extensions 解析出的变量字段定义。

`CharacterDefinition.variable_schema: Vec<VariableField>` 是角色定义的变量 schema。

**缺口**：没有把 MVU 的 `variable_schema` 合并进 `CharacterDefinition.variable_schema` 的可审阅 patch。现在 MVU 分析完就存着，用户无法应用它。

## 已有的纯函数（直接复用，不要重写）

- `storyforge_domain::variables::merge_schema(base: &[VariableField], extra: &[VariableField]) -> Vec<VariableField>`（`variables.rs:143`）：extra 同名覆盖 base，新字段追加。
- `storyforge_domain::mvu_translation::MvuTranslation::merged_variable_schema(&self, base: &[VariableField]) -> Vec<VariableField>`（`mvu_translation.rs:145`）：内部调 merge_schema。
- `storyforge_domain::variables::init_values_from_schema(schema: &[VariableField], turn: u32) -> Vec<VariableValue>`（`variables.rs:133`）：从 schema 初始化变量值。

## 要实现的（`mvu_apply.rs`）

### 数据结构

```rust
use serde::{Deserialize, Serialize};
use storyforge_domain::variables::VariableField;

/// MVU schema 合并预览：对比 CharacterDefinition 当前 schema vs 合并 MVU 后的 schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuApplyPreview {
    pub source_character_id: String,
    pub character_name: String,
    pub definition_id: String,
    /// 合并后会被新增的字段（MVU 有、当前 schema 无）
    pub added_fields: Vec<VariableField>,
    /// 合并后会被覆盖的字段（同名，MVU 版本替换当前）
    pub overwritten_fields: Vec<VariableField>,
    /// 不变的字段（同名同值）
    pub unchanged_count: usize,
    /// 合并后的完整 schema（预览最终结果）
    pub merged_schema: Vec<VariableField>,
    /// 是否有任何变化（added + overwritten 都空 = 无变化）
    pub has_changes: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MvuApplyError {
    #[error("MVU 翻译不存在: {0}")]
    TranslationNotFound(String),
    #[error("Definition 不存在: {0}")]
    DefinitionNotFound(String),
    #[error("合并后无变化")]
    NoChanges,
}
```

### 纯函数（签名锁死）

```rust
/// 计算合并预览（纯函数，不写盘）。
/// 输入：当前 definition 的 variable_schema + MVU translation 的 variable_schema。
pub fn compute_apply_preview(
    current_schema: &[VariableField],
    mvu_schema: &[VariableField],
    definition_id: &str,
    character_name: &str,
    source_character_id: &str,
) -> MvuApplyPreview;

/// 把合并后的 schema 应用到一份 definition 副本（纯函数，验证用）。
pub fn apply_schema_to_definition(
    def: &mut storyforge_domain::character::CharacterDefinition,
    merged_schema: Vec<VariableField>,
);
```

`compute_apply_preview` 逻辑：
1. `merged = merge_schema(current, mvu)`。
2. `added` = merged 中 current 没有的 key。
3. `overwritten` = merged 中 current 有同名但内容（label/value_type/default）不同的。
4. `unchanged` = 其余。
5. `has_changes` = added 或 overwritten 非空。

## 要实现的 Tauri 命令（2 个，在 lib.rs 追加）

### 1. `meta_preview_mvu_apply`

```rust
#[tauri::command]
fn meta_preview_mvu_apply(
    source_character_id: String,
    campaign_id: String,
) -> Result<MvuApplyPreview, String>
```

逻辑：
1. 从 `get_campaign_store().get_mvu(&source_character_id)` 取 MVU translation。不存在 → Err。
2. 从 campaign → card → `character_definitions`，**列出所有 definition**（让用户选哪个应用——但本命令先返回第一个有变化的，或所有 definition 的预览列表）。
   - **简化**：返回 `Vec<MvuApplyPreview>`（每个 definition 一条预览），让前端展示列表。改签名为 `-> Result<Vec<MvuApplyPreview>, String>`。
3. 对每个 definition 调 `compute_apply_preview`。

### 2. `meta_apply_mvu_schema`

```rust
#[tauri::command]
fn meta_apply_mvu_schema(
    source_character_id: String,
    definition_id: String,
) -> Result<(), String>
```

逻辑：
1. 取 MVU translation + 该 definition。
2. `compute_apply_preview` 算 merged_schema。`has_changes == false` → Err(NoChanges)。
3. **写盘**：clone card → 改对应 definition 的 variable_schema → `get_campaign_store().update_card(card)`。
   - 注意：`character_definitions` 在 `CharacterCard` 里，改一个 definition 要把整个 card 写回。
4. 写盘后，**可选**：对已存在的 instances 补齐新变量（用 `init_values_from_schema` 取新字段 default，`set_variable`）。这步 best-effort，失败不阻断 schema 应用。

## 注册

把 2 个命令加进 invoke_handler。

## 约束

- **不写盘前必须先 compute_apply_preview**（apply 命令内部先调它，no changes 则拒绝）。
- **不破坏现有 MVU 数据**：只读 `StoredMvuTranslation`，不改它。
- apply 是**不可逆**的（schema 合并后无法自动回滚）——但因为是「MVU 覆盖定义」语义明确，可接受。前端会先 preview 再 apply。
- 不要在 app-meta 里碰 store lock（mvu_apply.rs 是纯函数；store 访问全在 lib.rs 命令里）。

## 验证（你的 worktree 内运行）

```cmd
cargo test -p storyforge-app-meta
cargo test -p storyforge --lib
cargo check --workspace
```

`cargo check --workspace` 必须无新编译错误。

## 测试要求

**mvu_apply.rs 纯函数测试（≥ 6 个）**：
- `compute_apply_preview`：MVU 全新字段 → added 非空，has_changes=true。
- `compute_apply_preview`：MVU 覆盖现有字段 → overwritten 非空。
- `compute_apply_preview`：无变化 → has_changes=false。
- `compute_apply_preview`：混合（部分新增部分覆盖部分不变）→ 三类计数正确。
- `apply_schema_to_definition`：def.variable_schema 被替换为 merged。
- `apply_schema_to_definition`：其他字段（persona_prompt 等）不变。

**lib.rs 集成测试（≥ 1 个）**：
- `meta_apply_mvu_schema`：构造 MVU translation + definition，apply 后重新读 card 确认 variable_schema 含新字段。

## 完成后报告

1. 改动文件列表 + 行数。
2. 新增测试数量 + 两个 test 命令结果。
3. `cargo check --workspace` 是否有新错误。
4. 是否动了禁止文件（尤其确认没碰 meta_conversation.rs 和 app-agent）。
