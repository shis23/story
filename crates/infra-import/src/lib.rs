pub mod compat;
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
    let card: StCharacterCard = serde_json::from_slice(strip_utf8_bom(data))?;
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

    let card: StCharacterCard = serde_json::from_slice(strip_utf8_bom(&json_bytes))?;
    Ok(Character::from_st_card(card))
}

/// 导入 ST 预设（JSON）
pub fn import_preset(data: &[u8]) -> Result<Preset, ImportError> {
    let st: StPreset = serde_json::from_slice(strip_utf8_bom(data))?;
    Ok(Preset::from_st(st))
}

/// 检查是否为 PNG 文件
fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == png::PNG_SIGNATURE
}

/// Strip a UTF-8 BOM so ST JSON exports saved as "UTF-8 with BOM" still parse.
pub fn strip_utf8_bom(data: &[u8]) -> &[u8] {
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    data.strip_prefix(UTF8_BOM).unwrap_or(data)
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
        assert!(entry0.constant);
        assert_eq!(
            entry0.route,
            storyforge_domain::world_info::LoreRoute::Constant
        );

        let entry1 = &book.entries[1];
        assert_eq!(entry1.keys, vec!["战斗", "危险"]);
        assert!(entry1.selective);
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
    fn test_import_character_from_json_v2_missing_optional_fields_default() {
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "data": {
                "name": "V2 Minimal"
            }
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let character = import_character_from_json(&bytes).expect("minimal V2 should parse");

        assert_eq!(character.name, "V2 Minimal");
        assert_eq!(character.spec_version, "2.0");
        assert_eq!(character.description, "");
        assert_eq!(character.personality, "");
        assert_eq!(character.scenario, "");
        assert_eq!(character.first_mes, "");
        assert!(character.tags.is_empty());
        assert!(character.alternate_greetings.is_empty());
        assert!(character.extensions.is_null());
        assert_eq!(character.raw_card_json["name"], "V2 Minimal");
    }

    #[test]
    fn test_import_character_from_json_v3_preserves_extensions_extra_and_book_edges() {
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "V3 Fidelity",
                "description": "keep everything important",
                "system_prompt": "system stays",
                "post_history_instructions": "post stays",
                "alternate_greetings": ["alt 1", "alt 2"],
                "group_only": true,
                "creator_notes": "unknown data-level ST field",
                "extensions": {
                    "unknown_plugin": {
                        "state": 42,
                        "nested": {"flag": true}
                    },
                    "regex_scripts": [
                        {
                            "id": "scoped-1",
                            "scriptName": "Scoped output cleanup",
                            "findRegex": "foo",
                            "replaceString": "bar",
                            "placement": [2],
                            "disabled": false,
                            "markdownOnly": true
                        }
                    ]
                },
                "character_book": {
                    "name": "Opaque Book",
                    "description": "book-level metadata should survive",
                    "scan_depth": 9,
                    "extensions": {
                        "book_plugin": {"enabled": true}
                    },
                    "entries": [
                        {
                            "id": 7,
                            "keys": ["primary"],
                            "secondary_keys": ["secondary"],
                            "content": "book entry",
                            "selective": true,
                            "selective_logic": 1,
                            "position": "after_char",
                            "order": 13,
                            "depth": 5,
                            "extensions": {
                                "entry_extra": {"rank": 9}
                            }
                        }
                    ]
                }
            }
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let character = import_character_from_json(&bytes).expect("V3 should parse");

        assert_eq!(character.name, "V3 Fidelity");
        assert_eq!(character.spec_version, "3.0");
        assert_eq!(character.system_prompt, "system stays");
        assert_eq!(character.post_history_instructions, "post stays");
        assert_eq!(character.alternate_greetings, vec!["alt 1", "alt 2"]);
        assert_eq!(character.extensions["unknown_plugin"]["state"], 42);
        assert_eq!(
            character.extensions["unknown_plugin"]["nested"]["flag"],
            true
        );
        assert_eq!(character.raw_card_json["group_only"], true);
        assert_eq!(
            character.raw_card_json["creator_notes"],
            "unknown data-level ST field"
        );
        assert_eq!(
            character.raw_card_json["character_book"]["name"],
            "Opaque Book"
        );
        assert_eq!(
            character.raw_card_json["character_book"]["description"],
            "book-level metadata should survive"
        );
        assert_eq!(character.raw_card_json["character_book"]["scan_depth"], 9);
        assert_eq!(
            character.raw_card_json["character_book"]["extensions"]["book_plugin"]["enabled"],
            true
        );

        let scoped_regex = character.scoped_regex_scripts();
        assert_eq!(scoped_regex.len(), 1);
        assert_eq!(scoped_regex[0].id, "scoped-1");
        assert_eq!(scoped_regex[0].markdown_only, Some(true));

        let book = character
            .embedded_world_info
            .expect("V3 character_book should import");
        assert_eq!(book.entries.len(), 1);
        let entry = &book.entries[0];
        assert_eq!(entry.st_id, Some(7));
        assert_eq!(entry.keys, vec!["primary"]);
        assert_eq!(entry.secondary_keys, vec!["secondary"]);
        assert_eq!(entry.content, "book entry");
        assert_eq!(
            entry.selective_logic,
            storyforge_domain::world_info::SelectiveLogic::Or
        );
        assert_eq!(entry.position, 1);
        assert_eq!(entry.order, 13);
        assert_eq!(entry.depth, 5);
        assert_eq!(entry.extensions["entry_extra"]["rank"], 9);
    }

    #[test]
    fn test_complex_card_json_to_png_round_trip_preserves_compat_fields() {
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "Complex Gate Card",
                "description": "release gate fixture",
                "personality": "precise",
                "scenario": "compatibility lab",
                "first_mes": "Primary opening.",
                "mes_example": "<START>\nExample dialog",
                "system_prompt": "system prompt survives",
                "post_history_instructions": "post history survives",
                "tags": ["compat", "roundtrip"],
                "creator": "StoryForge",
                "character_version": "gate-1",
                "alternate_greetings": ["Alt one.", "Alt two."],
                "group_only": true,
                "creator_notes": "unknown top-level data field survives",
                "extensions": {
                    "tavern_helper": {
                        "scripts": [
                            {"name": "status hook", "enabled": true}
                        ]
                    },
                    "regex_scripts": [
                        {
                            "id": "gate-regex",
                            "scriptName": "Gate regex",
                            "findRegex": "/foo/g",
                            "replaceString": "bar",
                            "placement": [1, 2],
                            "disabled": false,
                            "promptOnly": true
                        }
                    ],
                    "world": "Gate World",
                    "unknown_plugin": {
                        "nested": {"flag": "keep"}
                    }
                },
                "character_book": {
                    "entries": [
                        {
                            "id": 10,
                            "keys": ["always"],
                            "content": "Constant lore.",
                            "constant": true,
                            "position": "before_char",
                            "order": 20
                        },
                        {
                            "id": 11,
                            "keys": ["trigger"],
                            "secondary_keys": ["secondary"],
                            "content": "Selective lore.",
                            "selective": true,
                            "selective_logic": 1,
                            "position": "after_char",
                            "order": 21,
                            "depth": 4,
                            "extensions": {
                                "entry_extra": {"rank": 2}
                            }
                        }
                    ]
                }
            }
        });

        let imported =
            import_character_from_json(&serde_json::to_vec(&json).unwrap()).expect("JSON imports");
        let exported_data = storyforge_domain::character::to_st_data(
            &imported,
            None,
            imported
                .embedded_world_info
                .as_ref()
                .map(storyforge_domain::world_info::WorldInfoBook::to_st_book),
        );
        let st_card = png::make_st_card(exported_data, &imported.spec_version);
        let png_bytes =
            png::write_st_card_png(&st_card, None).expect("complex card should export as PNG");

        let round_tripped = import_character(&png_bytes).expect("exported PNG should re-import");

        assert_eq!(round_tripped.name, "Complex Gate Card");
        assert_eq!(round_tripped.spec_version, "3.0");
        assert_eq!(
            round_tripped.alternate_greetings,
            vec!["Alt one.", "Alt two."]
        );
        assert_eq!(round_tripped.raw_card_json["group_only"], true);
        assert_eq!(
            round_tripped.raw_card_json["creator_notes"],
            "unknown top-level data field survives"
        );
        assert_eq!(round_tripped.extensions["world"], "Gate World");
        assert_eq!(
            round_tripped.extensions["tavern_helper"]["scripts"][0]["name"],
            "status hook"
        );
        assert_eq!(
            round_tripped.extensions["unknown_plugin"]["nested"]["flag"],
            "keep"
        );

        let scripts = round_tripped.scoped_regex_scripts();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "gate-regex");
        assert_eq!(scripts[0].placement_codes, vec![1, 2]);
        assert_eq!(scripts[0].prompt_only, Some(true));

        let book = round_tripped
            .embedded_world_info
            .as_ref()
            .expect("world book should survive JSON -> PNG -> import");
        assert_eq!(book.entries.len(), 2);
        assert!(book.entries.iter().any(|entry| entry.constant));
        let selective = book
            .entries
            .iter()
            .find(|entry| entry.selective)
            .expect("selective lore should survive");
        assert_eq!(selective.keys, vec!["trigger"]);
        assert_eq!(selective.secondary_keys, vec!["secondary"]);
        assert_eq!(
            selective.selective_logic,
            storyforge_domain::world_info::SelectiveLogic::Or
        );
        assert_eq!(selective.depth, 4);
        assert_eq!(selective.extensions["entry_extra"]["rank"], 2);
    }

    #[test]
    #[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
    fn test_real_complex_card_fixture_preserves_core_st_fields() {
        let fixture_path = std::env::var_os("SF_COMPLEX_CARD_FIXTURE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("test-card.png")
            });
        let bytes = std::fs::read(&fixture_path).unwrap_or_else(|err| {
            panic!(
                "REAL-CORPUS MODE requires a local real card fixture, but failed to read {}: {err}.
Set SF_COMPLEX_CARD_FIXTURE or place test-card.png at the repo root.",
                fixture_path.display()
            )
        });

        let character = import_character(&bytes).expect("real complex card should import");

        assert_eq!(character.name, "命定之诗与黄昏之歌v4.1");
        assert_eq!(character.spec_version, "2.0");
        assert_eq!(character.alternate_greetings.len(), 6);
        assert!(character.raw_card_json.is_object());
        assert!(character.raw_card_json.get("extensions").is_some());

        let extensions = character
            .extensions
            .as_object()
            .expect("extensions should remain an object");
        for key in [
            "depth_prompt",
            "regex_scripts",
            "tavern_helper",
            "world",
            "xiaobaix-template",
        ] {
            assert!(extensions.contains_key(key), "missing extension key: {key}");
        }

        let book = character
            .embedded_world_info
            .as_ref()
            .expect("real complex card should include an embedded world book");
        assert_eq!(book.entries.len(), 441);
        assert_eq!(
            book.entries.iter().filter(|entry| entry.constant).count(),
            85
        );
        assert_eq!(
            book.entries.iter().filter(|entry| entry.selective).count(),
            340
        );
        assert!(
            book.entries
                .iter()
                .any(|entry| entry.content.contains("命定系统")),
            "expected imported world book to preserve 命定系统 content"
        );

        // Privacy-safe evidence: counts, known extension keys, feature flags,
        // and a linkable SHA-256 fingerprint. Print + write so --nocapture /
        // smoke runners actually produce auditable evidence.
        let evidence = crate::compat::sanitize_real_card_evidence(&character);
        assert_eq!(evidence.spec_version, "2.0");
        assert_eq!(evidence.alternate_greeting_count, 6);
        assert_eq!(evidence.world_book_entry_count, 441);
        assert_eq!(evidence.raw_card_json_sha256.len(), 64);
        assert_eq!(
            evidence.fingerprint_privacy,
            "stable-linkable-not-anonymous"
        );
        let evidence_json = serde_json::to_string(&evidence).expect("evidence serializes");
        assert!(
            !evidence_json.contains("命定之诗") && !evidence_json.contains("命定系统"),
            "sanitized real-card evidence leaked private content"
        );
        let again = crate::compat::sanitize_real_card_evidence(&character);
        assert_eq!(
            evidence.raw_card_json_sha256, again.raw_card_json_sha256,
            "real-card fingerprint must be deterministic (linkable, not anonymous)"
        );

        // Always print a one-line sanitized evidence summary (visible with --nocapture).
        let line = crate::compat::format_real_card_evidence_line(&evidence);
        println!("{line}");

        // Also write JSON evidence under artifacts/ (gitignored) when possible.
        let evidence_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("artifacts")
            .join("import-export-compat")
            .join("real-card-evidence.json");
        match crate::compat::write_real_card_evidence(&evidence, &evidence_path) {
            Ok(path) => println!("REAL-CARD EVIDENCE PATH: {}", path.display()),
            Err(e) => println!("REAL-CARD EVIDENCE WRITE SKIPPED: {e}"),
        }
    }

    #[test]
    fn test_import_character_from_png_without_chara_returns_no_character_data() {
        let png = png::build_placeholder_png();
        let err = import_character_from_png(&png).expect_err("PNG without chara should fail");

        assert!(matches!(err, ImportError::NoCharacterData));
    }

    #[test]
    fn test_import_character_from_png_rejects_bad_chara_base64() {
        let png = png_with_text_chunk("chara", "%%%not-base64%%%");
        let err = import_character_from_png(&png).expect_err("bad base64 should fail");

        assert!(matches!(err, ImportError::Base64Error(_)));
    }

    #[test]
    fn test_import_character_from_png_rejects_bad_chara_json() {
        let bad_json = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            br#"{"spec":"chara_card_v2","data":{"name":"broken"}"#,
        );
        let png = png_with_text_chunk("chara", &bad_json);
        let err = import_character_from_png(&png).expect_err("bad JSON should fail");

        assert!(matches!(err, ImportError::JsonError(_)));
    }

    #[test]
    fn test_import_character_from_png_accepts_bom_json_payload() {
        let mut json_bytes = vec![0xEF, 0xBB, 0xBF];
        json_bytes.extend(serde_json::to_vec(&make_test_card_json()).unwrap());
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json_bytes);
        let png = png_with_text_chunk("chara", &encoded);

        let character = import_character_from_png(&png).expect("PNG chara BOM should be accepted");

        assert_eq!(character.name, "测试角色");
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
                        "placement": [1],
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

    #[test]
    fn test_import_preset_preserves_st_regex_metadata() {
        let json = serde_json::json!({
            "name": "Regex Metadata Preset",
            "prompts": [],
            "extensions": {
                "regex_scripts": [
                    {
                        "id": "meta-1",
                        "scriptName": "Display-only output cleanup",
                        "findRegex": "/^[ \\t]+/gm",
                        "replaceString": "",
                        "placement": [2, 1],
                        "disabled": false,
                        "flags": "",
                        "markdownOnly": true,
                        "promptOnly": false,
                        "runOnEdit": true,
                        "substituteRegex": 0,
                        "trimStrings": ["`", "```"],
                        "minDepth": null,
                        "maxDepth": 3
                    },
                    {
                        "id": "meta-2",
                        "scriptName": "Prompt-only input wrapper",
                        "findRegex": "{{input}}",
                        "replaceString": "<input>$0</input>",
                        "placement": [1],
                        "disabled": true,
                        "promptOnly": true,
                        "runOnEdit": false,
                        "substituteRegex": 1,
                        "trimStrings": [],
                        "minDepth": 0,
                        "maxDepth": 0
                    }
                ]
            }
        });

        let bytes = serde_json::to_vec(&json).unwrap();
        let preset = import_preset(&bytes).expect("preset should parse");

        assert_eq!(preset.regex_scripts.len(), 2);

        let output = &preset.regex_scripts[0];
        assert_eq!(output.placement_codes, vec![2, 1]);
        assert_eq!(
            output.placement,
            storyforge_domain::preset::RegexPlacement::Output
        );
        assert_eq!(output.markdown_only, Some(true));
        assert_eq!(output.prompt_only, Some(false));
        assert_eq!(output.run_on_edit, Some(true));
        assert_eq!(output.substitute_regex, Some(0));
        assert_eq!(output.trim_strings, vec!["`", "```"]);
        assert_eq!(output.min_depth, None);
        assert_eq!(output.max_depth, Some(3));

        let prompt_only = &preset.regex_scripts[1];
        assert_eq!(prompt_only.placement_codes, vec![1]);
        assert_eq!(prompt_only.prompt_only, Some(true));
        assert_eq!(prompt_only.run_on_edit, Some(false));
        assert_eq!(prompt_only.substitute_regex, Some(1));
        assert_eq!(prompt_only.min_depth, Some(0));
        assert_eq!(prompt_only.max_depth, Some(0));
    }

    fn png_with_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&png::PNG_SIGNATURE);
        write_test_png_chunk(&mut data, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);

        let mut text_data = Vec::new();
        text_data.extend_from_slice(keyword.as_bytes());
        text_data.push(0);
        text_data.extend_from_slice(text.as_bytes());
        write_test_png_chunk(&mut data, b"tEXt", &text_data);
        write_test_png_chunk(&mut data, b"IEND", &[]);
        data
    }

    fn write_test_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(chunk_type);
        out.extend_from_slice(data);

        let mut crc_input = Vec::with_capacity(chunk_type.len() + data.len());
        crc_input.extend_from_slice(chunk_type);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32fast::hash(&crc_input).to_be_bytes());
    }

    #[test]
    fn test_import_rejects_empty_chara_payload_fail_closed() {
        // A chara tEXt whose payload is empty must be rejected (no JSON), not
        // silently produce a half-built Character.
        let png = png_with_text_chunk("chara", "");
        let err = import_character(&png).expect_err("empty chara must fail closed");
        assert!(matches!(
            err,
            ImportError::JsonError(_) | ImportError::NoCharacterData
        ));
    }

    #[test]
    fn test_import_rejects_oversized_total_import_before_any_parse() {
        // MAX_IMPORT_SIZE (100 MiB) is enforced before PNG parsing begins, so a
        // decompression/size bomb is rejected with no partial store and no
        // observable parse side effects.
        let mut bomb = Vec::new();
        bomb.extend_from_slice(b"\xEF\xBB\xBF{");
        bomb.extend(std::iter::repeat_n(b'A', MAX_IMPORT_SIZE + 16));
        bomb.extend_from_slice(b"}");
        let err = import_character(&bomb).expect_err("oversized import must fail closed");
        assert!(matches!(err, ImportError::PngError(_)));
    }

    #[test]
    fn test_import_is_all_or_nothing_no_partial_character() {
        // The importer is pure parsing with no store, so the no-partial-store
        // invariant is: a rejection never yields a (half-built) Character. We
        // exercise several rejection shapes and assert each returns Err with no
        // Ok value that could leak a partial build.
        let bad_png = png::build_placeholder_png(); // no chara chunk
        let truncated_json = br#"{"spec":"chara_card_v2","data":{"name":"x""#; // no closing brace
        let bad_base64 = png_with_text_chunk("chara", "%%%not-base64%%%");
        let bad_json = png_with_text_chunk(
            "chara",
            &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, br#"not json"#),
        );
        for (label, input) in [
            ("placeholder-png", bad_png.as_slice()),
            ("truncated-json", truncated_json),
            ("bad-base64", &bad_base64),
            ("bad-json", &bad_json),
        ] {
            assert!(
                import_character(input).is_err(),
                "{label}: importer must reject without producing a partial Character"
            );
        }
    }
}
