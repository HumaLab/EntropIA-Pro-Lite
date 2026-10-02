//! Durable E1b-2 reconciliation tests.
//!
//! Every row and entity key here is synthetic. The tests drive the public
//! repository seam against SQLite, never a live Zotero connection or payload.

use entropia_desktop_lib::bibliography::reconciliation::{
    advance_phase, begin_run, checkpoint_page, finalize_run, get_run, mark_blocked,
    mark_interrupted, record_error, resume_run, AdvanceReconciliationPhaseInput,
    BeginReconciliationInput, ReconciliationEntityKind, ReconciliationErrorInput,
    ReconciliationPageInput, ReconciliationPhase, ReconciliationRunRef, ReconciliationSeenInput,
    ReconciliationState,
};
use entropia_desktop_lib::bibliography::repository::{
    upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
    SourceOrigin, UpsertConnection, UpsertLibrary,
};
use rusqlite::Connection;

const CATALOG_MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0040_bibliography_catalog.sql");
const RELATIONS_MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0041_bibliography_relations.sql");
const RECONCILIATION_MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0042_bibliography_reconciliation.sql");

fn migrated_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open synthetic database");
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE _migrations (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             name TEXT NOT NULL UNIQUE,
             applied_at INTEGER NOT NULL
         );",
    )
    .expect("migration tracking table");

    for _ in 0..2 {
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{CATALOG_MIGRATION_SQL}\nCOMMIT;"
        ))
        .expect("apply catalog migration");
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{RELATIONS_MIGRATION_SQL}\nCOMMIT;"
        ))
        .expect("apply relations migration");
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{RECONCILIATION_MIGRATION_SQL}\nCOMMIT;"
        ))
        .expect("apply reconciliation migration");
    }

    for name in [
        "0040_bibliography_catalog",
        "0041_bibliography_relations",
        "0042_bibliography_reconciliation",
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [name],
        )
        .expect("record migration");
    }
    conn
}

fn connection(id: &str) -> UpsertConnection {
    UpsertConnection {
        id: id.to_string(),
        source_origin: SourceOrigin::Local,
        source_instance_id: Some(format!("synthetic-{id}")),
        endpoint: Some("http://synthetic.invalid".to_string()),
        capabilities_json: r#"{"read":true}"#.to_string(),
    }
}

fn library(connection_id: &str, library_id: &str) -> UpsertLibrary {
    UpsertLibrary {
        connection_id: connection_id.to_string(),
        library_type: LibraryType::User,
        library_id: library_id.to_string(),
        name: format!("Synthetic {library_id}"),
        last_modified_version: Some(7),
    }
}

fn begin_input(library_id: &str, connection_revision: i64) -> BeginReconciliationInput {
    BeginReconciliationInput {
        library_id: library_id.to_string(),
        connection_revision,
        cursor_start: 0,
        cursor_limit: 2,
        remote_total: None,
        target_version: Some(9),
    }
}

fn run_ref(
    run: &entropia_desktop_lib::bibliography::reconciliation::ReconciliationRun,
) -> ReconciliationRunRef {
    ReconciliationRunRef {
        library_id: run.library_id.clone(),
        run_id: run.run_id.clone(),
        connection_revision: run.connection_revision,
    }
}

fn seen(
    kind: ReconciliationEntityKind,
    entity_key: &str,
    parent_key: Option<&str>,
    remote_version: Option<i64>,
    observed_at: i64,
) -> ReconciliationSeenInput {
    ReconciliationSeenInput {
        entity_kind: kind,
        entity_key: entity_key.to_string(),
        parent_key: parent_key.map(str::to_string),
        remote_version,
        observed_at,
    }
}

fn page(
    run: &entropia_desktop_lib::bibliography::reconciliation::ReconciliationRun,
    cursor_start: i64,
    next_cursor_start: i64,
    seen_entities: Vec<ReconciliationSeenInput>,
) -> ReconciliationPageInput {
    ReconciliationPageInput {
        run: run_ref(run),
        phase: ReconciliationPhase::Versions,
        cursor_start,
        next_cursor_start,
        remote_total: None,
        seen: seen_entities,
    }
}

