use crate::ImportError;
use storyforge_domain::character::{StCharacterCard, StCharacterData};

/// PNG 签名（8 字节）
pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// PNG 块类型
#[derive(Debug)]
pub enum PngChunk {
    Text { keyword: String, text: String },
    Other { chunk_type: [u8; 4] },
}

/// 解析 PNG 文件，提取所有块
pub fn parse_png(data: &[u8]) -> Result<Vec<PngChunk>, ImportError> {
    // 验证签名
    if data.len() < 8 {
        return Err(ImportError::PngError("文件太小，不是有效 PNG".into()));
    }
    if data[..8] != PNG_SIGNATURE {
        return Err(ImportError::PngError("PNG 签名不匹配".into()));
    }

    let mut chunks = Vec::new();
    let mut pos = 8; // 跳过签名

    while pos + 8 <= data.len() {
        // 读取长度（大端序）
        let length =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;

        // H-7 防护：单 chunk 大小上限（64MB）。恶意 PNG 可声明超大 tEXt 块吃光内存
        const MAX_CHUNK_SIZE: usize = 64 * 1024 * 1024;
        if length > MAX_CHUNK_SIZE {
            return Err(ImportError::PngError(format!(
                "PNG 块过大（{} 字节，上限 {} 字节），可能为恶意文件",
                length, MAX_CHUNK_SIZE
            )));
        }

        // 读取类型
        let chunk_type = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];

        // 检查剩余数据是否足够
        let total_chunk_size = 4 + 4 + length + 4; // length + type + data + crc
        if pos + total_chunk_size > data.len() {
            return Err(ImportError::PngError(format!(
                "块 {} 数据不完整（期望 {} 字节，剩余 {}）",
                String::from_utf8_lossy(&chunk_type),
                total_chunk_size,
                data.len() - pos
            )));
        }

        // 验证 CRC（覆盖 type + data）
        let crc_start = pos + 4; // type 开始
        let crc_end = pos + 8 + length; // data 结束
        let expected_crc = u32::from_be_bytes([
            data[crc_end],
            data[crc_end + 1],
            data[crc_end + 2],
            data[crc_end + 3],
        ]);
        let actual_crc = crc32fast::hash(&data[crc_start..crc_end]);
        if actual_crc != expected_crc {
            return Err(ImportError::PngError(format!(
                "CRC 校验失败: 块 {}",
                String::from_utf8_lossy(&chunk_type)
            )));
        }

        // 解析 tEXt 块
        let type_str = std::str::from_utf8(&chunk_type).unwrap_or("");
        if type_str == "tEXt" {
            let chunk_data = &data[pos + 8..pos + 8 + length];
            if let Some((keyword, text)) = parse_text_chunk(chunk_data) {
                chunks.push(PngChunk::Text { keyword, text });
            }
        } else {
            chunks.push(PngChunk::Other { chunk_type });
        }

        // 跳到下一块
        pos += total_chunk_size;

        // IEND 块后停止
        if type_str == "IEND" {
            break;
        }
    }

    Ok(chunks)
}

/// 解析 tEXt 块数据：keyword\0text
fn parse_text_chunk(data: &[u8]) -> Option<(String, String)> {
    // 找 null 分隔符
    let null_pos = data.iter().position(|&b| b == 0)?;
    let keyword = String::from_utf8(data[..null_pos].to_vec()).ok()?;
    let text = String::from_utf8(data[null_pos + 1..].to_vec()).ok()?;
    Some((keyword, text))
}

// ─── PNG 导出 ───────────────────────────────────────────────────────────────

