//! Card Studio Phase 2：小说蒸馏流水线骨架（纯 domain，无 LLM/IO）。
//!
//! 方法论来源：明月小说文风蒸馏总结工具（FEASIBILITY §3.4）——
//! 文档切分 → 每块（文风片段报告 + 剧情小结）→ 阶段公式 → 总公式/文风 prompt
//! → 角色/世界观/关系/伏笔账本 → 一键预填 CardProject artifacts。
//!
//! 本模块只提供确定性骨架：切分器、作业状态机、断点续跑语义、账本聚合。
//! LLM 调用（块报告/公式合成）由 tauri-app 命令层驱动，逐块产出经
//! `apply_chunk_result` 写回；进程重启后从持久化的 job 恢复，`next_pending_chunk`
//! 天然就是断点。

use serde::{Deserialize, Serialize};

/// 单个蒸馏块（按字符区间引用原文，不复制文本——长篇小说内存友好）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistillChunk {
    pub index: usize,
    /// 原文字符偏移（char 计数，非字节）
    pub char_start: usize,
    pub char_end: usize,
    /// 块级剧情小结（LLM 产出；None = 未处理）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 块级文风片段报告（LLM 产出）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_report: Option<String>,
}

impl DistillChunk {
    pub fn is_done(&self) -> bool {
        self.summary.is_some() && self.style_report.is_some()
    }
}

/// 蒸馏账本（总结阶段的四类聚合产物）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DistillLedgers {
    #[serde(default)]
    pub characters: Vec<String>,
    #[serde(default)]
    pub world: Vec<String>,
    #[serde(default)]
    pub relations: Vec<String>,
    #[serde(default)]
    pub foreshadow: Vec<String>,
}

/// 蒸馏作业阶段（线性推进；持久化后重启从当前阶段续跑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistillStage {
    /// 逐块处理（summary + style_report）
    Chunks,
    /// 全块完成 → 文风公式合成（阶段公式 → final formula）
    StyleFormula,
    /// 账本聚合（角色/世界观/关系/伏笔）
    Ledgers,
    /// 全部完成，可一键预填 CardProject artifacts
    Done,
}

/// 蒸馏作业：可序列化断点。原文本身不入 job（由 CardProject.novel_text /
/// 外部文件持有），块只存区间与产物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NovelDistillJob {
    pub project_id: String,
    pub stage: DistillStage,
    pub chunks: Vec<DistillChunk>,
    /// 文风总公式（StyleFormula 阶段产出）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_formula: Option<String>,
    /// 可挂写作 profile 的最终文风 prompt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_style_prompt: Option<String>,
    #[serde(default)]
    pub ledgers: DistillLedgers,
}

/// 默认目标块大小（字符）。明月工具经验值量级：单块一章上下。
pub const DEFAULT_CHUNK_CHARS: usize = 6_000;

/// 把小说文本切成目标大小的块，尽量落在段落边界（`\n` 组）上。
///
/// 规则：
/// - 以空行/换行为候选切点；块体积达到 `target_chars` 后在下一个换行切；
/// - 无换行的超长段按硬上限 `target_chars * 2` 强切（防单段吃满内存/预算）；
/// - 返回块区间按 char 偏移（调用方用 `slice_chunk` 取文本）。
pub fn chunk_novel(text: &str, target_chars: usize) -> Vec<DistillChunk> {
    let target = target_chars.max(200);
    let hard_cap = target * 2;
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;

    while cursor < total {
        let len = cursor - start + 1;
        let at_newline = chars[cursor] == '\n';
        let should_cut = (len >= target && at_newline) || len >= hard_cap;
        if should_cut {
            chunks.push(DistillChunk {
                index: chunks.len(),
                char_start: start,
                char_end: cursor + 1,
                summary: None,
                style_report: None,
            });
            start = cursor + 1;
        }
        cursor += 1;
    }
    if start < total {
        chunks.push(DistillChunk {
            index: chunks.len(),
            char_start: start,
            char_end: total,
            summary: None,
            style_report: None,
        });
    }
    chunks
}

