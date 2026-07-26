-- MVU 翻译缓存（卡级玩法翻译产物）。
-- payload_json = tauri-app StoredMvuTranslation 的全量 JSON（含 translation 树）；
-- source_character_id / character_name 冗余成列供索引与列表查询。
-- 此前 SQLite 后端没有该权威 → 后处理【卡片变量更新规则】注入、MVU fallback
-- 片段、前端 MVU 状态面板在 SQLite 下全部空转（V22 补齐）。
CREATE TABLE IF NOT EXISTS mvu_translations (
    source_character_id TEXT PRIMARY KEY,
    character_name TEXT NOT NULL DEFAULT '',
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT ''
);
