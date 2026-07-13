//! Chronicle B/C publication UoW contracts.
//!
//! These tests are intentionally written against the desired typed API before
//! the production implementation lands.

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use storyforge_infra_sqlite::publication::{
    PublishFault, PublishOutcome, PublishRequest, SqliteChronicleRepository,
};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    db: Database,
    campaign_id: Id,
    conversation_id: Id,
    lineage_id: Id,
    leaves: Vec<RoundSummary>,
}

fn open_db() -> (TempDir, Database) {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    (dir, db)
}

fn leaf(
    campaign_id: &Id,
    conversation_id: &Id,
    lineage_id: &Id,
    id: &str,
    turn: u32,
    code: &str,
) -> RoundSummary {
    let mut summary = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        turn,
        format!("leaf body {turn}"),
    )
    .with_code(code)
    .with_headline(format!("h{turn}"))
    .with_lineage(lineage_id.clone());
    summary.id = Id::from_str(id);
    summary.turn_end = turn;
    summary
}

struct ParentSpec<'a> {
    campaign_id: &'a Id,
    conversation_id: &'a Id,
    lineage_id: &'a Id,
    id: &'a str,
    covers: Vec<Id>,
    turn_start: u32,
    turn_end: u32,
    code: &'a str,
    level: u8,
}

fn parent_summary(spec: ParentSpec<'_>) -> RoundSummary {
    let mut summary = RoundSummary::new(
        spec.campaign_id.clone(),
        spec.conversation_id.clone(),
        spec.turn_start,
        format!("parent body {}", spec.code),
    )
    .with_code(spec.code)
    .with_headline(spec.code)
    .with_lineage(spec.lineage_id.clone());
    summary.id = Id::from_str(spec.id);
    summary.level = spec.level;
    summary.turn_end = spec.turn_end;
    summary.covers = spec.covers;
    summary
}

#[allow(clippy::too_many_arguments)]
fn parent_b(
    campaign_id: &Id,
    conversation_id: &Id,
    lineage_id: &Id,
    id: &str,
    covers: Vec<Id>,
    turn_start: u32,
    turn_end: u32,
    code: &str,
) -> RoundSummary {
    parent_summary(ParentSpec {
        campaign_id,
        conversation_id,
        lineage_id,
        id,
        covers,
        turn_start,
        turn_end,
        code,
        level: 1,
    })
}

#[allow(clippy::too_many_arguments)]
fn parent_c(
    campaign_id: &Id,
    conversation_id: &Id,
    lineage_id: &Id,
    id: &str,
    covers: Vec<Id>,
    turn_start: u32,
    turn_end: u32,
    code: &str,
) -> RoundSummary {
    parent_summary(ParentSpec {
        campaign_id,
        conversation_id,
        lineage_id,
        id,
        covers,
        turn_start,
        turn_end,
        code,
        level: 2,
    })
}

fn fixture_with_leaves(count: u32) -> Fixture {
    let (dir, mut db) = open_db();
    let campaign_id = Id::from_str("camp-chronicle");
    let conversation_id = Id::from_str("conv-chronicle");
    let lineage_id = Id::from_str("lin-chronicle");

    let mut campaign = Campaign::new(Id::from_str("card-chronicle"), "Chronicle UoW");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.lineage_id = Some(lineage_id.clone());
    campaign.chronicle_revision = 1;

    let mut conversation = Conversation::new(None, Some(campaign_id.clone()));
    conversation.id = conversation_id.clone();
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let mut leaves = Vec::new();
    for turn in 1..=count {
        let summary = leaf(
            &campaign_id,
            &conversation_id,
            &lineage_id,
            &format!("leaf-{turn}"),
            turn,
            &format!("A{turn:04}"),
        );
        leaves.push(summary.clone());
        SqliteChronicleRepository::seed_summary(&mut db, &summary).unwrap();
    }

    Fixture {
        _dir: dir,
        db,
        campaign_id,
        conversation_id,
        lineage_id,
        leaves,
    }
}

