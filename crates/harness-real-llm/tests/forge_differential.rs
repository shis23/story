//! forge 差分预言机（确定性，无 LLM，无网络）。
//!
//! 前置：用社区 forge CLI 把两张验收卡 unpack 到某目录（命令见
//! `src/forge_differential.rs` 模块头），然后：
//! ```text
//! STORYFORGE_FORGE_OUT=<out> cargo test -p harness-real-llm --test forge_differential -- --nocapture
//! ```
//! 未设 `STORYFORGE_FORGE_OUT` 或卡文件缺失时跳过（不 fail）。

use std::path::PathBuf;

use harness_real_llm::card_translation::{load_card, repo_root};
use harness_real_llm::forge_differential::{
    diff_contents, diff_sections, diff_worldbook, find_initvar_yaml, flatten_initvar_yaml,
    load_forge_state, schema_alignment,
};

#[test]
fn forge_differential_worldbook_semantics() {
    let Some(out_dir) = std::env::var_os("STORYFORGE_FORGE_OUT") else {
        eprintln!("skip: STORYFORGE_FORGE_OUT 未设（forge unpack 产物目录）");
        return;
    };
    let out_dir = PathBuf::from(out_dir);

    let combos = [
        ("命定之诗", "test-card.png", "destiny"),
        ("卿卿", "卿卿 (33).png", "qingqing"),
    ];

    let mut ran = 0usize;
    for (label, card_file, forge_sub) in combos {
        let card_path = repo_root().join(card_file);
        if !card_path.exists() {
            eprintln!("skip {label}: 卡文件缺失 {}", card_path.display());
            continue;
        }
        let forge_dir = out_dir.join(forge_sub);
        if !forge_dir.join("tavern-cards-state.json").exists() {
            eprintln!("skip {label}: forge 产物缺失 {}", forge_dir.display());
            continue;
        }

        let character = load_card(&card_path).expect("导入失败");
        let forge = load_forge_state(&forge_dir).expect("读 forge state 失败");
        let report = diff_worldbook(label, character.embedded_world_info.as_ref(), &forge);

        eprintln!(
            "── {label} 差分报告 ──\n{}",
            serde_json::to_string_pretty(&report).unwrap()
        );

        // 硬不变量：名字集合互覆盖（trim 归一化后无缺失）+ 一对一组零启停分歧。
        // 总数允许差 = 重名收缩（ST 允许重名条目，forge manifest 键唯一会吞并）。
        assert!(
            report.hard_invariants_hold(),
            "{label}: 硬不变量不成立（missing_in_ours={:?}，missing_in_forge={:?}，启停分歧={:?}）",
            report.missing_in_ours,
            report.missing_in_forge,
            report.enabled_mismatches
        );
        // 名字覆盖：绝大多数条目应能按名对上（留 5% 未命名/改名余量）
        assert!(
            report.matched_by_name * 100 >= report.forge_total * 95,
            "{label}: 按名匹配率过低 {}/{}",
            report.matched_by_name,
            report.forge_total
        );
        // 结构字段：路由 / 排序 / 主键无分歧
        assert!(
            report.route_mismatches.is_empty(),
            "{label}: 路由分歧 {:?}",
            report.route_mismatches
        );
        assert!(
            report.order_mismatches.is_empty(),
            "{label}: 排序分歧 {:?}",
            report.order_mismatches
        );
        assert!(
            report.keys_mismatches.is_empty(),
            "{label}: 主键分歧 {:?}",
            report.keys_mismatches
        );

        // V2-3：开场白 / 正则 / tavern_helper 清单对照（计数硬断言，名字报告）
        let sections = diff_sections(label, &character, &forge_dir, &forge);
        eprintln!(
            "── {label} section 清单 ──\n{}",
            serde_json::to_string_pretty(&sections).unwrap()
        );
        assert!(
            sections.counts_match(),
            "{label}: section 计数不一致（非空开场白 {}/{}，正则去重 {}/{}，脚本去重 {}/{}）",
            sections.ours_greetings,
            sections.forge_greetings,
            sections.ours_regex_unique,
            sections.forge_regex,
            sections.ours_th_unique,
            sections.forge_th_scripts
        );
        // V2-1：正文级对照（一对一组逐条归一化等值）
        let contents = diff_contents(
            label,
            character.embedded_world_info.as_ref(),
            &forge,
            &forge_dir,
        );
        eprintln!(
            "── {label} 正文对照 ──\n{}",
            serde_json::to_string_pretty(&contents).unwrap()
        );
        assert_eq!(
            contents.missing_file, 0,
            "{label}: forge manifest 声明的正文文件缺失 {} 个",
            contents.missing_file
        );
        assert!(
            contents.mismatched.is_empty(),
            "{label}: 正文不一致 {:?}（{} 比较 / {} 一致）",
            contents.mismatched,
            contents.compared,
            contents.matched
        );

        ran += 1;
    }

    assert!(ran > 0, "STORYFORGE_FORGE_OUT 已设但没有任何组合可跑");
}