fn item_input(key: &str) -> BibliographicItemInput {
    BibliographicItemInput {
        item_key: key.to_string(),
        item_version: Some(1),
        native_json_snapshot: format!(r#"{{"key":"{key}"}}"#),
        csl_json_snapshot: format!(r#"{{"id":"synthetic-{key}","type":"book"}}"#),
        ..Default::default()
    }
}

#[test]
fn migration_is_replay_safe_and_preserves_the_prior_catalog() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let item = upsert_item(&mut conn, &library.id, item_input("CATALOG-01")).expect("item");

    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM _migrations WHERE name='0042_bibliography_reconciliation'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("migration record"),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE id=?1",
            [&item.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("catalog row"),
        1
    );
    for table in ["zotero_reconciliation_runs", "zotero_reconciliation_seen"] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .expect("table metadata"),
            1,
            "missing {table}"
        );
    }
}

#[test]
fn begin_creates_one_fenced_run_and_rejects_an_active_fresh_begin() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");

    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    assert!(!run.run_id.is_empty());
    assert_eq!(run.library_id, library.id);
    assert_eq!(run.connection_revision, 0);
    assert_eq!(run.state, ReconciliationState::Running);
    assert_eq!(run.phase, ReconciliationPhase::Versions);
    assert_eq!(run.cursor_start, 0);
    assert_eq!(run.cursor_limit, 2);
    assert_eq!(run.target_version, Some(9));
    assert_eq!(run.checkpoint_version, None);
    assert_eq!(run.attempt_count, 1);
    assert_eq!(run.retry_count, 0);
    assert_eq!(
        get_run(&conn, &library.id).expect("read run"),
        Some(run.clone())
    );

    let error = begin_run(&mut conn, begin_input(&library.id, source.revision))
        .expect_err("a second fresh run must not steal an active run");
    assert_eq!(error.code, "active_run");
}

#[test]
fn begin_and_checkpoint_use_connection_revision_as_the_stale_identity_fence() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");

    let mut changed_connection = connection("conn-1");
    changed_connection.endpoint = Some("http://synthetic-revision-2.invalid".to_string());
    let refreshed = upsert_connection(&mut conn, changed_connection).expect("refresh connection");
    assert_eq!(refreshed.revision, 1);

    let error = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Item,
                "ITEM-01",
                None,
                Some(1),
                10,
            )],
        ),
    )
    .expect_err("stale connection revision must reject the page");
    assert_eq!(error.code, "stale_connection");
    let stored = get_run(&conn, &library.id)
        .expect("read run")
        .expect("run exists");
    assert_eq!(stored.cursor_start, 0);
    assert_eq!(stored.revision, run.revision);
}

#[test]
fn identical_native_keys_in_different_libraries_and_runs_stay_distinct() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let first_library = upsert_library(&mut conn, library(&source.id, "0")).expect("first library");
    let second_library =
        upsert_library(&mut conn, library(&source.id, "6680944")).expect("second library");

    let first =
        begin_run(&mut conn, begin_input(&first_library.id, source.revision)).expect("first run");
    let second =
        begin_run(&mut conn, begin_input(&second_library.id, source.revision)).expect("second run");
    checkpoint_page(
        &mut conn,
        page(
            &first,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Item,
                "SAME-NATIVE-KEY",
                None,
                Some(3),
                11,
            )],
        ),
    )
    .expect("first page");
    checkpoint_page(
        &mut conn,
        page(
            &second,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Item,
                "SAME-NATIVE-KEY",
                None,
                Some(4),
                12,
            )],
        ),
    )
    .expect("second page");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen WHERE entity_key='SAME-NATIVE-KEY'",
            [],
            |row| row.get(0),
        )
        .expect("seen count");
    assert_eq!(count, 2);
    let scoped: Vec<(String, String, i64)> = {
        let mut statement = conn
            .prepare(
                "SELECT library_id, run_id, remote_version
                   FROM zotero_reconciliation_seen
                  WHERE entity_key='SAME-NATIVE-KEY'
                  ORDER BY library_id",
            )
            .expect("seen query");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("seen rows")
            .map(|row| row.expect("seen row"))
            .collect()
    };
    assert_eq!(scoped.len(), 2);
    assert_ne!(scoped[0].0, scoped[1].0);
    assert_ne!(scoped[0].1, scoped[1].1);
}

