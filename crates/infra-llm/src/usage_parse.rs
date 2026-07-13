//! Provider usage 解析（OpenAI / DeepSeek / Anthropic 兼容）。
//!
//! 流式与非流式共用同一字段语义，避免 SSE 嵌套 cache 漏解析。
//! 本模块只做 schema 归一化，**不**宣称供应商 cache 命中。

use storyforge_domain::llm::Usage;

/// 从供应商 usage JSON 解析统一 [`Usage`]。
///
/// - 缺少 `prompt_tokens` / `completion_tokens` 且无法安全恢复 → `None`
/// - 畸形数值（字符串、null、负数）按字段降级为 0；若核心字段全不可用 → `None`
/// - 缓存字段缺失视为 0，不失败
pub fn parse_provider_usage(usage: &serde_json::Value) -> Option<Usage> {
    if !usage.is_object() {
        return None;
    }

    let prompt_tokens = read_u32_field(usage, "prompt_tokens")?;
    let completion_tokens = read_u32_field(usage, "completion_tokens").unwrap_or(0);
    let total_tokens = read_u32_field(usage, "total_tokens")
        .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));

    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens: parse_cached_tokens(usage),
        cache_creation_tokens: parse_cache_creation_tokens(usage),
    })
}

/// 解析缓存命中 token（多厂商字段兼容）。
///
/// - DeepSeek: `prompt_cache_hit_tokens`（顶层，仅正值优先）
/// - OpenAI: `prompt_tokens_details.cached_tokens`
/// - Anthropic: `cache_read_input_tokens`
///
/// 正值优先避免流式路径把缺省 `0` 字段误当成最终结果，掩盖嵌套 OpenAI 字段。
pub fn parse_cached_tokens(usage: &serde_json::Value) -> u32 {
    if let Some(v) = read_u32_field(usage, "prompt_cache_hit_tokens")
        && v > 0
    {
        return v;
    }
    if let Some(details) = usage.get("prompt_tokens_details")
        && let Some(v) = read_u32_field(details, "cached_tokens")
        && v > 0
    {
        return v;
    }
    if let Some(v) = read_u32_field(usage, "cache_read_input_tokens")
        && v > 0
    {
        return v;
    }
    0
}

/// 解析缓存创建 token（多厂商字段兼容）。
///
/// - DeepSeek: `prompt_cache_miss_tokens`（顶层，仅正值优先）
/// - Anthropic: `cache_creation_input_tokens`
pub fn parse_cache_creation_tokens(usage: &serde_json::Value) -> u32 {
    if let Some(v) = read_u32_field(usage, "prompt_cache_miss_tokens")
        && v > 0
    {
        return v;
    }
    if let Some(v) = read_u32_field(usage, "cache_creation_input_tokens")
        && v > 0
    {
        return v;
    }
    0
}