/// V2-2：翻译 schema 对 InitVar ground truth 的覆盖率/幻觉率评测。
///
/// 额外前置：`STORYFORGE_CT_EVIDENCE_DIR` 指向验收证据目录
/// （artifacts/card-translation，含 schema_keys_sample）。评测性质：
/// 打印各模型分数；硬断言仅（a）ground truth 解析健全（b）幻觉率 ≤ 50%
/// ——分数解读交给人，阈值防的是彻底脱轨。
#[test]
fn forge_schema_alignment_scores_translations() {
    let Some(out_dir) = std::env::var_os("STORYFORGE_FORGE_OUT") else {
        eprintln!("skip: STORYFORGE_FORGE_OUT 未设");
        return;
    };
    let Some(evidence_dir) = std::env::var_os("STORYFORGE_CT_EVIDENCE_DIR") else {
        eprintln!("skip: STORYFORGE_CT_EVIDENCE_DIR 未设（验收证据目录）");
        return;
    };
    let out_dir = PathBuf::from(out_dir);
    let evidence_dir = PathBuf::from(evidence_dir);

    let combos = [("命定之诗", "destiny"), ("卿卿", "qingqing")];
    let mut ran = 0usize;
    for (label, forge_sub) in combos {
        let forge_dir = out_dir.join(forge_sub);
        let Some(initvar_path) = find_initvar_yaml(&forge_dir) else {
            eprintln!("skip {label}: 世界书目录无 initvar yaml");
            continue;
        };
        let yaml = std::fs::read_to_string(&initvar_path).expect("读 initvar yaml 失败");
        let author = flatten_initvar_yaml(&yaml).expect("initvar yaml 解析失败");
        assert!(
            author.len() >= 20,
            "{label}: ground truth 叶数异常少（{}），解析可能失败",
            author.len()
        );

        // 遍历该卡的所有模型证据文件
        let Ok(entries) = std::fs::read_dir(&evidence_dir) else {
            eprintln!("skip {label}: 证据目录不可读");
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let Some(name) = name else { continue };
            if !name.starts_with(label) || !name.ends_with(".json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).expect("读证据失败");
            let evidence: serde_json::Value = serde_json::from_str(&raw).expect("证据 JSON 解析");
            let model = evidence["model"].as_str().unwrap_or("?").to_string();
            let keys: Vec<String> = evidence["stats"]["schema_keys_sample"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if keys.is_empty() {
                continue;
            }
            let total_fields = evidence["stats"]["schema_fields"].as_u64().unwrap_or(0) as usize;
            let keys_are_sample = keys.len() < total_fields;

            let report = schema_alignment(label, &model, &keys, &author, keys_are_sample);
            eprintln!(
                "── {label} × {model} schema 对齐 ──\n{}",
                serde_json::to_string_pretty(&report).unwrap()
            );
            assert!(
                report.hallucination_pct <= 50.0,
                "{label}×{model}: 幻觉率脱轨 {:.1}%（样例 {:?}）",
                report.hallucination_pct,
                report.hallucinated
            );
            ran += 1;
        }
    }
    assert!(ran > 0, "环境已设但没有任何证据文件可评");
}
