//! The required traces of `docs/zotero-annotations-plan.md` (sections 3 to
//! 6.3). Every transition is followed by an invariant check.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::machine::*;
use super::model::*;
use super::wire::Data;

const KEY: &str = "ABCD2345";

fn data(value: Value) -> Data {
    value.as_object().cloned().unwrap()
}

fn server(version: i64, comment: &str, color: &str) -> Data {
    data(json!({
        "key": KEY,
        "version": version,
        "itemType": "annotation",
        "parentItem": "PARENT22",
        "annotationType": "highlight",
        "annotationComment": comment,
        "annotationColor": color,
        "annotationPosition": "{\"pageIndex\":0,\"rects\":[[1,2,3,4]]}",
        "dateModified": format!("2026-10-{version:02}")
    }))
}

fn synced_row(comment: &str) -> AnnotationRow {
    let row = AnnotationRow::from_server(KEY, 5, server(5, comment, "#ffd400"));
    ok(&row);
    row
}

fn ok(row: &AnnotationRow) {
    if let Err(message) = check_invariants(row) {
        panic!("invariant broken: {message}\n{row:#?}");
    }
}

fn with(row: &AnnotationRow, field: &str, value: Value) -> Data {
    let mut desired = row.desired().cloned().unwrap();
    desired.insert(field.into(), value);
    desired
}

fn edit(row: &mut AnnotationRow, field: &str, value: Value) {
    let desired = with(row, field, value);
    assert_eq!(local_edit(row, desired), RowAction::Save);
    ok(row);
}

fn send(row: &mut AnnotationRow) -> Request {
    let request = next_request(row).expect("a request");
    begin_send(row, &request);
    ok(row);
    request
}

fn comment(row: &AnnotationRow) -> Value {
    row.desired().unwrap()["annotationComment"].clone()
}

fn base_comment(row: &AnnotationRow) -> Value {
    row.base_json.as_ref().unwrap()["annotationComment"].clone()
}

fn patch_fields(request: Request) -> Data {
    match request {
        Request::Update { fields, .. } => fields,
        other => panic!("expected a PATCH, got {other:?}"),
    }
}

fn decisions(entries: &[(&str, Decision)]) -> BTreeMap<String, Decision> {
    entries
        .iter()
        .map(|(field, decision)| (field.to_string(), *decision))
        .collect()
}

// --- Local mutations (section 3) ------------------------------------------

#[test]
fn an_edit_back_to_the_base_collapses_to_synced_when_nothing_is_in_flight() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    assert_eq!(row.sync_state, SyncState::Pending);
    edit(&mut row, "annotationComment", json!("A"));
    assert_eq!(row.sync_state, SyncState::Synced);
    assert!(row.local_json.is_none());
}

#[test]
fn an_edit_releases_error_and_waiting_parent_rows() {
    for blocked in [SyncState::Error, SyncState::WaitingParent] {
        let mut row = synced_row("A");
        edit(&mut row, "annotationComment", json!("B"));
        row.sync_state = blocked;
        ok(&row);
        edit(&mut row, "annotationComment", json!("C"));
        assert_eq!(row.sync_state, SyncState::Pending);
    }
}

#[test]
fn deleting_a_never_sent_create_removes_the_row_without_a_request() {
    let mut row = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    ok(&row);
    assert_eq!(local_delete(&mut row), Ok(RowAction::Remove));
}

#[test]
fn a_delete_in_conflict_needs_confirmation() {
    let mut row = comment_conflict();
    assert_eq!(
        local_delete(&mut row),
        Err(MachineError::DeleteNeedsConfirmation)
    );
}

// --- Acknowledgement traces (section 5) -------------------------------------

#[test]
fn a_revert_while_a_patch_is_in_flight_is_sent_after_the_acknowledgement() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    edit(&mut row, "annotationComment", json!("A"));
    assert!(
        row.local_json.is_some(),
        "kept while the base is about to change"
    );
    assert_eq!(
        apply_outcome(&mut row, Outcome::Applied { version: 6 }),
        Next::Done(RowAction::Save)
    );
    ok(&row);
    assert_eq!(base_comment(&row), json!("B"));
    assert_eq!(row.sync_state, SyncState::Pending);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "A"}))
    );
}

#[test]
fn a_newer_edit_during_a_patch_is_sent_against_the_new_base() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    edit(&mut row, "annotationComment", json!("C"));
    apply_outcome(&mut row, Outcome::Applied { version: 6 });
    ok(&row);
    assert_eq!(base_comment(&row), json!("B"));
    let request = send(&mut row);
    assert!(matches!(&request, Request::Update { version: 6, .. }));
    assert_eq!(
        patch_fields(request),
        data(json!({"annotationComment": "C"}))
    );
}