#[test]
fn checkpoint_is_atomic_replay_safe_and_rejects_stale_page_fences() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    let first_page = page(
        &run,
        0,
        2,
        vec![
            seen(ReconciliationEntityKind::Item, "ITEM-01", None, Some(1), 10),
            seen(
                ReconciliationEntityKind::Attachment,
                "ATT-01",
                Some("ITEM-01"),
                Some(2),
                10,
            ),
        ],
    );

    let advanced = checkpoint_page(&mut conn, first_page.clone()).expect("checkpoint page");
    assert_eq!(advanced.cursor_start, 2);
    assert_eq!(advanced.checkpoint_version, None);
    assert!(advanced.checkpointed_at.is_some());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .expect("seen count"),
        2
    );

    let replay = checkpoint_page(&mut conn, first_page).expect("replay already advanced page");
    assert_eq!(
        replay, advanced,
        "page replay must not create a new revision"
    );

    let stale_cursor = checkpoint_page(
        &mut conn,
        page(
            &advanced,
            1,
            3,
            vec![seen(
                ReconciliationEntityKind::Item,
                "ITEM-03",
                None,
                Some(3),
                11,
            )],
        ),
    )
    .expect_err("a cursor that is neither current nor an exact replay is stale");
    assert_eq!(stale_cursor.code, "stale_cursor");

    let mut wrong_run = page(
        &advanced,
        2,
        4,
        vec![seen(
            ReconciliationEntityKind::Item,
            "ITEM-04",
            None,
            Some(4),
            12,
        )],
    );
    wrong_run.run.run_id = "stale-run-id".to_string();
    assert_eq!(
        checkpoint_page(&mut conn, wrong_run)
            .expect_err("wrong run must be rejected")
            .code,
        "stale_run"
    );

    let mut wrong_phase = page(
        &advanced,
        2,
        4,
        vec![seen(
            ReconciliationEntityKind::Item,
            "ITEM-04",
            None,
            Some(4),
            12,
        )],
    );
    wrong_phase.phase = ReconciliationPhase::Catalog;
    assert_eq!(
        checkpoint_page(&mut conn, wrong_phase)
            .expect_err("wrong phase must be rejected")
            .code,
        "stale_phase"
    );

    conn.execute_batch(
        "CREATE TRIGGER fail_reconciliation_seen
         BEFORE INSERT ON zotero_reconciliation_seen
         WHEN NEW.entity_key='FAIL-ATOMIC'
         BEGIN SELECT RAISE(ABORT, 'synthetic checkpoint failure'); END;",
    )
    .expect("failure trigger");
    let injected_failure = checkpoint_page(
        &mut conn,
        page(
            &advanced,
            2,
            4,
            vec![
                seen(ReconciliationEntityKind::Item, "ITEM-02", None, Some(2), 13),
                seen(
                    ReconciliationEntityKind::Item,
                    "FAIL-ATOMIC",
                    None,
                    Some(3),
                    13,
                ),
            ],
        ),
    )
    .expect_err("injected seen-set failure must roll back the whole page");
    assert_eq!(injected_failure.code, "sql_error");
    conn.execute_batch("DROP TRIGGER fail_reconciliation_seen")
        .expect("remove failure trigger");
    let after_failure = get_run(&conn, &library.id)
        .expect("read run")
        .expect("run exists");
    assert_eq!(after_failure.cursor_start, 2);
    assert_eq!(after_failure.revision, advanced.revision);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen WHERE entity_key IN ('ITEM-02','FAIL-ATOMIC')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("rolled-back seen rows"),
        0
    );
}

#[test]
fn checkpoint_rejects_remote_total_regression_but_accepts_monotonic_updates() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");

    let known = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(5),
            ..page(&run, 0, 1, vec![])
        },
    )
    .expect("initial remote total");
    assert_eq!(known.remote_total, Some(5));

    let regressed = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(4),
            ..page(&known, 1, 2, vec![])
        },
    )
    .expect_err("a known remote total must not regress");
    assert_eq!(regressed.code, "stale_total");
    assert_eq!(
        get_run(&conn, &library.id).expect("read run"),
        Some(known.clone())
    );

    let equal = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(5),
            ..page(&known, 1, 2, vec![])
        },
    )
    .expect("an equal remote total remains valid");
    let higher = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(6),
            ..page(&equal, 2, 3, vec![])
        },
    )
    .expect("a higher remote total remains valid");
    let unknown = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: None,
            ..page(&higher, 3, 4, vec![])
        },
    )
    .expect("an omitted remote total remains valid");
    assert_eq!(unknown.remote_total, Some(6));
    assert_eq!(unknown.cursor_start, 4);
}

