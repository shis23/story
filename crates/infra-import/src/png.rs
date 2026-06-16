use crate::ImportError;

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
        let length = u32::from_be_bytes([
            data[pos],
            data[pos + 1],
            data[pos + 2],
            data[pos + 3],
        ]) as usize;

        // H-7 防护：单 chunk 大小上限（64MB）。恶意 PNG 可声明超大 tEXt 块吃光内存
        const MAX_CHUNK_SIZE: usize = 64 * 1024 * 1024;
        if length > MAX_CHUNK_SIZE {
            return Err(ImportError::PngError(format!(
                "PNG 块过大（{} 字节，上限 {} 字节），可能为恶意文件",
                length,
                MAX_CHUNK_SIZE
            )));
        }

        // 读取类型
        let chunk_type = [
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ];

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