#[test]
fn undoing_an_in_flight_delete_recreates_the_annotation_under_a_new_key() {
    let mut row = synced_row("A");
    local_delete(&mut row).unwrap();
    ok(&row);
    assert!(matches!(send(&mut row), Request::Delete { version: 5, .. }));
    let desired = row.base_json.clone().unwrap();
    local_edit(&mut row, desired);
    ok(&row);
    let Next::Done(RowAction::Rekeyed { old_key }) = apply_outcome(&mut row, Outcome::Gone) else {
        panic!("expected a recreate");
    };
    ok(&row);
    assert_eq!(old_key, KEY);
    assert_ne!(row.annotation_key, KEY);
    assert_eq!(row.base_version, None);
    assert_eq!(comment(&row), json!("A"));
    assert!(matches!(send(&mut row), Request::Create { .. }));
}

#[test]
fn undo_then_delete_again_during_an_in_flight_delete_removes_the_row() {
    let mut row = synced_row("A");
    local_delete(&mut row).unwrap();
    send(&mut row);
    let desired = row.base_json.clone().unwrap();
    local_edit(&mut row, desired);
    local_delete(&mut row).unwrap();
    ok(&row);
    assert_eq!(
        apply_outcome(&mut row, Outcome::Gone),
        Next::Done(RowAction::Remove)
    );
}

#[test]
fn a_rejected_create_waits_for_an_edit_then_is_sent() {
    let mut row = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut row);
    apply_outcome(
        &mut row,
        Outcome::Rejected {
            message: "bad position".into(),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Error);
    assert_eq!(next_request(&mut row), None);
    edit(&mut row, "annotationComment", json!("fixed"));
    assert_eq!(row.sync_state, SyncState::Pending);
    assert!(matches!(send(&mut row), Request::Create { .. }));
}

#[test]
fn a_400_with_a_newer_edit_stays_pending() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    edit(&mut row, "annotationComment", json!("C"));
    apply_outcome(
        &mut row,
        Outcome::Rejected {
            message: "bad".into(),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Pending);
}

#[test]
fn parent_item_not_found_has_three_outcomes() {
    let mut pending_parent = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut pending_parent);
    apply_outcome(
        &mut pending_parent,
        Outcome::ParentMissing {
            parent: ParentStatus::PendingInZotero,
            message: "parent".into(),
        },
    );
    ok(&pending_parent);
    assert_eq!(pending_parent.sync_state, SyncState::WaitingParent);
    assert!(release_waiting_parent(&mut pending_parent));
    ok(&pending_parent);
    assert!(matches!(send(&mut pending_parent), Request::Create { .. }));

    let mut on_server = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut on_server);
    apply_outcome(
        &mut on_server,
        Outcome::ParentMissing {
            parent: ParentStatus::OnServer,
            message: "x".into(),
        },
    );
    ok(&on_server);
    assert_eq!(on_server.sync_state, SyncState::Error);

    let mut orphan = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut orphan);
    apply_outcome(
        &mut orphan,
        Outcome::ParentMissing {
            parent: ParentStatus::Deleted,
            message: "x".into(),
        },
    );
    ok(&orphan);
    assert_eq!(
        orphan.existence_reason(),
        Some(ExistenceReason::AttachmentDeleted)
    );
    assert_eq!(
        resolve(&mut orphan, &decisions(&[(EXISTENCE, Decision::Keep)])),
        Err(MachineError::CannotKeepOrphan)
    );
    assert_eq!(
        resolve(&mut orphan, &decisions(&[(EXISTENCE, Decision::Delete)])),
        Ok(RowAction::Remove)
    );
}

#[test]
fn a_stale_patch_without_conflict_is_retried_once() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    let remote = RemoteChange::Item {
        version: 7,
        data: server(7, "A", "#ff6666"),
    };
    let next = apply_outcome(&mut row, Outcome::Stale { remote });
    ok(&row);
    assert_eq!(next, Next::RetryOnce(RowAction::Save));
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "B"}))
    );
}

#[test]
fn a_lost_create_acknowledgement_is_recovered_by_comparing_content_only() {
    let mut equal = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut equal);
    apply_outcome(
        &mut equal,
        Outcome::CreateExists {
            version: 3,
            data: server(3, "new", "#ffd400"),
        },
    );
    ok(&equal);
    assert_eq!(equal.sync_state, SyncState::Synced);
    assert_eq!(equal.base_version, Some(3));

    let mut edited = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
    send(&mut edited);
    apply_outcome(
        &mut edited,
        Outcome::CreateExists {
            version: 4,
            data: server(4, "other", "#ffd400"),
        },
    );
    ok(&edited);
    assert_eq!(edited.conflict_fields, vec!["annotationComment"]);
}