#[test]
fn phase_transition_preserves_seen_rows_and_fences_catalog_pages() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    let versions = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Item,
                "ITEM-PHASE",
                None,
                Some(1),
                15,
            )],
        ),
    )
    .expect("versions page");

    let catalog = advance_phase(
        &mut conn,
        AdvanceReconciliationPhaseInput {
            run: run_ref(&versions),
            phase: ReconciliationPhase::Versions,
            next_phase: ReconciliationPhase::Catalog,
            next_cursor_start: 0,
        },
    )
    .expect("advance to catalog phase");
    assert_eq!(catalog.phase, ReconciliationPhase::Catalog);
    assert_eq!(catalog.cursor_start, 0);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .expect("preserved phase seen rows"),
        1
    );

    let catalog_page = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            run: run_ref(&catalog),
            phase: ReconciliationPhase::Catalog,
            cursor_start: 0,
            next_cursor_start: 1,
            remote_total: Some(1),
            seen: vec![seen(
                ReconciliationEntityKind::Collection,
                "COLL-PHASE",
                None,
                Some(2),
                16,
            )],
        },
    )
    .expect("catalog page");
    let completed =
        finalize_run(&mut conn, run_ref(&catalog_page)).expect("finalize catalog phase");
    assert_eq!(completed.phase, ReconciliationPhase::Finalize);
    assert_eq!(completed.state, ReconciliationState::Completed);
}

#[test]
fn seen_set_constraints_require_normalized_synthetic_identity() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");

    let empty_key = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            1,
            vec![seen(ReconciliationEntityKind::Item, "   ", None, None, 1)],
        ),
    )
    .expect_err("entity keys must be non-empty");
    assert_eq!(empty_key.code, "invalid_input");

    let missing_parent = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            1,
            vec![seen(
                ReconciliationEntityKind::Attachment,
                "ATT-ORPHAN",
                None,
                None,
                1,
            )],
        ),
    )
    .expect_err("attachments require a parent key");
    assert_eq!(missing_parent.code, "invalid_input");

    let negative_version = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            1,
            vec![seen(
                ReconciliationEntityKind::Item,
                "ITEM-NEGATIVE",
                None,
                Some(-1),
                1,
            )],
        ),
    )
    .expect_err("remote versions must be non-negative");
    assert_eq!(negative_version.code, "invalid_input");

    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .expect("seen count"),
        0
    );
}

#[test]
fn record_error_rejects_a_stale_revision_without_mutating_retry_metadata() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    let checkpointed = checkpoint_page(&mut conn, page(&run, 0, 1, vec![])).expect("checkpoint");

    let stale = record_error(
        &mut conn,
        ReconciliationErrorInput {
            run: run_ref(&run),
            expected_revision: run.revision,
            phase: ReconciliationPhase::Versions,
            code: "stale-timeout".to_string(),
            message: "synthetic stale metadata".to_string(),
            retryable: true,
            next_retry_at: Some(100),
        },
    )
    .expect_err("a stale revision must reject the error submission");
    assert_eq!(stale.code, "stale_revision");
    assert_eq!(
        get_run(&conn, &library.id).expect("read run"),
        Some(checkpointed)
    );
}

