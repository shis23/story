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
use harness_real_llm::forge_differential::{diff_worldbook, load_forge_state};

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
        ran += 1;
    }

    assert!(ran > 0, "STORYFORGE_FORGE_OUT 已设但没有任何组合可跑");
}