// --- Uncertain outcome traces (section 5) ----------------------------------

fn uncertain_patch(local_after: Option<&str>) -> AnnotationRow {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    if let Some(value) = local_after {
        edit(&mut row, "annotationComment", json!(value));
    }
    apply_outcome(&mut row, Outcome::Uncertain);
    ok(&row);
    assert!(row.sent_uncertain);
    row
}

#[test]
fn an_uncertain_row_is_never_merged_before_reconciliation() {
    let mut row = uncertain_patch(None);
    let change = RemoteChange::Item {
        version: 6,
        data: server(6, "B", "#ffd400"),
    };
    assert_eq!(
        merge_remote(&mut row, change),
        Err(MachineError::NeedsReconcile)
    );
}

#[test]
fn a_lost_patch_that_was_applied_keeps_the_revert_made_meanwhile() {
    let mut row = uncertain_patch(Some("A"));
    reconcile(
        &mut row,
        Observed::Found {
            version: 6,
            data: server(6, "B", "#ffd400"),
        },
    );
    ok(&row);
    assert_eq!(base_comment(&row), json!("B"));
    assert_eq!(row.sync_state, SyncState::Pending);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "A"}))
    );
}

#[test]
fn a_lost_patch_that_was_not_applied_ends_synced_after_a_revert() {
    let mut row = uncertain_patch(Some("A"));
    reconcile(
        &mut row,
        Observed::Found {
            version: 5,
            data: server(5, "A", "#ffd400"),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Synced);
}

#[test]
fn a_lost_patch_applied_under_a_remote_color_change_is_not_a_conflict() {
    let mut row = uncertain_patch(None);
    reconcile(
        &mut row,
        Observed::Found {
            version: 7,
            data: server(7, "B", "#ff6666"),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Synced);
    assert_eq!(base_comment(&row), json!("B"));
}

#[test]
fn a_lost_patch_that_never_landed_under_a_remote_change_is_still_sent() {
    let mut row = uncertain_patch(None);
    reconcile(
        &mut row,
        Observed::Found {
            version: 6,
            data: server(6, "A", "#ff6666"),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Pending);
    assert_eq!(comment(&row), json!("B"));
    assert_eq!(row.desired().unwrap()["annotationColor"], json!("#ff6666"));
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "B"}))
    );
}

#[test]
fn a_lost_patch_on_a_permanently_deleted_annotation_becomes_an_existence_conflict() {
    let mut row = uncertain_patch(None);
    reconcile(&mut row, Observed::NotFound);
    ok(&row);
    assert_eq!(row.existence_reason(), Some(ExistenceReason::Deleted));
}

#[test]
fn a_lost_delete_that_was_applied_removes_the_row() {
    let mut row = synced_row("A");
    local_delete(&mut row).unwrap();
    send(&mut row);
    apply_outcome(&mut row, Outcome::Uncertain);
    assert_eq!(reconcile(&mut row, Observed::NotFound), RowAction::Remove);
}

#[test]
fn a_lost_delete_not_applied_after_an_undo_ends_synced() {
    let mut row = synced_row("A");
    local_delete(&mut row).unwrap();
    send(&mut row);
    let desired = row.base_json.clone().unwrap();
    local_edit(&mut row, desired);
    apply_outcome(&mut row, Outcome::Uncertain);
    ok(&row);
    reconcile(
        &mut row,
        Observed::Found {
            version: 5,
            data: server(5, "A", "#ffd400"),
        },
    );
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Synced);
}

#[test]
fn a_restart_marks_an_outstanding_request_uncertain_and_keeps_the_revert() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    edit(&mut row, "annotationComment", json!("A"));
    // The app is killed here; at startup:
    assert!(mark_outstanding_uncertain(&mut row));
    ok(&row);
    reconcile(
        &mut row,
        Observed::Found {
            version: 6,
            data: server(6, "B", "#ffd400"),
        },
    );
    ok(&row);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "A"}))
    );
}

#[test]
fn deleting_a_create_whose_post_is_uncertain_waits_for_its_outcome() {
    let created = || {
        let mut row = AnnotationRow::new_local(KEY, server(0, "new", "#ffd400"));
        send(&mut row);
        apply_outcome(&mut row, Outcome::Uncertain);
        assert_eq!(local_delete(&mut row), Ok(RowAction::Save));
        ok(&row);
        row
    };
    let mut landed = created();
    reconcile(
        &mut landed,
        Observed::Found {
            version: 3,
            data: server(3, "new", "#ffd400"),
        },
    );
    ok(&landed);
    assert!(matches!(
        send(&mut landed),
        Request::Delete { version: 3, .. }
    ));

    let mut lost = created();
    assert_eq!(reconcile(&mut lost, Observed::NotFound), RowAction::Remove);
}