fn read_u32_field(obj: &serde_json::Value, key: &str) -> Option<u32> {
    let v = obj.get(key)?;
    match v {
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                u32::try_from(u).ok()
            } else if let Some(i) = n.as_i64() {
                if i < 0 {
                    Some(0)
                } else {
                    u32::try_from(i).ok()
                }
            } else {
                None
            }
        }
        serde_json::Value::String(s) => s.trim().parse::<u32>().ok(),
        serde_json::Value::Null => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Case {
        name: &'static str,
        json: serde_json::Value,
        expect: Option<Usage>,
    }

    fn usage(prompt: u32, completion: u32, total: u32, cached: u32, creation: u32) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cached_tokens: cached,
            cache_creation_tokens: creation,
        }
    }

    fn matrix() -> Vec<Case> {
        vec![
            Case {
                name: "openai_nested_cached_non_stream",
                json: serde_json::json!({
                    "prompt_tokens": 215,
                    "completion_tokens": 35,
                    "total_tokens": 250,
                    "prompt_tokens_details": {"cached_tokens": 128}
                }),
                expect: Some(usage(215, 35, 250, 128, 0)),
            },
            Case {
                name: "deepseek_top_level_hit_miss",
                json: serde_json::json!({
                    "prompt_tokens": 1000,
                    "completion_tokens": 10,
                    "total_tokens": 1010,
                    "prompt_cache_hit_tokens": 800,
                    "prompt_cache_miss_tokens": 200
                }),
                expect: Some(usage(1000, 10, 1010, 800, 200)),
            },
            Case {
                name: "anthropic_cache_fields",
                json: serde_json::json!({
                    "prompt_tokens": 500,
                    "completion_tokens": 20,
                    "total_tokens": 520,
                    "cache_read_input_tokens": 300,
                    "cache_creation_input_tokens": 50
                }),
                expect: Some(usage(500, 20, 520, 300, 50)),
            },
            Case {
                name: "deepseek_preferred_over_openai_nested",
                json: serde_json::json!({
                    "prompt_tokens": 1000,
                    "completion_tokens": 10,
                    "prompt_cache_hit_tokens": 800,
                    "prompt_cache_miss_tokens": 200,
                    "prompt_tokens_details": {"cached_tokens": 1}
                }),
                // total 缺省时用 prompt+completion
                expect: Some(usage(1000, 10, 1010, 800, 200)),
            },
            Case {
                name: "missing_usage_object_fields",
                json: serde_json::json!({}),
                expect: None,
            },
            Case {
                name: "missing_prompt_tokens",
                json: serde_json::json!({
                    "completion_tokens": 10,
                    "total_tokens": 10
                }),
                expect: None,
            },
            Case {
                name: "partial_stream_chunk_usage_with_prompt_only_completion_default_0",
                json: serde_json::json!({
                    "prompt_tokens": 90,
                    "prompt_tokens_details": {"cached_tokens": 40}
                }),
                expect: Some(usage(90, 0, 90, 40, 0)),
            },
            Case {
                name: "final_stream_chunk_full_usage",
                json: serde_json::json!({
                    "prompt_tokens": 400,
                    "completion_tokens": 80,
                    "total_tokens": 480,
                    "prompt_tokens_details": {"cached_tokens": 320}
                }),
                expect: Some(usage(400, 80, 480, 320, 0)),
            },
            Case {
                name: "malformed_string_numbers_accepted",
                json: serde_json::json!({
                    "prompt_tokens": "120",
                    "completion_tokens": "8",
                    "total_tokens": "128",
                    "prompt_cache_hit_tokens": "96"
                }),
                expect: Some(usage(120, 8, 128, 96, 0)),
            },
            Case {
                name: "malformed_null_cache_defaults_zero",
                json: serde_json::json!({
                    "prompt_tokens": 50,
                    "completion_tokens": 5,
                    "total_tokens": 55,
                    "prompt_cache_hit_tokens": null,
                    "prompt_tokens_details": {"cached_tokens": null}
                }),
                expect: Some(usage(50, 5, 55, 0, 0)),
            },
            Case {
                name: "malformed_negative_tokens_clamp_zero_for_cache",
                json: serde_json::json!({
                    "prompt_tokens": 50,
                    "completion_tokens": 5,
                    "prompt_cache_hit_tokens": -3
                }),
                expect: Some(usage(50, 5, 55, 0, 0)),
            },
            Case {
                name: "non_object_usage",
                json: serde_json::json!("not-an-object"),
                expect: None,
            },
            Case {
                name: "missing_total_filled_from_prompt_plus_completion",
                json: serde_json::json!({
                    "prompt_tokens": 11,
                    "completion_tokens": 2
                }),
                expect: Some(usage(11, 2, 13, 0, 0)),
            },
            Case {
                name: "explicit_total_zero_preserved",
                json: serde_json::json!({
                    "prompt_tokens": 11,
                    "completion_tokens": 2,
                    "total_tokens": 0
                }),
                expect: Some(usage(11, 2, 0, 0, 0)),
            },
            Case {
                name: "zero_cached_is_valid_not_success_condition",
                json: serde_json::json!({
                    "prompt_tokens": 10,
                    "completion_tokens": 1,
                    "total_tokens": 11,
                    "prompt_tokens_details": {"cached_tokens": 0}
                }),
                expect: Some(usage(10, 1, 11, 0, 0)),
            },
        ]
    }

    #[test]
    fn usage_parser_matrix_covers_provider_schemas() {
        for case in matrix() {
            let got = parse_provider_usage(&case.json);
            assert_eq!(
                got.as_ref().map(|u| (
                    u.prompt_tokens,
                    u.completion_tokens,
                    u.total_tokens,
                    u.cached_tokens,
                    u.cache_creation_tokens
                )),
                case.expect.as_ref().map(|u| (
                    u.prompt_tokens,
                    u.completion_tokens,
                    u.total_tokens,
                    u.cached_tokens,
                    u.cache_creation_tokens
                )),
                "case {} failed: got={got:?} expect={:?}",
                case.name,
                case.expect
            );
        }
    }

    #[test]
    fn zero_cached_is_not_treated_as_vacuous_success() {
        // 防止 `0 >= 0` 式 cache 成功条件：解析出 0 只表示“无命中字段/未命中”，
        // 调用方必须另有 prompt_tokens > 0 与样本分类，不能把 cached==0 当通过。
        let u = parse_provider_usage(&serde_json::json!({
            "prompt_tokens": 42,
            "completion_tokens": 1,
            "total_tokens": 43
        }))
        .expect("usage present");
        assert!(u.prompt_tokens > 0);
        assert_eq!(
            u.cached_tokens, 0,
            "zero cached must not satisfy positive cache signal"
        );
    }
}