#[test]
fn errors_change_only_retry_state_and_preserve_cursor_and_seen_rows() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    let checkpointed = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Item,
                "ITEM-ERROR",
                None,
                Some(1),
                20,
            )],
        ),
    )
    .expect("checkpoint");
    let seen_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| row.get(0),
        )
        .expect("seen count");

    let retry_wait = record_error(
        &mut conn,
        ReconciliationErrorInput {
            run: run_ref(&checkpointed),
            expected_revision: checkpointed.revision,
            phase: ReconciliationPhase::Versions,
            code: "timeout".to_string(),
            message: "synthetic metadata-only timeout".to_string(),
            retryable: true,
            next_retry_at: Some(100),
        },
    )
    .expect("retryable error");
    assert_eq!(retry_wait.state, ReconciliationState::RetryWait);
    assert_eq!(retry_wait.retry_count, 1);
    assert_eq!(
        retry_wait
            .latest_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("timeout")
    );
    assert_eq!(
        retry_wait
            .latest_error
            .as_ref()
            .map(|error| error.retryable),
        Some(true)
    );
    assert_eq!(retry_wait.cursor_start, checkpointed.cursor_start);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("seen count after retry"),
        seen_before
    );

    let replayed = record_error(
        &mut conn,
        ReconciliationErrorInput {
            run: run_ref(&retry_wait),
            expected_revision: retry_wait.revision,
            phase: ReconciliationPhase::Versions,
            code: "timeout".to_string(),
            message: "synthetic metadata-only timeout".to_string(),
            retryable: true,
            next_retry_at: Some(100),
        },
    )
    .expect_err("a retryable error must transition only from running");
    assert_eq!(replayed.code, "invalid_state");
    assert_eq!(
        get_run(&conn, &library.id).expect("read run"),
        Some(retry_wait.clone())
    );

    let resumed = resume_run(&mut conn, run_ref(&retry_wait)).expect("explicit retry resume");
    assert_eq!(resumed.state, ReconciliationState::Running);
    assert_eq!(resumed.cursor_start, checkpointed.cursor_start);
    assert_eq!(resumed.attempt_count, retry_wait.attempt_count + 1);

    let failed = record_error(
        &mut conn,
        ReconciliationErrorInput {
            run: run_ref(&resumed),
            expected_revision: resumed.revision,
            phase: ReconciliationPhase::Versions,
            code: "blocked-by-fixture".to_string(),
            message: "synthetic non-retryable metadata".to_string(),
            retryable: false,
            next_retry_at: None,
        },
    )
    .expect("non-retryable error");
    assert_eq!(failed.state, ReconciliationState::Failed);
    assert_eq!(failed.next_retry_at, None);
    assert_eq!(
        failed.latest_error.as_ref().map(|error| error.retryable),
        Some(false)
    );
    assert_eq!(failed.cursor_start, checkpointed.cursor_start);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("seen count after failure"),
        seen_before
    );

    let too_long = record_error(
        &mut conn,
        ReconciliationErrorInput {
            run: run_ref(&failed),
            expected_revision: failed.revision,
            phase: ReconciliationPhase::Versions,
            code: "too-long".to_string(),
            message: "x".repeat(1025),
            retryable: false,
            next_retry_at: None,
        },
    )
    .expect_err("error messages must be bounded");
    assert_eq!(too_long.code, "invalid_input");
}

#[test]
fn interruption_requires_explicit_resume_and_preserves_the_seen_set() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let run = begin_run(&mut conn, begin_input(&library.id, source.revision)).expect("begin run");
    let checkpointed = checkpoint_page(
        &mut conn,
        page(
            &run,
            0,
            2,
            vec![seen(
                ReconciliationEntityKind::Collection,
                "COLL-01",
                None,
                Some(1),
                30,
            )],
        ),
    )
    .expect("checkpoint");
    let interrupted = mark_interrupted(&mut conn, run_ref(&checkpointed)).expect("interrupt");
    assert_eq!(interrupted.state, ReconciliationState::Interrupted);

    let fresh_error = begin_run(&mut conn, begin_input(&library.id, source.revision))
        .expect_err("fresh begin must not discard an interrupted run");
    assert_eq!(fresh_error.code, "interrupted_run");
    let resumed = resume_run(&mut conn, run_ref(&interrupted)).expect("resume interrupted run");
    assert_eq!(resumed.state, ReconciliationState::Running);
    assert_eq!(resumed.cursor_start, interrupted.cursor_start);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .expect("preserved seen count"),
        1
    );

    let blocked = mark_blocked(&mut conn, run_ref(&resumed)).expect("block explicitly");
    assert_eq!(blocked.state, ReconciliationState::Blocked);
}