// --- Merge table (section 4.1) ----------------------------------------------

fn item(version: i64, comment: &str, color: &str) -> RemoteChange {
    RemoteChange::Item {
        version,
        data: server(version, comment, color),
    }
}

fn trashed(version: i64, comment: &str) -> RemoteChange {
    let mut data = server(version, comment, "#ffd400");
    data.insert("deleted".into(), json!(1));
    RemoteChange::Item { version, data }
}

#[test]
fn merge_nothing_pending_takes_modified_trashed_and_restored() {
    let mut row = synced_row("A");
    merge_remote(&mut row, item(6, "C", "#ffd400")).unwrap();
    ok(&row);
    assert_eq!(base_comment(&row), json!("C"));
    merge_remote(&mut row, trashed(7, "C")).unwrap();
    ok(&row);
    assert!(row.remote_trashed);
    merge_remote(&mut row, item(8, "C", "#ffd400")).unwrap();
    ok(&row);
    assert!(!row.remote_trashed);
    assert_eq!(row.sync_state, SyncState::Synced);
}

#[test]
fn merge_field_edit_pending_with_a_remote_modification_is_three_way() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    merge_remote(&mut row, item(6, "A", "#ff6666")).unwrap();
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Pending);
    assert_eq!(comment(&row), json!("B"));
    assert_eq!(row.desired().unwrap()["annotationColor"], json!("#ff6666"));
    assert_eq!(row.base_version, Some(6));

    merge_remote(&mut row, item(7, "C", "#ff6666")).unwrap();
    ok(&row);
    assert_eq!(row.conflict_fields, vec!["annotationComment"]);
    assert_eq!(
        row.conflict_json.as_ref().unwrap()["annotationComment"],
        json!("C")
    );
}

#[test]
fn merge_field_edit_pending_with_a_remote_trash_is_an_existence_conflict() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    merge_remote(&mut row, trashed(6, "A")).unwrap();
    ok(&row);
    assert_eq!(row.existence_reason(), Some(ExistenceReason::Trashed));
}

#[test]
fn merge_delete_pending_rows() {
    let deleted = || {
        let mut row = synced_row("A");
        local_delete(&mut row).unwrap();
        row
    };
    let mut modified = deleted();
    merge_remote(&mut modified, item(6, "C", "#ffd400")).unwrap();
    ok(&modified);
    assert_eq!(modified.existence_reason(), Some(ExistenceReason::Modified));

    let mut both_gone = deleted();
    merge_remote(&mut both_gone, trashed(6, "A")).unwrap();
    ok(&both_gone);
    assert_eq!(both_gone.sync_state, SyncState::Pending);
    assert!(matches!(
        send(&mut both_gone),
        Request::Delete { version: 6, .. }
    ));

    let mut gone = deleted();
    assert_eq!(
        merge_remote(&mut gone, RemoteChange::PermanentlyDeleted),
        Ok(RowAction::Remove)
    );
}

#[test]
fn merge_permanent_deletions() {
    let mut clean = synced_row("A");
    assert_eq!(
        merge_remote(&mut clean, RemoteChange::PermanentlyDeleted),
        Ok(RowAction::Remove)
    );

    let mut edited = synced_row("A");
    edit(&mut edited, "annotationComment", json!("B"));
    merge_remote(&mut edited, RemoteChange::PermanentlyDeleted).unwrap();
    ok(&edited);
    assert_eq!(edited.existence_reason(), Some(ExistenceReason::Deleted));
}

// --- Conflict traces (section 6.3) ------------------------------------------

fn comment_conflict() -> AnnotationRow {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    merge_remote(&mut row, item(6, "C", "#ffd400")).unwrap();
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Conflict);
    row
}

#[test]
fn an_edit_during_a_conflict_stays_in_conflict_and_zotero_wins_the_comment() {
    let mut row = comment_conflict();
    edit(&mut row, "annotationColor", json!("#5fb236"));
    assert_eq!(row.sync_state, SyncState::Conflict);
    assert_eq!(next_request(&mut row), None);
    resolve(
        &mut row,
        &decisions(&[("annotationComment", Decision::Zotero)]),
    )
    .unwrap();
    ok(&row);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationColor": "#5fb236"}))
    );
    assert_eq!(comment(&row), json!("C"));
}

