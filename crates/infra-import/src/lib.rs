pub mod png;

use storyforge_domain::character::{Character, StCharacterCard};
use storyforge_domain::preset::{Preset, StPreset};

/// 导入错误
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("文件读取失败: {0}")]
    IoError(#[from] std::io::Error),

    #[error("PNG 格式错误: {0}")]
    PngError(String),

    #[error("Base64 解码失败: {0}")]
    Base64Error(#[from] base64::DecodeError),

    #[error("JSON 解析失败: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("未找到角色卡数据（PNG 中无 chara 块）")]
    NoCharacterData,

    #[error("不支持的文件格式: {0}")]
    UnsupportedFormat(String),
}

/// 导入文件大小上限（100MB）。恶意/误操作的超大文件会耗尽内存。
pub const MAX_IMPORT_SIZE: usize = 100 * 1024 * 1024;

/// 从文件字节导入角色卡（自动判断 PNG / JSON）
pub fn import_character(data: &[u8]) -> Result<Character, ImportError> {
    // H-7 防护：限制总大小，避免超大 JSON/PNG 耗尽内存
    if data.len() > MAX_IMPORT_SIZE {
        return Err(ImportError::PngError(format!(
            "文件过大（{} 字节，上限 {} 字节）",
            data.len(),
            MAX_IMPORT_SIZE
        )));
    }
    if is_png(data) {
        import_character_from_png(data)
    } else {
        import_character_from_json(data)
    }
}

/// 从 JSON 直接导入角色卡
pub fn import_character_from_json(data: &[u8]) -> Result<Character, ImportError> {
    let card: StCharacterCard = serde_json::from_slice(data)?;
    Ok(Character::from_st_card(card))
}

/// 从 PNG 文件导入角色卡（提取 tEXt "chara" 块）
pub fn import_character_from_png(data: &[u8]) -> Result<Character, ImportError> {
    let chunks = png::parse_png(data)?;

    // 找 "chara" tEXt 块
    let chara_text = chunks
        .iter()
        .find_map(|c| match c {
            png::PngChunk::Text { keyword, text } if keyword.eq_ignore_ascii_case("chara") => {
                Some(text.as_str())
            }
            _ => None,
        })
        .ok_or(ImportError::NoCharacterData)?;

    // Base64 解码 → JSON 解析
    let json_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chara_text)?;

    let card: StCharacterCard = serde_json::from_slice(&json_bytes)?;
    Ok(Character::from_st_card(card))
}

/// 导入 ST 预设（JSON）
pub fn import_preset(data: &[u8]) -> Result<Preset, ImportError> {
    let st: StPreset = serde_json::from_slice(data)?;
    Ok(Preset::from_st(st))
}