fn a_to_b_request(f: &Fixture) -> (Vec<RoundSummary>, Vec<(Id, Id)>, Id) {
    let left_ids = vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()];
    let right_ids = vec![f.leaves[2].id.clone(), f.leaves[3].id.clone()];
    let parents = vec![
        parent_b(
            &f.campaign_id,
            &f.conversation_id,
            &f.lineage_id,
            "parent-b1",
            left_ids.clone(),
            1,
            2,
            "B0001",
        ),
        parent_b(
            &f.campaign_id,
            &f.conversation_id,
            &f.lineage_id,
            "parent-b2",
            right_ids.clone(),
            3,
            4,
            "B0002",
        ),
    ];
    let child_covered_by = vec![
        (left_ids[0].clone(), parents[0].id.clone()),
        (left_ids[1].clone(), parents[0].id.clone()),
        (right_ids[0].clone(), parents[1].id.clone()),
        (right_ids[1].clone(), parents[1].id.clone()),
    ];
    (parents, child_covered_by, Id::from_str("pub-a-to-b"))
}

fn publish(
    db: &mut Database,
    campaign_id: &Id,
    publication_id: &Id,
    parents: &[RoundSummary],
    child_covered_by: &[(Id, Id)],
) -> storyforge_infra_sqlite::Result<PublishOutcome> {
    // job_id is optional and unique when present; default to publication_id for isolation.
    let job = format!("job-{}", publication_id.as_str());
    SqliteChronicleRepository::publish_compress(
        db,
        PublishRequest {
            campaign_id,
            publication_id,
            parents,
            child_covered_by,
            job_id: Some(job.as_str()),
        },
    )
}

fn assert_pre_publish_state(f: &Fixture) {
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 1);
    assert!(campaign.pending_compress_publication.is_none());
    let summaries = SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id).unwrap();
    assert_eq!(summaries.len(), f.leaves.len());
    assert!(summaries.iter().all(|s| s.covered_by.is_none()));
    assert_eq!(
        SqliteChronicleRepository::count_publication_jobs(&f.db).unwrap(),
        0
    );
}

#[test]
fn publish_a_to_b_commits_parents_covers_revision_and_job() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);

    let outcome = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();
    assert_eq!(outcome, PublishOutcome::Applied);

    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 2);
    assert!(campaign.pending_compress_publication.is_none());
    assert!(campaign.context_epoch.is_none());

    let summaries = SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id).unwrap();
    assert_eq!(summaries.len(), 6);
    assert_eq!(summaries.iter().filter(|s| s.level == 1).count(), 2);
    for (child_id, parent_id) in &child_covered_by {
        let child = summaries.iter().find(|s| s.id == *child_id).unwrap();
        assert_eq!(child.covered_by.as_ref(), Some(parent_id));
    }
    for parent in &parents {
        let stored = summaries.iter().find(|s| s.id == parent.id).unwrap();
        assert_eq!(stored.covers, parent.covers);
        assert_eq!(stored.level, 1);
    }
    assert_eq!(
        SqliteChronicleRepository::count_publication_jobs(&f.db).unwrap(),
        1
    );
}

#[test]
fn publish_b_to_c_requires_existing_stage_children() {
    let mut f = fixture_with_leaves(4);
    let (b_parents, b_covers, b_pub) = a_to_b_request(&f);
    publish(&mut f.db, &f.campaign_id, &b_pub, &b_parents, &b_covers).unwrap();

    let c_parent = parent_c(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-c1",
        b_parents.iter().map(|p| p.id.clone()).collect(),
        1,
        4,
        "C0001",
    );
    let child_covered_by = vec![
        (b_parents[0].id.clone(), c_parent.id.clone()),
        (b_parents[1].id.clone(), c_parent.id.clone()),
    ];
    let publication_id = Id::from_str("pub-b-to-c");
    let outcome = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        std::slice::from_ref(&c_parent),
        &child_covered_by,
    )
    .unwrap();
    assert_eq!(outcome, PublishOutcome::Applied);

    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 3);
    let summaries = SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id).unwrap();
    assert_eq!(summaries.iter().filter(|s| s.level == 2).count(), 1);
    for (child_id, parent_id) in &child_covered_by {
        let child = summaries.iter().find(|s| s.id == *child_id).unwrap();
        assert_eq!(child.covered_by.as_ref(), Some(parent_id));
        assert_eq!(child.level, 1);
    }
}

#[test]
fn repeated_identical_publication_is_noop_replay() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    let first = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();
    assert_eq!(first, PublishOutcome::Applied);

    let second = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();
    assert_eq!(second, PublishOutcome::AlreadyPublished);

    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 2);
    assert_eq!(
        SqliteChronicleRepository::count_publication_jobs(&f.db).unwrap(),
        1
    );
}