/// 生成 1×1 灰度占位 PNG（底图缺失时用）
///
/// 手工构建，无第三方 png crate 依赖。
/// 结构：签名 + IHDR(1x1 grayscale) + IDAT(zlib stored block, 2 bytes) + IEND。
pub fn build_placeholder_png() -> Vec<u8> {
    let mut png = Vec::with_capacity(128);

    // PNG 签名
    png.extend_from_slice(&PNG_SIGNATURE);

    // IHDR：1×1, grayscale (bit_depth=8, color_type=0)
    let ihdr_data: [u8; 13] = [
        0, 0, 0, 1, // width = 1
        0, 0, 0, 1, // height = 1
        8, // bit depth = 8
        0, // color type = grayscale
        0, // compression
        0, // filter
        0, // interlace
    ];
    write_chunk(&mut png, b"IHDR", &ihdr_data);

    // IDAT：zlib-wrapped deflate stored block for 2 bytes (filter=0, pixel=0x80)
    // zlib header (0x78, 0x01) + stored block header (0x01, 0x02, 0x00, 0xFD, 0xFF) +
    // data (0x00, 0x80) + adler32 (0x00, 0x81, 0x00, 0x81)
    let idat_data: [u8; 13] = [
        0x78, 0x01, // zlib header (CM=8, CINFO=7, no dict, level 0)
        0x01, // deflate stored block (BFINAL=1, BTYPE=00)
        0x02, 0x00, // LEN = 2
        0xFD, 0xFF, // NLEN = ~2
        0x00, 0x80, // data: filter=None, pixel=128 (gray)
        0x00, 0x81, 0x00, 0x81, // adler32 of [0x00, 0x80]
    ];
    write_chunk(&mut png, b"IDAT", &idat_data);

    // IEND
    write_chunk(&mut png, b"IEND", &[]);

    png
}

/// 构建 tEXt 块字节（keyword\0text）
fn build_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(keyword.len() + 1 + text.len());
    data.extend_from_slice(keyword.as_bytes());
    data.push(0); // null separator
    data.extend_from_slice(text.as_bytes());
    data
}

/// 写一个 PNG 块：length(4) + type(4) + data + crc(4)
fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let length = data.len() as u32;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc_input: Vec<u8> = chunk_type.iter().chain(data.iter()).copied().collect();
    let crc = crc32fast::hash(&crc_input);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// 将 ST 角色卡写入 PNG（tEXt "chara" 块 + 底图）
///
/// - `card`：要导出的 ST 角色卡结构
/// - `base_image`：底图 PNG 字节。若为 None 或无效，用占位纯色图。
///
/// 返回完整的 PNG 文件字节。
pub fn write_st_card_png(
    card: &StCharacterCard,
    base_image: Option<&[u8]>,
) -> Result<Vec<u8>, ImportError> {
    // 序列化 card → JSON → base64
    let json_bytes = serde_json::to_vec(card)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &json_bytes);

    // 底图：优先调用方提供的，缺则占位
    let base = match base_image {
        Some(data) if data.len() >= 8 && data[..8] == PNG_SIGNATURE => data,
        _ => &build_placeholder_png(),
    };

    // 从底图提取：签名 + IHDR（到第一个非 IHDR 块之前）
    // 策略：复制签名和所有非 IEND 块，在 IEND 之前插入 tEXt
    let mut out = Vec::with_capacity(base.len() + b64.len() + 128);
    out.extend_from_slice(&base[..8]); // 签名

    // 遍历底图的块
    let mut pos = 8usize;
    let mut inserted = false;
    while pos + 8 <= base.len() {
        let length =
            u32::from_be_bytes([base[pos], base[pos + 1], base[pos + 2], base[pos + 3]]) as usize;
        let chunk_type = [base[pos + 4], base[pos + 5], base[pos + 6], base[pos + 7]];
        let total = 4 + 4 + length + 4; // length + type + data + crc
        if pos + total > base.len() {
            break; // 数据不完整，截断
        }

        // 在 IEND 之前插入 tEXt
        if &chunk_type == b"IEND" && !inserted {
            let text_data = build_text_chunk("chara", &b64);
            write_chunk(&mut out, b"tEXt", &text_data);
            inserted = true;
        }

        // 复制原始块
        out.extend_from_slice(&base[pos..pos + total]);
        pos += total;

        if &chunk_type == b"IEND" {
            break;
        }
    }

    Ok(out)
}

