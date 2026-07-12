//! Store contract 测试骨架。
//!
//! 本阶段不切换默认 JSON Store，只验证 SQLite 侧能承载
//! Campaign / Conversation / Turn 的最小读写契约，作为后续适配基线。

use serde_json::Value;

use crate::connection::Database;
use crate::error::Result;
use crate::importer;
use crate::migrations;
use crate::unit_of_work::UnitOfWork;

/// 最小 Campaign 读写契约（SQLite 适配验证）。
pub struct SqliteCampaignContract;

impl SqliteCampaignContract {
    pub fn save_campaign(db: &mut Database, campaign: &Value) -> Result<()> {
        migrations::migrate(db)?;
        // 确保 card 存在以满足 FK
        if let Some(card_id) = campaign.get("card_id").and_then(|v| v.as_str()) {
            ensure_card(db, card_id)?;
        }
        let uow = UnitOfWork::begin(db.connection_mut())?;
        {
            let tx = uow.transaction()?;
            crate::importer::upsert_campaign_for_contract(tx, campaign)?;
        }
        uow.commit()?;
        Ok(())
    }

    pub fn get_campaign(db: &Database, campaign_id: &str) -> Result<Option<Value>> {
        importer::load_payload(db, "campaigns", "campaign_id", campaign_id)
    }
}

fn ensure_card(db: &mut Database, card_id: &str) -> Result<()> {
    let exists: i64 = db.connection().query_row(
        "SELECT COUNT(*) FROM character_cards WHERE card_id = ?1",
        [card_id],
        |r| r.get(0),
    )?;
    if exists > 0 {
        return Ok(());
    }
    let uow = UnitOfWork::begin(db.connection_mut())?;
    uow.execute(
        r#"
        INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json)
        VALUES (?1, NULL, 'contract-placeholder', NULL, '{}')
        "#,
        rusqlite::params![card_id],
    )?;
    uow.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Database;
    use crate::importer::JsonImporter;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn campaign_contract_save_get_roundtrip() {
        let mut db = Database::open_in_memory().unwrap();
        let campaign = json!({
            "id": "camp-contract",
            "card_id": "card-contract",
            "name": "Contract Campaign",
            "created_at": "2026-07-13T00:00:00Z",
            "revision": 3,
            "chronicle_revision": 0,
            "story_clock": "Day 1",
            "variables": []
        });
        SqliteCampaignContract::save_campaign(&mut db, &campaign).unwrap();
        let loaded = SqliteCampaignContract::get_campaign(&db, "camp-contract")
            .unwrap()
            .unwrap();
        assert_eq!(loaded["name"], "Contract Campaign");
        assert_eq!(loaded["revision"], 3);
    }

    #[test]
    fn unit_of_work_can_span_turn_and_campaign_revision() {
        let mut db = Database::open_in_memory().unwrap();
        migrations::migrate(&mut db).unwrap();

        // seed card + campaign + conversation
        {
            let uow = UnitOfWork::begin(db.connection_mut()).unwrap();
            uow.execute(
                "INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json) VALUES ('card-1', NULL, 'C', NULL, '{}')",
                [],
            )
            .unwrap();
            uow.execute(
                "INSERT INTO campaigns (campaign_id, card_id, name, conversation_id, revision, chronicle_revision, lineage_id, story_clock, created_at, payload_json)
                 VALUES ('camp-1', 'card-1', 'Camp', 'conv-1', 0, 0, NULL, 'Day 1', 't', '{\"id\":\"camp-1\",\"revision\":0}')",
                [],
            )
            .unwrap();
            uow.execute(
                "INSERT INTO conversations (conversation_id, campaign_id, character_id, archived_upto, created_at, updated_at, payload_json)
                 VALUES ('conv-1', 'camp-1', NULL, 0, 't', 't', '{}')",
                [],
            )
            .unwrap();
            uow.commit().unwrap();
        }

        // 同一事务：插入 turn + bump campaign revision
        {
            let uow = UnitOfWork::begin(db.connection_mut()).unwrap();
            uow.execute(
                "INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision, status, accepted_attempt_id, failure_reason, created_at, updated_at, payload_json)
                 VALUES ('turn-1', 'camp-1', 'conv-1', 'n1', 0, 'committed', NULL, NULL, 't', 't', '{}')",
                [],
            )
            .unwrap();
            uow.execute(
                "UPDATE campaigns SET revision = 1, payload_json = '{\"id\":\"camp-1\",\"revision\":1}' WHERE campaign_id = 'camp-1'",
                [],
            )
            .unwrap();
            uow.commit().unwrap();
        }

        let revision: i64 = db
            .connection()
            .query_row(
                "SELECT revision FROM campaigns WHERE campaign_id = 'camp-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revision, 1);
        let turns: i64 = importer::table_count(&db, "turns").unwrap();
        assert_eq!(turns, 1);
    }

    #[test]
    fn importer_then_contract_read_is_stable() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("cards.json"),
            serde_json::to_vec_pretty(&json!([{
                "card": {"id": "card-1", "name": "N", "source_character_id": "c", "character_definitions": []},
                "imported_at": "t"
            }]))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("campaigns.json"),
            serde_json::to_vec_pretty(&json!([{
                "id": "camp-1",
                "card_id": "card-1",
                "name": "Imported",
                "created_at": "t",
                "revision": 5,
                "story_clock": "Day 1",
                "variables": []
            }]))
            .unwrap(),
        )
        .unwrap();

        let mut db = Database::open_in_memory().unwrap();
        JsonImporter::new(&mut db).import_data_dir(root).unwrap();
        let loaded = SqliteCampaignContract::get_campaign(&db, "camp-1")
            .unwrap()
            .unwrap();
        assert_eq!(loaded["revision"], 5);
        assert_eq!(loaded["name"], "Imported");
    }
}