#[test]
fn same_publication_id_with_different_payload_is_rejected() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();

    let mut altered = parents.clone();
    altered[0].content = "different payload".into();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &altered,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("payload") || err.to_string().contains("conflict"),
        "{err}"
    );
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 2);
}

#[test]
fn rejects_overlapping_or_non_continuous_covers() {
    let mut f = fixture_with_leaves(4);
    let overlapping = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "bad-overlap",
        vec![f.leaves[0].id.clone(), f.leaves[2].id.clone()],
        1,
        3,
        "B0099",
    )];
    let covers = vec![
        (f.leaves[0].id.clone(), overlapping[0].id.clone()),
        (f.leaves[2].id.clone(), overlapping[0].id.clone()),
    ];
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-overlap"),
        &overlapping,
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("cover")
            || err.to_string().contains("contiguous")
            || err.to_string().contains("continuous"),
        "{err}"
    );
    assert_pre_publish_state(&f);
}

#[test]
fn rejects_wrong_lineage_parent_level_and_turn_span() {
    let mut f = fixture_with_leaves(2);

    let mut wrong_lineage = parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &Id::from_str("other-lineage"),
        "bad-lineage",
        vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()],
        1,
        2,
        "B0001",
    );
    let covers = vec![
        (f.leaves[0].id.clone(), wrong_lineage.id.clone()),
        (f.leaves[1].id.clone(), wrong_lineage.id.clone()),
    ];
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-lineage"),
        &[wrong_lineage.clone()],
        &covers,
    )
    .unwrap_err();
    assert!(err.to_string().contains("lineage"), "{err}");

    wrong_lineage.lineage_id = Some(f.lineage_id.clone());
    wrong_lineage.level = 2; // C cannot cover A directly
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-level"),
        &[wrong_lineage.clone()],
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("level") || err.to_string().contains("child"),
        "{err}"
    );

    wrong_lineage.level = 1;
    wrong_lineage.turn = 1;
    wrong_lineage.turn_end = 9;
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-span"),
        &[wrong_lineage],
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("turn") || err.to_string().contains("span"),
        "{err}"
    );
    assert_pre_publish_state(&f);
}

#[test]
fn same_parent_identity_with_different_payload_is_rejected() {
    let mut f = fixture_with_leaves(2);
    let parents = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-stable",
        vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()],
        1,
        2,
        "B0001",
    )];
    let covers = vec![
        (f.leaves[0].id.clone(), parents[0].id.clone()),
        (f.leaves[1].id.clone(), parents[0].id.clone()),
    ];
    publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-first"),
        &parents,
        &covers,
    )
    .unwrap();

    // Second publication reuses parent id with different content under a new publication id.
    let mut conflict = parents.clone();
    conflict[0].content = "rewritten stage body".into();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-second"),
        &conflict,
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("payload") || err.to_string().contains("conflict"),
        "{err}"
    );
}

fn fault_leaves_zero_side_effects(fault: PublishFault) {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    let err = SqliteChronicleRepository::publish_compress_with_fault(
        &mut f.db,
        PublishRequest {
            campaign_id: &f.campaign_id,
            publication_id: &publication_id,
            parents: &parents,
            child_covered_by: &child_covered_by,
            job_id: Some("job-fault"),
        },
        fault,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected"), "{err}");
    assert_pre_publish_state(&f);
}

#[test]
fn fault_after_parent_insert_rolls_back_publication() {
    fault_leaves_zero_side_effects(PublishFault::AfterParentInsert);
}

#[test]
fn fault_after_child_update_rolls_back_publication() {
    fault_leaves_zero_side_effects(PublishFault::AfterChildUpdate);
}

#[test]
fn fault_after_revision_bump_rolls_back_publication() {
    fault_leaves_zero_side_effects(PublishFault::AfterRevisionBump);
}

#[test]
fn fault_after_marker_cleanup_rolls_back_publication() {
    fault_leaves_zero_side_effects(PublishFault::AfterMarkerCleanup);
}

#[test]
fn existing_turn_accept_uow_still_green_after_publication_module() {
    // Smoke-check the production accept path remains importable and usable.
    let f = fixture_with_leaves(1);
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.chronicle_revision, 1);
    assert_eq!(
        SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn child_with_missing_lineage_is_rejected() {
    let mut f = fixture_with_leaves(2);
    f.db.connection()
        .execute(
            "UPDATE round_summaries
             SET lineage_id = NULL,
                 payload_json = json_remove(payload_json, '$.lineage_id')
             WHERE summary_id = ?1",
            [f.leaves[0].id.as_str()],
        )
        .unwrap();

    let parents = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-no-child-lineage",
        vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()],
        1,
        2,
        "B0001",
    )];
    let covers = vec![
        (f.leaves[0].id.clone(), parents[0].id.clone()),
        (f.leaves[1].id.clone(), parents[0].id.clone()),
    ];
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-no-child-lineage"),
        &parents,
        &covers,
    )
    .unwrap_err();
    assert!(err.to_string().contains("lineage"), "{err}");
}