#[test]
fn keeping_an_annotation_deleted_locally_and_modified_remotely_sends_no_delete() {
    let mut row = synced_row("A");
    local_delete(&mut row).unwrap();
    merge_remote(&mut row, item(6, "C", "#ffd400")).unwrap();
    resolve(&mut row, &decisions(&[(EXISTENCE, Decision::Keep)])).unwrap();
    ok(&row);
    assert!(!row.local_deleted);
    assert_eq!(next_request(&mut row), None);
    edit(&mut row, "annotationComment", json!("D"));
    assert!(matches!(send(&mut row), Request::Update { .. }));
}

#[test]
fn a_confirmed_delete_during_a_conflict_is_sent() {
    let mut row = comment_conflict();
    local_delete_confirmed(&mut row);
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Pending);
    assert!(matches!(send(&mut row), Request::Delete { .. }));
}

#[test]
fn keeping_an_annotation_trashed_in_zotero_restores_it_with_the_local_fields() {
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    merge_remote(&mut row, trashed(6, "A")).unwrap();
    resolve(&mut row, &decisions(&[(EXISTENCE, Decision::Keep)])).unwrap();
    ok(&row);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "B", "deleted": 0}))
    );
}

#[test]
fn a_conflict_clears_by_itself_when_the_remote_value_converges() {
    let mut row = comment_conflict();
    merge_remote(&mut row, item(7, "B", "#ffd400")).unwrap();
    ok(&row);
    assert_eq!(row.sync_state, SyncState::Synced);
}

#[test]
fn keeping_a_permanently_deleted_annotation_recreates_it_with_zotero_s_comment() {
    let mut row = comment_conflict();
    merge_remote(&mut row, RemoteChange::PermanentlyDeleted).unwrap();
    ok(&row);
    let action = resolve(
        &mut row,
        &decisions(&[
            (EXISTENCE, Decision::Keep),
            ("annotationComment", Decision::Zotero),
        ]),
    )
    .unwrap();
    ok(&row);
    assert!(matches!(action, RowAction::Rekeyed { .. }));
    assert_eq!(comment(&row), json!("C"));
    assert!(matches!(send(&mut row), Request::Create { .. }));
}

#[test]
fn a_resolution_must_decide_every_disputed_entry() {
    let mut row = comment_conflict();
    assert_eq!(
        resolve(&mut row, &BTreeMap::new()),
        Err(MachineError::Undecided("annotationComment".into()))
    );
}

#[test]
fn restoring_a_trashed_annotation_sends_deleted_zero() {
    let mut row = synced_row("A");
    merge_remote(&mut row, trashed(6, "A")).unwrap();
    local_restore(&mut row).unwrap();
    ok(&row);
    assert_eq!(patch_fields(send(&mut row)), data(json!({"deleted": 0})));
}

#[test]
fn a_remote_trash_during_a_field_conflict_adds_an_existence_choice() {
    let mut row = comment_conflict();
    merge_remote(&mut row, trashed(7, "C")).unwrap();
    ok(&row);
    assert_eq!(row.existence_reason(), Some(ExistenceReason::Trashed));
    assert!(row
        .conflict_fields
        .iter()
        .any(|field| field == "annotationComment"));
    assert!(
        !super::wire::is_trashed(row.local_json.as_ref().unwrap()),
        "the trash is a decision, never merged into the local intent"
    );
    resolve(
        &mut row,
        &decisions(&[
            (EXISTENCE, Decision::Keep),
            ("annotationComment", Decision::Local),
        ]),
    )
    .unwrap();
    ok(&row);
    assert_eq!(
        patch_fields(send(&mut row)),
        data(json!({"annotationComment": "B", "deleted": 0}))
    );
}

#[test]
fn a_pull_of_our_own_last_write_is_not_a_remote_change() {
    // The PATCH of a run lands after that run's pull: the next pull brings the
    // same version back. Nothing new arrived, so a delete made since is not
    // "modified in Zotero after you deleted it".
    let mut row = synced_row("A");
    edit(&mut row, "annotationComment", json!("B"));
    send(&mut row);
    apply_outcome(&mut row, Outcome::Applied { version: 6 });
    ok(&row);
    let own_write = row.base_json.clone().unwrap();
    local_delete(&mut row).unwrap();
    merge_remote(
        &mut row,
        RemoteChange::Item {
            version: 6,
            data: own_write,
        },
    )
    .unwrap();
    ok(&row);
    assert!(row.conflict_fields.is_empty(), "{:?}", row.conflict_fields);
    assert!(matches!(send(&mut row), Request::Delete { version: 6, .. }));
}