/// 将 StCharacterData 包装为 StCharacterCard（导出辅助）
pub fn make_st_card(data: StCharacterData, spec_version: &str) -> StCharacterCard {
    StCharacterCard {
        spec: Some("chara_card_v2".into()),
        spec_version: Some(spec_version.into()),
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在占位 PNG 的 IEND 前插入任意 keyword 的 tEXt 块（测试 ccv3-only 卡用）
    fn placeholder_png_with_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
        let base = build_placeholder_png();
        let mut out = Vec::with_capacity(base.len() + text.len() + 64);
        out.extend_from_slice(&base[..8]);
        let mut pos = 8usize;
        while pos + 8 <= base.len() {
            let length =
                u32::from_be_bytes([base[pos], base[pos + 1], base[pos + 2], base[pos + 3]])
                    as usize;
            let chunk_type = [base[pos + 4], base[pos + 5], base[pos + 6], base[pos + 7]];
            let total = 4 + 4 + length + 4;
            if &chunk_type == b"IEND" {
                let text_data = build_text_chunk(keyword, text);
                write_chunk(&mut out, b"tEXt", &text_data);
            }
            out.extend_from_slice(&base[pos..pos + total]);
            pos += total;
            if &chunk_type == b"IEND" {
                break;
            }
        }
        out
    }

    #[test]
    fn test_import_falls_back_to_ccv3_only_chunk() {
        // v3-only 卡：PNG 里只有 ccv3 块、无 chara 块（chara_card_v3 规范允许）
        let card = StCharacterCard {
            spec: Some("chara_card_v3".into()),
            spec_version: Some("3.0".into()),
            data: serde_json::from_value(serde_json::json!({ "name": "V3Only" }))
                .expect("minimal st data"),
        };
        let json_bytes = serde_json::to_vec(&card).unwrap();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &json_bytes);
        let png = placeholder_png_with_text_chunk("ccv3", &b64);

        let character = crate::import_character_from_png(&png).expect("ccv3-only 卡应可导入");
        assert_eq!(character.name, "V3Only");
        assert_eq!(character.spec_version, "3.0");

        // 无 chara 也无 ccv3 → 仍然 fail-closed
        let empty = placeholder_png_with_text_chunk("other", "x");
        assert!(crate::import_character_from_png(&empty).is_err());
    }

    #[test]
    fn test_png_signature_check() {
        let not_png = b"not a png";
        assert!(parse_png(not_png).is_err());

        let too_short = &[137, 80, 78];
        assert!(parse_png(too_short).is_err());
    }

    #[test]
    fn test_parse_text_chunk() {
        // keyword: "chara", null, text: "hello"
        let data = b"chara\0hello";
        let (keyword, text) = parse_text_chunk(data).unwrap();
        assert_eq!(keyword, "chara");
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_build_placeholder_png_is_valid() {
        let png = build_placeholder_png();
        assert!(png.len() >= 8);
        assert_eq!(png[..8], PNG_SIGNATURE);
        // 可以被 parse_png 解析
        let chunks = parse_png(&png).expect("占位 PNG 应可解析");
        // 应有 IHDR, IDAT, IEND（至少 3 个块）
        assert!(chunks.len() >= 3);
    }

    #[test]
    fn test_write_st_card_png_round_trip() {
        use storyforge_domain::character::StCharacterData;

        let data = StCharacterData {
            name: "测试角色".into(),
            description: "用于测试".into(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec!["test".into()],
            creator: "StoryForge".into(),
            character_version: "1.0".into(),
            alternate_greetings: vec![],
            extensions: serde_json::json!({}),
            character_book: None,
            extra: Default::default(),
        };
        let card = make_st_card(data, "3.0");

        // 写入 PNG（无底图，用占位图）
        let png_bytes = write_st_card_png(&card, None).expect("write_png 失败");

        // 验证：能被 parse_png 解析
        let chunks = parse_png(&png_bytes).expect("导出 PNG 应可解析");

        // 找 tEXt "chara" 块
        let chara_text = chunks
            .iter()
            .find_map(|c| match c {
                PngChunk::Text { keyword, text } if keyword == "chara" => Some(text.as_str()),
                _ => None,
            })
            .expect("应有 chara tEXt 块");

        // base64 decode → JSON → StCharacterCard
        let json_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chara_text)
                .expect("base64 解码失败");
        let exported_card: StCharacterCard =
            serde_json::from_slice(&json_bytes).expect("JSON 解析失败");

        assert_eq!(exported_card.data.name, "测试角色");
        assert_eq!(exported_card.data.description, "用于测试");
        assert_eq!(exported_card.data.tags, vec!["test"]);
        assert_eq!(exported_card.spec_version, Some("3.0".into()));
    }

    #[test]
    fn test_write_st_card_png_preserves_v3_extensions_extra_and_book_payload() {
        use std::collections::BTreeMap;
        use storyforge_domain::character::{StCharacterData, StWorldInfoBook, StWorldInfoEntry};

        let mut extra = BTreeMap::new();
        extra.insert("group_only".into(), serde_json::json!(true));
        extra.insert(
            "creator_notes".into(),
            serde_json::json!("unknown data-level ST field"),
        );
        extra.insert(
            "custom_nested".into(),
            serde_json::json!({"flag": "keep-me"}),
        );
        let mut book_extra = BTreeMap::new();
        book_extra.insert("name".into(), serde_json::json!("PNG Opaque Book"));
        book_extra.insert(
            "description".into(),
            serde_json::json!("book-level metadata should survive PNG export"),
        );
        book_extra.insert("scan_depth".into(), serde_json::json!(11));
        book_extra.insert(
            "extensions".into(),
            serde_json::json!({"book_plugin": {"enabled": true}}),
        );

        let data = StCharacterData {
            name: "V3 PNG Fidelity".into(),
            description: "export should keep opaque ST fields".into(),
            personality: "steady".into(),
            scenario: "edge case lab".into(),
            first_mes: "hello".into(),
            mes_example: String::new(),
            system_prompt: "system stays".into(),
            post_history_instructions: "post stays".into(),
            tags: vec!["compat".into(), "png".into()],
            creator: "StoryForge".into(),
            character_version: "7".into(),
            alternate_greetings: vec!["alt 1".into(), "alt 2".into()],
            extensions: serde_json::json!({
                "unknown_plugin": {
                    "state": 42,
                    "nested": {"flag": true}
                },
                "assets": {
                    "html": "<main>card</main>",
                    "css": "main { color: red; }",
                    "js": "window.card = true;"
                }
            }),
            character_book: Some(StWorldInfoBook {
                entries: vec![StWorldInfoEntry {
                    id: Some(7),
                    keys: vec!["primary".into()],
                    key_alias: None,
                    secondary_keys: Some(vec!["secondary".into()]),
                    keysecondary_alias: None,
                    content: Some("book entry".into()),
                    constant: false,
                    selective: true,
                    selective_logic: Some(1),
                    position: Some(serde_json::json!("after_char")),
                    disable: Some(false),
                    order: Some(13),
                    depth: Some(5),
                    extensions: serde_json::json!({"entry_extra": {"rank": 9}}),
                    extra: Default::default(),
                }],
                extra: book_extra,
            }),
            extra,
        };
        let card = make_st_card(data, "3.0");

        let png_bytes = write_st_card_png(&card, None).expect("write_png should succeed");
        let chunks = parse_png(&png_bytes).expect("exported PNG should parse");
        let chara_text = chunks
            .iter()
            .find_map(|chunk| match chunk {
                PngChunk::Text { keyword, text } if keyword == "chara" => Some(text.as_str()),
                _ => None,
            })
            .expect("exported PNG should contain chara text chunk");

        let json_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chara_text)
                .expect("chara payload should be base64");
        let exported: StCharacterCard =
            serde_json::from_slice(&json_bytes).expect("chara payload should be ST JSON");
        let exported_json = serde_json::to_value(&exported.data).unwrap();

        assert_eq!(exported.spec, Some("chara_card_v2".into()));
        assert_eq!(exported.spec_version, Some("3.0".into()));
        assert_eq!(exported.data.name, "V3 PNG Fidelity");
        assert_eq!(exported.data.alternate_greetings, vec!["alt 1", "alt 2"]);
        assert_eq!(exported.data.extensions["unknown_plugin"]["state"], 42);
        assert_eq!(
            exported.data.extensions["assets"]["js"],
            "window.card = true;"
        );
        assert_eq!(exported_json["group_only"], true);
        assert_eq!(
            exported_json["creator_notes"],
            "unknown data-level ST field"
        );
        assert_eq!(exported_json["custom_nested"]["flag"], "keep-me");

        let book = exported
            .data
            .character_book
            .expect("character_book should round-trip in chara payload");
        assert_eq!(book.entries.len(), 1);
        let entry = &book.entries[0];
        assert_eq!(entry.id, Some(7));
        assert_eq!(entry.secondary_keys, Some(vec!["secondary".into()]));
        assert_eq!(entry.selective_logic, Some(1));
        assert_eq!(entry.position, Some(serde_json::json!("after_char")));
        assert_eq!(entry.extensions["entry_extra"]["rank"], 9);
        assert_eq!(book.extra["name"], "PNG Opaque Book");
        assert_eq!(
            book.extra["description"],
            "book-level metadata should survive PNG export"
        );
        assert_eq!(book.extra["scan_depth"], 11);
        assert_eq!(book.extra["extensions"]["book_plugin"]["enabled"], true);
    }

    #[test]
    fn test_write_st_card_png_with_base_image() {
        use storyforge_domain::character::StCharacterData;

        let data = StCharacterData {
            name: "带底图".into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            extensions: serde_json::json!({}),
            character_book: None,
            extra: Default::default(),
        };
        let card = make_st_card(data, "2.0");
        let base = build_placeholder_png();

        let png_bytes = write_st_card_png(&card, Some(&base)).expect("write_png 失败");
        let chunks = parse_png(&png_bytes).expect("解析失败");
        let has_chara = chunks
            .iter()
            .any(|c| matches!(c, PngChunk::Text { keyword, .. } if keyword == "chara"));
        assert!(has_chara, "应有 chara tEXt 块");
    }

    // Build a raw PNG chunk (length BE + type + data + crc) for hostile-input tests.
    fn raw_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + data.len());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(chunk_type);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(chunk_type.len() + data.len());
        crc_input.extend_from_slice(chunk_type);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32fast::hash(&crc_input).to_be_bytes());
        out
    }

    fn minimal_png_with_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        png.extend_from_slice(&raw_chunk(
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0],
        ));
        png.extend_from_slice(&raw_chunk(chunk_type, data));
        png.extend_from_slice(&raw_chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn parse_png_rejects_chunk_with_bad_crc() {
        let mut png = minimal_png_with_chunk(b"tEXt", b"chara\0payload");
        // Corrupt the CRC of the tEXt chunk (last 4 bytes before IEND).
        // PNG = sig(8) + IHDR(25) + tEXt(12+payload) ... corrupt final crc byte.
        let crc_pos = png.len() - 4 - 12; // 12 = IEND chunk total (4+4+0+4)
        png[crc_pos] ^= 0xFF;
        let err = parse_png(&png).expect_err("bad CRC must be rejected");
        assert!(matches!(err, ImportError::PngError(_)));
    }

    #[test]
    fn parse_png_rejects_oversized_single_chunk() {
        // Declare a 70 MiB chunk body but provide only a few bytes.
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        png.extend_from_slice(&raw_chunk(
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0],
        ));
        // length = 70 * 1024 * 1024 (> 64 MiB MAX_CHUNK_SIZE)
        png.extend_from_slice(&(70u32 * 1024 * 1024).to_be_bytes());
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(b"truncated-body");
        let err = parse_png(&png).expect_err("oversized chunk must be rejected");
        assert!(matches!(err, ImportError::PngError(_)));
    }

    #[test]
    fn parse_png_rejects_truncated_chunk_body() {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        png.extend_from_slice(&raw_chunk(
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0],
        ));
        // Declare a 1000-byte tEXt body but give only 5 bytes (no CRC either).
        png.extend_from_slice(&1000u32.to_be_bytes());
        png.extend_from_slice(b"tEXt");
        png.extend_from_slice(b"short");
        let err = parse_png(&png).expect_err("truncated chunk body must be rejected");
        assert!(matches!(err, ImportError::PngError(_)));
    }

    #[test]
    fn parse_png_stops_cleanly_on_dangling_partial_header() {
        // A valid PNG followed by a few trailing bytes (partial next chunk
        // header) must not error: the loop guard stops at pos+8 > len and
        // returns the chunks collected so far.
        let mut png = minimal_png_with_chunk(b"IDAT", &[0x78, 0x01, 0x01]);
        png.extend_from_slice(&[0u8; 3]); // dangling 3 bytes (< 8 header)
        let chunks = parse_png(&png).expect("dangling partial header should not error");
        assert!(!chunks.is_empty(), "should still return collected chunks");
    }
}