#[test]
fn child_with_wrong_conversation_is_rejected() {
    let mut f = fixture_with_leaves(2);
    // Insert a second conversation so the FK still holds while scope drifts.
    f.db.connection()
        .execute(
            "INSERT INTO conversations (
                conversation_id, campaign_id, character_id, archived_upto,
                created_at, updated_at, payload_json
             ) VALUES ('conv-other', ?1, NULL, 0, 't', 't', '{}')",
            [f.campaign_id.as_str()],
        )
        .unwrap();
    f.db.connection()
        .execute(
            "UPDATE round_summaries
             SET conversation_id = 'conv-other',
                 payload_json = json_set(payload_json, '$.conversation_id', 'conv-other')
             WHERE summary_id = ?1",
            [f.leaves[0].id.as_str()],
        )
        .unwrap();

    let parents = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-wrong-conv",
        vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()],
        1,
        2,
        "B0001",
    )];
    let covers = vec![
        (f.leaves[0].id.clone(), parents[0].id.clone()),
        (f.leaves[1].id.clone(), parents[0].id.clone()),
    ];
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-wrong-conv"),
        &parents,
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("conversation") || err.to_string().contains("scope"),
        "{err}"
    );
}

#[test]
fn completed_replay_rejects_when_db_state_drifted() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();

    // Drift: clear one child's covered_by after successful publication.
    f.db.connection()
        .execute(
            "UPDATE round_summaries
             SET covered_by = NULL,
                 payload_json = json_remove(payload_json, '$.covered_by')
             WHERE summary_id = ?1",
            [f.leaves[0].id.as_str()],
        )
        .unwrap();

    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("drift")
            || err.to_string().contains("incomplete")
            || err.to_string().contains("covered"),
        "{err}"
    );
}

#[test]
fn completed_replay_rejects_exact_parent_and_edge_drift() {
    let mut f = fixture_with_leaves(5);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();

    f.db.connection()
        .execute(
            "UPDATE round_summaries
             SET headline = 'drifted',
                 payload_json = json_set(payload_json, '$.headline', 'drifted')
             WHERE summary_id = ?1",
            [parents[0].id.as_str()],
        )
        .unwrap();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(err.to_string().contains("drift"), "{err}");

    f.db.connection()
        .execute(
            "UPDATE round_summaries
             SET headline = ?1, payload_json = ?2
             WHERE summary_id = ?3",
            rusqlite::params![
                parents[0].headline,
                serde_json::to_string(&parents[0]).unwrap(),
                parents[0].id.as_str()
            ],
        )
        .unwrap();
    f.db.connection()
        .execute(
            "DELETE FROM round_summary_covers WHERE parent_id = ?1 AND child_id = ?2",
            rusqlite::params![parents[0].id.as_str(), f.leaves[0].id.as_str()],
        )
        .unwrap();
    f.db.connection()
        .execute(
            "INSERT INTO round_summary_covers (parent_id, child_id) VALUES (?1, ?2)",
            rusqlite::params![parents[0].id.as_str(), f.leaves[4].id.as_str()],
        )
        .unwrap();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(err.to_string().contains("drift"), "{err}");
}