/// 取块对应的原文切片（char 偏移安全；越界返回空串防御）。
pub fn slice_chunk(text: &str, chunk: &DistillChunk) -> String {
    text.chars()
        .skip(chunk.char_start)
        .take(chunk.char_end.saturating_sub(chunk.char_start))
        .collect()
}

impl NovelDistillJob {
    /// 从原文新建作业（Chunks 阶段起步）。
    pub fn new(project_id: impl Into<String>, novel_text: &str, target_chars: usize) -> Self {
        Self {
            project_id: project_id.into(),
            stage: DistillStage::Chunks,
            chunks: chunk_novel(novel_text, target_chars),
            style_formula: None,
            final_style_prompt: None,
            ledgers: DistillLedgers::default(),
        }
    }

    /// 断点：下一个未完成块。None = 块阶段完毕（可推进 StyleFormula）。
    pub fn next_pending_chunk(&self) -> Option<&DistillChunk> {
        self.chunks.iter().find(|c| !c.is_done())
    }

    /// 写回单块产物（幂等：重复写覆盖旧值）。块全完后自动推进 StyleFormula。
    /// index 越界或阶段不符返回 Err（调用方持久化前必查）。
    pub fn apply_chunk_result(
        &mut self,
        index: usize,
        summary: String,
        style_report: String,
    ) -> Result<(), String> {
        if self.stage != DistillStage::Chunks {
            return Err(format!("当前阶段 {:?} 不接受块产物", self.stage));
        }
        let total = self.chunks.len();
        let chunk = self
            .chunks
            .get_mut(index)
            .ok_or_else(|| format!("块 #{index} 不存在（共 {total} 块）"))?;
        chunk.summary = Some(summary);
        chunk.style_report = Some(style_report);
        if self.next_pending_chunk().is_none() {
            self.stage = DistillStage::StyleFormula;
        }
        Ok(())
    }

    /// 写回文风公式与最终 prompt，推进 Ledgers。
    pub fn apply_style_formula(
        &mut self,
        formula: String,
        final_prompt: String,
    ) -> Result<(), String> {
        if self.stage != DistillStage::StyleFormula {
            return Err(format!("当前阶段 {:?} 不接受文风公式", self.stage));
        }
        self.style_formula = Some(formula);
        self.final_style_prompt = Some(final_prompt);
        self.stage = DistillStage::Ledgers;
        Ok(())
    }

    /// 写回账本，作业完成。
    pub fn apply_ledgers(&mut self, ledgers: DistillLedgers) -> Result<(), String> {
        if self.stage != DistillStage::Ledgers {
            return Err(format!("当前阶段 {:?} 不接受账本", self.stage));
        }
        self.ledgers = ledgers;
        self.stage = DistillStage::Done;
        Ok(())
    }

    /// 进度（块完成数 / 总数；后续阶段按 1 块计权重简化显示）。
    pub fn progress(&self) -> (usize, usize) {
        let done = self.chunks.iter().filter(|c| c.is_done()).count();
        (done, self.chunks.len())
    }

    /// Done 后的一键预填素材（notes/style/世界观账本行；由命令层写入
    /// CardProject artifacts——保持 domain 无 CardProject 依赖方向的自由）。
    pub fn prefill_material(&self) -> Option<DistillPrefill> {
        if self.stage != DistillStage::Done {
            return None;
        }
        Some(DistillPrefill {
            style_prompt: self.final_style_prompt.clone().unwrap_or_default(),
            character_notes: self.ledgers.characters.clone(),
            world_notes: self.ledgers.world.clone(),
            relation_notes: self.ledgers.relations.clone(),
            foreshadow_notes: self.ledgers.foreshadow.clone(),
            chapter_summaries: self
                .chunks
                .iter()
                .filter_map(|c| c.summary.clone())
                .collect(),
        })
    }
}

