pub mod agent;
pub mod campaign;
pub mod character;
pub mod character_knowledge;
pub mod conversation;
pub mod llm;
pub mod message_layout;
pub mod mvu_translation;
pub mod preset;
pub mod prompt_module;
pub mod story_task;
pub mod variables;
pub mod world_info;

// 公共类型
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 唯一标识符（通用 newtype）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Id(String);

impl Id {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
    pub fn from_str(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 来源标记（区分数据从哪来）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    ImportedFromST, // 从 SillyTavern 导入
    Native,         // 本应用原生创建
}