#[test]
fn completed_replay_rejects_campaign_revision_marker_and_epoch_drift() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();

    let mut campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    campaign.chronicle_revision += 1;
    campaign.pending_compress_publication = Some(
        storyforge_domain::chronicle::PendingCompressPublication::new(
            campaign.chronicle_revision,
            vec![Id::from_str("foreign-parent")],
            vec![(
                Id::from_str("foreign-child"),
                Id::from_str("foreign-parent"),
            )],
        ),
    );
    campaign.context_epoch = Some(
        storyforge_domain::chronicle::ContextEpochSnapshot::new_empty(
            "stale-epoch",
            campaign.chronicle_revision + 1,
        ),
    );
    f.db.connection()
        .execute(
            "UPDATE campaigns
             SET chronicle_revision = ?1, payload_json = ?2
             WHERE campaign_id = ?3",
            rusqlite::params![
                campaign.chronicle_revision,
                serde_json::to_string(&campaign).unwrap(),
                f.campaign_id.as_str()
            ],
        )
        .unwrap();

    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(err.to_string().contains("revision"), "{err}");

    campaign.chronicle_revision -= 1;
    f.db.connection()
        .execute(
            "UPDATE campaigns SET chronicle_revision = ?1, payload_json = ?2 WHERE campaign_id = ?3",
            rusqlite::params![
                campaign.chronicle_revision,
                serde_json::to_string(&campaign).unwrap(),
                f.campaign_id.as_str()
            ],
        )
        .unwrap();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(err.to_string().contains("marker"), "{err}");

    campaign.pending_compress_publication = None;
    f.db.connection()
        .execute(
            "UPDATE campaigns SET payload_json = ?1 WHERE campaign_id = ?2",
            rusqlite::params![
                serde_json::to_string(&campaign).unwrap(),
                f.campaign_id.as_str()
            ],
        )
        .unwrap();
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap_err();
    assert!(err.to_string().contains("epoch"), "{err}");
}

#[test]
fn duplicate_job_id_is_rejected() {
    let mut f = fixture_with_leaves(4);
    let (parents, child_covered_by, publication_id) = a_to_b_request(&f);
    publish(
        &mut f.db,
        &f.campaign_id,
        &publication_id,
        &parents,
        &child_covered_by,
    )
    .unwrap();

    // Seed two fresh leaves and publish under same job id.
    let leaf5 = leaf(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "leaf-5",
        5,
        "A0005",
    );
    let leaf6 = leaf(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "leaf-6",
        6,
        "A0006",
    );
    SqliteChronicleRepository::seed_summary(&mut f.db, &leaf5).unwrap();
    SqliteChronicleRepository::seed_summary(&mut f.db, &leaf6).unwrap();
    let parents2 = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-job-dup",
        vec![leaf5.id.clone(), leaf6.id.clone()],
        5,
        6,
        "B0003",
    )];
    let covers2 = vec![
        (leaf5.id.clone(), parents2[0].id.clone()),
        (leaf6.id.clone(), parents2[0].id.clone()),
    ];
    let first_job = format!("job-{}", publication_id.as_str());
    let err = SqliteChronicleRepository::publish_compress(
        &mut f.db,
        PublishRequest {
            campaign_id: &f.campaign_id,
            publication_id: &Id::from_str("pub-job-dup"),
            parents: &parents2,
            child_covered_by: &covers2,
            job_id: Some(first_job.as_str()),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("job"), "{err}");
}

#[test]
fn existing_pending_marker_is_not_silently_overwritten() {
    let mut f = fixture_with_leaves(2);
    let mut campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    campaign.pending_compress_publication = Some(
        storyforge_domain::chronicle::PendingCompressPublication::new(
            campaign.chronicle_revision,
            vec![Id::from_str("foreign-parent")],
            vec![(
                Id::from_str("foreign-child"),
                Id::from_str("foreign-parent"),
            )],
        ),
    );
    // Write campaign pending marker outside publication path.
    let uow = storyforge_infra_sqlite::UnitOfWork::begin(f.db.connection_mut()).unwrap();
    let tx = uow.transaction().unwrap();
    tx.execute(
        "UPDATE campaigns SET payload_json = ?1 WHERE campaign_id = ?2",
        rusqlite::params![
            serde_json::to_string(&campaign).unwrap(),
            f.campaign_id.as_str()
        ],
    )
    .unwrap();
    uow.commit().unwrap();

    let parents = vec![parent_b(
        &f.campaign_id,
        &f.conversation_id,
        &f.lineage_id,
        "parent-with-existing-marker",
        vec![f.leaves[0].id.clone(), f.leaves[1].id.clone()],
        1,
        2,
        "B0001",
    )];
    let covers = vec![
        (f.leaves[0].id.clone(), parents[0].id.clone()),
        (f.leaves[1].id.clone(), parents[0].id.clone()),
    ];
    let err = publish(
        &mut f.db,
        &f.campaign_id,
        &Id::from_str("pub-existing-marker"),
        &parents,
        &covers,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("pending") || err.to_string().contains("marker"),
        "{err}"
    );
}