/// 蒸馏完成后的预填素材包。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistillPrefill {
    pub style_prompt: String,
    pub character_notes: Vec<String>,
    pub world_notes: Vec<String>,
    pub relation_notes: Vec<String>,
    pub foreshadow_notes: Vec<String>,
    pub chapter_summaries: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_respects_paragraph_boundaries_and_hard_cap() {
        // 5 段 × 300 字符 + 换行；target 600 → 每 2 段附近切一刀
        let para = "汉".repeat(300);
        let text = (0..5).map(|_| para.clone()).collect::<Vec<_>>().join("\n");
        let chunks = chunk_novel(&text, 600);
        assert!(chunks.len() >= 2, "应切成多块: {}", chunks.len());
        // 区间连续无缝且覆盖全文
        let total: usize = text.chars().count();
        assert_eq!(chunks.first().unwrap().char_start, 0);
        assert_eq!(chunks.last().unwrap().char_end, total);
        for w in chunks.windows(2) {
            assert_eq!(w[0].char_end, w[1].char_start, "块必须无缝连续");
        }
        // 无换行超长段：硬上限强切
        let long = "字".repeat(5_000);
        let hard = chunk_novel(&long, 1_000);
        assert!(hard.len() >= 2, "无换行超长段必须硬切: {}", hard.len());
        assert!(hard.iter().all(|c| c.char_end - c.char_start <= 2_000));
        // 空文本
        assert!(chunk_novel("", 1_000).is_empty());
    }

    #[test]
    fn slice_chunk_round_trips_char_ranges() {
        let text = "第一段。\n第二段带一些字。\n第三段。";
        let chunks = chunk_novel(text, 5);
        let joined: String = chunks.iter().map(|c| slice_chunk(text, c)).collect();
        assert_eq!(joined, text, "全部块切片拼接必须还原原文");
    }

    #[test]
    fn job_stages_advance_with_checkpoint_semantics() {
        let text = format!("{}\n{}", "甲".repeat(300), "乙".repeat(300));
        let mut job = NovelDistillJob::new("proj-1", &text, 300);
        assert_eq!(job.stage, DistillStage::Chunks);
        let total = job.chunks.len();
        assert!(total >= 2);

        // 断点：逐块推进；重复写覆盖不报错（幂等）
        while let Some(chunk) = job.next_pending_chunk() {
            let idx = chunk.index;
            job.apply_chunk_result(idx, format!("小结{idx}"), format!("文风{idx}"))
                .unwrap();
        }
        assert_eq!(job.stage, DistillStage::StyleFormula);
        assert_eq!(job.progress(), (total, total));

        // 阶段错序拒绝
        assert!(job.apply_ledgers(DistillLedgers::default()).is_err());
        assert!(job.apply_chunk_result(0, "x".into(), "y".into()).is_err());

        job.apply_style_formula("总公式".into(), "文风 prompt".into())
            .unwrap();
        assert_eq!(job.stage, DistillStage::Ledgers);
        job.apply_ledgers(DistillLedgers {
            characters: vec!["主角：守灯人".into()],
            world: vec!["末班车站".into()],
            relations: vec![],
            foreshadow: vec!["灯油将尽".into()],
        })
        .unwrap();
        assert_eq!(job.stage, DistillStage::Done);

        let prefill = job.prefill_material().expect("Done 后应有预填包");
        assert_eq!(prefill.style_prompt, "文风 prompt");
        assert_eq!(prefill.chapter_summaries.len(), total);
        assert_eq!(prefill.character_notes, vec!["主角：守灯人".to_string()]);

        // 序列化断点 round-trip（重启恢复）
        let json = serde_json::to_string(&job).unwrap();
        let restored: NovelDistillJob = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, job);
    }

    #[test]
    fn out_of_range_chunk_write_fails_closed() {
        let mut job = NovelDistillJob::new("proj-2", "短文本", 1_000);
        assert_eq!(job.chunks.len(), 1);
        assert!(job.apply_chunk_result(9, "s".into(), "r".into()).is_err());
    }
}