#[test]
fn finalize_requires_a_complete_known_total_and_never_regresses_checkpoint_version() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let mut run = begin_run(
        &mut conn,
        BeginReconciliationInput {
            remote_total: Some(3),
            ..begin_input(&library.id, source.revision)
        },
    )
    .expect("begin run");

    run = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(3),
            ..page(
                &run,
                0,
                2,
                vec![seen(
                    ReconciliationEntityKind::Item,
                    "ITEM-01",
                    None,
                    Some(1),
                    40,
                )],
            )
        },
    )
    .expect("partial page");
    assert_eq!(run.checkpoint_version, None);
    let incomplete = finalize_run(&mut conn, run_ref(&run))
        .expect_err("known remote total must prevent partial finalization");
    assert_eq!(incomplete.code, "incomplete_run");
    assert_eq!(
        get_run(&conn, &library.id)
            .expect("read run")
            .expect("run exists")
            .checkpoint_version,
        None
    );

    run = checkpoint_page(
        &mut conn,
        ReconciliationPageInput {
            remote_total: Some(3),
            ..page(
                &run,
                2,
                3,
                vec![seen(
                    ReconciliationEntityKind::Item,
                    "ITEM-02",
                    None,
                    Some(2),
                    41,
                )],
            )
        },
    )
    .expect("complete page");
    let completed = finalize_run(&mut conn, run_ref(&run)).expect("complete known total");
    assert_eq!(completed.state, ReconciliationState::Completed);
    assert_eq!(completed.phase, ReconciliationPhase::Finalize);
    assert_eq!(completed.checkpoint_version, Some(9));
    assert!(completed.completed_at.is_some());
    assert_eq!(completed.target_version, Some(9));

    let replayed = finalize_run(&mut conn, run_ref(&completed))
        .expect("replaying a completed finalization must be idempotent");
    assert_eq!(replayed, completed);

    let second_run = begin_run(
        &mut conn,
        BeginReconciliationInput {
            remote_total: None,
            target_version: Some(4),
            ..begin_input(&library.id, source.revision)
        },
    )
    .expect("fresh run after completion");
    assert_eq!(second_run.checkpoint_version, Some(9));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_reconciliation_seen",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .expect("fresh run clears old seen rows"),
        0
    );
    let unknown_total_completed = finalize_run(&mut conn, run_ref(&second_run))
        .expect("unknown totals require this explicit finalize call");
    assert_eq!(unknown_total_completed.checkpoint_version, Some(9));
    assert_eq!(unknown_total_completed.target_version, Some(4));
    assert_eq!(
        conn.query_row(
            "SELECT last_modified_version FROM zotero_libraries WHERE id=?1",
            [&library.id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .expect("catalog version"),
        Some(7)
    );
}

#[test]
fn sql_constraints_cover_boolean_retry_flags_non_negative_values_and_error_shape() {
    let mut conn = migrated_db();
    let missing_library = conn
        .execute(
            "INSERT INTO zotero_reconciliation_runs
             (library_id, run_id, connection_revision, state, phase, cursor_start, cursor_limit,
              retry_count, attempt_count, latest_error_retryable, created_at, updated_at)
             VALUES ('missing-library', 'run-1', 0, 'running', 'versions', 0, 1,
                     0, 0, 2, 1, 1)",
            [],
        )
        .expect_err("foreign key and boolean checks must reject invalid rows");
    assert!(
        missing_library.to_string().contains("FOREIGN KEY")
            || missing_library.to_string().contains("CHECK")
    );

    let source = upsert_connection(&mut conn, connection("conn-1")).expect("connection");
    let library = upsert_library(&mut conn, library(&source.id, "0")).expect("library");
    let bad_retry_flag = conn
        .execute(
            "INSERT INTO zotero_reconciliation_runs
             (library_id, run_id, connection_revision, state, phase, cursor_start, cursor_limit,
              retry_count, attempt_count, latest_error_phase, latest_error_code,
              latest_error_message, latest_error_retryable, latest_error_at, created_at, updated_at)
             VALUES (?1, 'run-1', 0, 'running', 'versions', 0, 1, 0, 0,
                     'versions', 'bad', 'metadata', 2, 1, 1, 1)",
            [&library.id],
        )
        .expect_err("retry flag must be boolean");
    assert!(bad_retry_flag.to_string().contains("CHECK"));

    let bad_cursor = conn
        .execute(
            "INSERT INTO zotero_reconciliation_runs
             (library_id, run_id, connection_revision, state, phase, cursor_start, cursor_limit,
              retry_count, attempt_count, created_at, updated_at)
             VALUES (?1, 'run-2', 0, 'running', 'versions', -1, 1, 0, 0, 1, 1)",
            [&library.id],
        )
        .expect_err("cursor start must be non-negative");
    assert!(bad_cursor.to_string().contains("CHECK"));
}