/// 检查是否为 PNG 文件
fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == png::PNG_SIGNATURE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个最小的 ST V3 角色卡 JSON
    fn make_test_card_json() -> serde_json::Value {
        serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "测试角色",
                "description": "一个用于测试的角色",
                "personality": "冷静、理性",
                "scenario": "在未来都市中",
                "first_mes": "你好，我是测试角色。",
                "mes_example": "",
                "system_prompt": "你是一个测试角色。",
                "post_history_instructions": "",
                "tags": ["test", "demo"],
                "creator": "StoryForge",
                "character_version": "1.0",
                "alternate_greetings": ["嗨！", "欢迎。"],
                "extensions": {},
                "character_book": {
                    "entries": [
                        {
                            "id": 1,
                            "keys": ["未来", "都市"],
                            "content": "这座城市建于2157年，是人类最后的庇护所。",
                            "constant": true,
                            "position": 0,
                            "order": 100
                        },
                        {
                            "id": 2,
                            "keys": ["战斗", "危险"],
                            "content": "城市外围有变异生物出没。",
                            "selective": true,
                            "position": 0,
                            "order": 100
                        }
                    ]
                }
            }
        })
    }

    #[test]
    fn test_import_character_from_json() {
        let json = make_test_card_json();
        let bytes = serde_json::to_vec(&json).unwrap();
        let character = import_character_from_json(&bytes).expect("解析失败");

        assert_eq!(character.name, "测试角色");
        assert_eq!(character.description, "一个用于测试的角色");
        assert_eq!(character.personality, "冷静、理性");
        assert_eq!(character.first_mes, "你好，我是测试角色。");
        assert_eq!(character.tags, vec!["test", "demo"]);
        assert_eq!(character.creator, "StoryForge");
        assert_eq!(character.spec_version, "3.0");
        assert_eq!(character.alternate_greetings, vec!["嗨！", "欢迎。"]);

        // 内嵌世界书
        let book = character.embedded_world_info.expect("应有内嵌世界书");
        assert_eq!(book.entries.len(), 2);

        let entry0 = &book.entries[0];
        assert_eq!(entry0.keys, vec!["未来", "都市"]);
        assert_eq!(entry0.constant, true);
        assert_eq!(
            entry0.route,
            storyforge_domain::world_info::LoreRoute::Constant
        );

        let entry1 = &book.entries[1];
        assert_eq!(entry1.keys, vec!["战斗", "危险"]);
        assert_eq!(entry1.selective, true);
        assert_eq!(
            entry1.route,
            storyforge_domain::world_info::LoreRoute::Selective
        );
    }

    #[test]
    fn test_import_character_from_json_v2_compat() {
        // V2 卡没有 spec_version 字段
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "data": {
                "name": "V2角色",
                "description": "旧版卡"
            }
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let character = import_character_from_json(&bytes).expect("V2 解析失败");

        assert_eq!(character.name, "V2角色");
        assert_eq!(character.spec_version, "2.0"); // 应默认为 2.0
        assert!(character.embedded_world_info.is_none()); // 无世界书
    }

    #[test]
    fn test_import_from_auto_detect() {
        // JSON 自动检测
        let json = make_test_card_json();
        let bytes = serde_json::to_vec(&json).unwrap();
        let character = import_character(&bytes).expect("自动检测 JSON 失败");
        assert_eq!(character.name, "测试角色");
    }

    #[test]
    fn test_import_preset() {
        let json = serde_json::json!({
            "name": "测试预设",
            "prompts": [
                {
                    "identifier": "main",
                    "name": "系统提示",
                    "role": "system",
                    "content": "你是一个写作助手。",
                    "system_prompt": true,
                    "deletable": false
                },
                {
                    "identifier": "user_input",
                    "name": "用户输入",
                    "role": "user",
                    "content": "",
                    "deletable": false
                }
            ],
            "extensions": {
                "regex_scripts": [
                    {
                        "id": "r1",
                        "scriptName": "去复述",
                        "findRegex": "^(.{0,20}).*\\1",
                        "replaceString": "$1",
                        "placement": [0],
                        "disabled": false,
                        "flags": "gm"
                    },
                    {
                        "id": "r2",
                        "scriptName": "格式清理",
                        "findRegex": "\\n{3,}",
                        "replaceString": "\\n\\n",
                        "placement": [2],
                        "disabled": false,
                        "flags": "gm"
                    }
                ]
            }
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let preset = import_preset(&bytes).expect("预设解析失败");

        assert_eq!(preset.name, "测试预设");
        assert_eq!(preset.prompts.len(), 2);
        assert_eq!(preset.regex_scripts.len(), 2);

        // 验证正则脚本
        assert_eq!(preset.regex_scripts[0].script_name, "去复述");
        assert_eq!(
            preset.regex_scripts[0].placement,
            storyforge_domain::preset::RegexPlacement::Input
        );
        assert_eq!(preset.regex_scripts[1].script_name, "格式清理");
        assert_eq!(
            preset.regex_scripts[1].placement,
            storyforge_domain::preset::RegexPlacement::Output
        );

        // 验证过滤
        assert_eq!(preset.input_regex_scripts().len(), 1);
        assert_eq!(preset.output_regex_scripts().len(), 1);
    }
}
