//! The annotation sync state machine (plan sections 3 to 6).
//!
//! Every function takes a row and returns what the caller must do next; none
//! does I/O. The repository persists the row and the transport performs the
//! request a [`Request`] describes.

use std::collections::BTreeMap;

use serde_json::Value;
use thiserror::Error;

use super::model::{AnnotationRow, ExistenceReason, Sent, SyncState, EXISTENCE};
use super::wire::{self, Data};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MachineError {
    #[error("the annotation is in conflict; deleting it needs an explicit confirmation")]
    DeleteNeedsConfirmation,
    #[error("the row has an uncertain request and must be reconciled first")]
    NeedsReconcile,
    #[error("a request is in flight for this row")]
    RequestInFlight,
    #[error("the row is not in conflict")]
    NotInConflict,
    #[error("no decision for `{0}`")]
    Undecided(String),
    #[error("`{0}` is not in conflict")]
    UnknownDecision(String),
    #[error("an annotation whose attachment was deleted can only be discarded")]
    CannotKeepOrphan,
    #[error("only an annotation in Zotero's trash can be restored")]
    NotTrashed,
}

/// What the caller does with the row after a transition.
#[derive(Debug, Clone, PartialEq)]
pub enum RowAction {
    /// Persist the row as it is now.
    Save,
    /// Delete the row; nothing is left on either side.
    Remove,
    /// The server item is gone but the user keeps the annotation: the row
    /// moved to `new_key` (version 0, no base). Delete the old key's row and
    /// persist this one.
    Rekeyed { old_key: String },
}

// ---------------------------------------------------------------------------
// Local mutations (section 3)
// ---------------------------------------------------------------------------

/// A local edit: `desired` is the full `data` object the user wants.
pub fn local_edit(row: &mut AnnotationRow, desired: Data) -> RowAction {
    row.edit_gen += 1;
    row.local_deleted = false;
    row.local_json = Some(desired);
    if row.in_conflict() {
        // Recorded, but the row stays in conflict and is not uploadable.
        return RowAction::Save;
    }
    row.recompute(false);
    RowAction::Save
}

/// A local delete. On a row in conflict it needs [`local_delete_confirmed`].
pub fn local_delete(row: &mut AnnotationRow) -> Result<RowAction, MachineError> {
    if row.in_conflict() {
        return Err(MachineError::DeleteNeedsConfirmation);
    }
    Ok(delete_intent(row))
}

/// A delete the user confirmed while the row was in conflict: a resolution
/// with `__existence = delete` (section 6.2).
pub fn local_delete_confirmed(row: &mut AnnotationRow) -> RowAction {
    row.clear_conflict();
    delete_intent(row)
}

fn delete_intent(row: &mut AnnotationRow) -> RowAction {
    if row.base_version.is_none() && row.sent_gen.is_none() {
        // Never reached the server: nothing to send.
        return RowAction::Remove;
    }
    row.edit_gen += 1;
    row.local_json = None;
    row.local_deleted = true;
    row.recompute(false);
    RowAction::Save
}

/// Restores an annotation that is in Zotero's trash (section 9).
pub fn local_restore(row: &mut AnnotationRow) -> Result<RowAction, MachineError> {
    let desired = row.desired().cloned().unwrap_or_default();
    if !wire::is_trashed(&desired) {
        return Err(MachineError::NotTrashed);
    }
    let mut restored = desired;
    wire::set_trashed(&mut restored, false);
    Ok(local_edit(row, restored))
}

// ---------------------------------------------------------------------------
// Pull merge (section 4.1)
// ---------------------------------------------------------------------------

/// What the pull saw for one key.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteChange {
    Item { version: i64, data: Data },
    /// Listed by `/deleted`.
    PermanentlyDeleted,
}

/// Merges a remote change into a row. Rows with an uncertain request must be
/// reconciled first; rows with a request in flight are never merged.
pub fn merge_remote(row: &mut AnnotationRow, change: RemoteChange) -> Result<RowAction, MachineError> {
    if row.sent_uncertain {
        return Err(MachineError::NeedsReconcile);
    }
    if row.sent_gen.is_some() {
        return Err(MachineError::RequestInFlight);
    }
    let merge_base = row.base_json.clone().unwrap_or_default();
    Ok(merge_with_base(row, change, &merge_base))
}

/// The merge proper, against an explicit merge base (the reconciliation
/// passes a corrected one).
fn merge_with_base(row: &mut AnnotationRow, change: RemoteChange, merge_base: &Data) -> RowAction {
    let keep_block = true;
    match change {
        RemoteChange::PermanentlyDeleted => {
            if row.in_conflict() {
                if row.local_deleted {
                    return RowAction::Remove;
                }
                row.add_existence_conflict(ExistenceReason::Deleted);
                return RowAction::Save;
            }
            if row.local_json.is_some() {
                row.add_existence_conflict(ExistenceReason::Deleted);
                RowAction::Save
            } else {
                // Nothing pending, or a delete pending: both sides agree.
                RowAction::Remove
            }
        }
        RemoteChange::Item { version, data } => {
            let remote_trashed = wire::is_trashed(&data);
            let base_trashed = wire::is_trashed(merge_base);
            if row.in_conflict() {
                remerge_conflict(row, merge_base, &data);
                row.set_base(version, data);
                if !row.in_conflict() {
                    row.recompute(keep_block);
                }
                return RowAction::Save;
            }
            if row.local_deleted {
                if remote_trashed {
                    // Both sides want it gone; the delete goes out with the
                    // new version.
                    row.set_base(version, data);
                } else {
                    row.set_base(version, data);
                    row.add_existence_conflict(ExistenceReason::Modified);
                }
                return RowAction::Save;
            }
            let Some(mut local) = row.local_json.take() else {
                row.set_base(version, data);
                row.recompute(keep_block);
                return RowAction::Save;
            };
            let local_trashed = wire::is_trashed(&local);
            if remote_trashed && !base_trashed && !local_trashed {
                row.local_json = Some(local);
                row.set_base(version, data);
                row.add_existence_conflict(ExistenceReason::Trashed);
                return RowAction::Save;
            }
            let disputed = three_way(merge_base, &mut local, &data);
            row.local_json = Some(local);
            for (field, remote) in disputed {
                row.add_conflict(&field, remote);
            }
            row.set_base(version, data);
            row.recompute(keep_block);
            RowAction::Save
        }
    }
}

/// Three-way merge of every field. Remote-only changes are applied to
/// `local`; local-only changes stay; fields changed differently on both sides
/// are returned with their remote value.
fn three_way(merge_base: &Data, local: &mut Data, remote: &Data) -> Vec<(String, Value)> {
    let mut disputed = Vec::new();
    let mut fields = wire::field_union(merge_base, local);
    for field in wire::field_union(local, remote) {
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    for field in fields {
        if field == "key" {
            continue;
        }
        let base_value = merge_base.get(&field);
        let local_value = local.get(&field).cloned();
        let remote_value = remote.get(&field);
        if wire::field_eq(&field, local_value.as_ref(), remote_value) {
            continue;
        }
        if wire::field_eq(&field, local_value.as_ref(), base_value) || wire::is_server_field(&field) {
            match remote_value {
                Some(value) => {
                    local.insert(field.clone(), value.clone());
                }
                None => {
                    local.remove(&field);
                }
            }
        } else if wire::field_eq(&field, remote_value, base_value) {
            // Local-only change: kept.
        } else {
            disputed.push((field, remote_value.cloned().unwrap_or(Value::Null)));
        }
    }
    disputed
}

/// A pull of a row already in conflict: the merge runs again against the new
/// base; converged fields leave `conflict_fields`.
fn remerge_conflict(row: &mut AnnotationRow, old_base: &Data, remote: &Data) {
    let remote_trashed = wire::is_trashed(remote);
    if row.existence_reason() == Some(ExistenceReason::Trashed) && !remote_trashed {
        row.remove_conflict(EXISTENCE);
    }
    let Some(mut local) = row.local_json.take() else {
        return;
    };
    // Moved to the trash in Zotero while the user keeps the annotation: that is
    // an existence decision (4.1), never a remote-only change merged into the
    // local intent.
    let newly_trashed =
        remote_trashed && !wire::is_trashed(old_base) && !wire::is_trashed(&local);
    if newly_trashed && row.existence_reason().is_none() {
        row.add_existence_conflict(ExistenceReason::Trashed);
    }
    let disputed_before: Vec<String> = row
        .conflict_fields
        .iter()
        .filter(|field| field.as_str() != EXISTENCE)
        .cloned()
        .collect();
    for field in &disputed_before {
        let remote_value = remote.get(field);
        if wire::field_eq(field, local.get(field), remote_value) {
            row.remove_conflict(field);
        } else {
            row.add_conflict(field, remote_value.cloned().unwrap_or(Value::Null));
        }
    }
    // Fields not yet in dispute merge three-way against the old base.
    let mut undisputed_base = old_base.clone();
    for field in &disputed_before {
        undisputed_base.remove(field);
    }
    let mut scratch = local.clone();
    for field in &disputed_before {
        scratch.remove(field);
    }
    let mut remote_scratch = remote.clone();
    for field in &disputed_before {
        remote_scratch.remove(field);
    }
    if newly_trashed {
        undisputed_base.remove(wire::DELETED_FIELD);
        scratch.remove(wire::DELETED_FIELD);
        remote_scratch.remove(wire::DELETED_FIELD);
    }
    for (field, value) in three_way(&undisputed_base, &mut scratch, &remote_scratch) {
        row.add_conflict(&field, value);
    }
    for (field, value) in scratch {
        local.insert(field, value);
    }
    row.local_json = Some(local);
}

// ---------------------------------------------------------------------------
// Push (section 5)
// ---------------------------------------------------------------------------

/// The request a row needs, built only for `pending` rows with nothing out.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Create { key: String, data: Data },
    Update { key: String, version: i64, fields: Data },
    Delete { key: String, version: i64 },
}

/// Builds the next request for a row, or `None` when it has nothing to send.
/// A row whose edit turned out to match the base collapses to `synced`.
pub fn next_request(row: &mut AnnotationRow) -> Option<Request> {
    if row.sync_state != SyncState::Pending || row.sent_gen.is_some() {
        return None;
    }
    let key = row.annotation_key.clone();
    match (row.base_version, row.local_deleted) {
        (None, true) => None,
        (None, false) => {
            let mut data = row.local_json.clone()?;
            data.insert("key".into(), Value::String(key.clone()));
            data.insert("version".into(), Value::from(0));
            data.insert("itemType".into(), Value::String("annotation".into()));
            data.remove("dateAdded");
            data.remove("dateModified");
            Some(Request::Create { key, data })
        }
        (Some(version), true) => Some(Request::Delete { key, version }),
        (Some(version), false) => {
            let base = row.base_json.clone().unwrap_or_default();
            let fields = wire::patch_body(&base, row.local_json.as_ref()?);
            if fields.is_empty() {
                row.recompute(false);
                return None;
            }
            Some(Request::Update { key, version, fields })
        }
    }
}

/// Records the request about to be sent, in the same transaction that reads
/// the row.
pub fn begin_send(row: &mut AnnotationRow, request: &Request) {
    row.sent_gen = Some(row.edit_gen);
    row.sent_uncertain = false;
    row.sent_json = Some(match request {
        Request::Create { data, .. } => Sent::Create { data: data.clone() },
        Request::Update { fields, .. } => Sent::Update { fields: fields.clone() },
        Request::Delete { .. } => Sent::Delete,
    });
}

/// Where the parent attachment of a failed create stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentStatus {
    /// `GET` of the parent returned 200: the failure is about something else.
    OnServer,
    /// 404, but the attachment is still in the local catalog.
    PendingInZotero,
    /// 404, and the attachment is gone from the local catalog.
    Deleted,
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// 200 `successful` / `unchanged` for a create, 204 for a PATCH.
    Applied { version: i64 },
    /// DELETE answered 204 or 404.
    Gone,
    /// 412: the caller reads the key and passes what it found.
    Stale { remote: RemoteChange },
    /// 412 on a create: the key exists; the server object.
    CreateExists { version: i64, data: Data },
    /// 400 or a `failed` entry that is not about the parent.
    Rejected { message: String },
    /// "parent item not found", already disambiguated.
    ParentMissing { parent: ParentStatus, message: String },
    /// 401, 403, 409, 429 with `Retry-After`, or another definitive refusal
    /// that leaves the row as it was; the caller pauses the library.
    Refused,
    /// No definitive answer: reset or timeout after sending, 5xx, an
    /// unparseable response.
    Uncertain,
}

/// What the drainer does after an outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    Done(RowAction),
    /// A 412 merged with no conflict: send once more against the new base.
    RetryOnce(RowAction),
}

impl Next {
    pub fn into_action(self) -> RowAction {
        match self {
            Next::Done(action) | Next::RetryOnce(action) => action,
        }
    }
}

/// Applies the answer to the request recorded by [`begin_send`].
pub fn apply_outcome(row: &mut AnnotationRow, outcome: Outcome) -> Next {
    let sent = row.sent_json.clone();
    let newer_edit = row.sent_gen != Some(row.edit_gen);
    match outcome {
        Outcome::Applied { version } => Next::Done(acknowledge(row, sent, version)),
        Outcome::Gone => Next::Done(delete_gone(row)),
        Outcome::Stale { remote } => {
            row.clear_sent();
            let action = merge_with_base(row, remote, &row.base_json.clone().unwrap_or_default());
            if action == RowAction::Save && row.sync_state == SyncState::Pending {
                Next::RetryOnce(action)
            } else {
                Next::Done(action)
            }
        }
        Outcome::CreateExists { version, data } => Next::Done(create_exists(row, version, data)),
        Outcome::Rejected { message } => {
            row.clear_sent();
            if newer_edit {
                row.recompute(false);
            } else {
                row.last_error = Some(message);
                row.sync_state = SyncState::Error;
            }
            Next::Done(RowAction::Save)
        }
        Outcome::ParentMissing { parent, message } => {
            row.clear_sent();
            match parent {
                ParentStatus::OnServer => {
                    row.last_error = Some(message);
                    row.sync_state = if newer_edit { SyncState::Pending } else { SyncState::Error };
                }
                ParentStatus::PendingInZotero => {
                    row.last_error = Some(message);
                    row.sync_state = SyncState::WaitingParent;
                }
                ParentStatus::Deleted => {
                    row.add_existence_conflict(ExistenceReason::AttachmentDeleted);
                }
            }
            Next::Done(RowAction::Save)
        }
        Outcome::Refused => {
            row.clear_sent();
            row.recompute(true);
            Next::Done(RowAction::Save)
        }
        Outcome::Uncertain => {
            row.sent_uncertain = true;
            Next::Done(RowAction::Save)
        }
    }
}

/// Success: the base becomes what was sent plus the returned version.
fn acknowledge(row: &mut AnnotationRow, sent: Option<Sent>, version: i64) -> RowAction {
    let base = row.base_json.clone().unwrap_or_default();
    let mut new_base = match sent {
        Some(Sent::Create { data }) => data,
        Some(Sent::Update { fields }) => wire::apply_patch(&base, &fields),
        Some(Sent::Delete) | None => base,
    };
    new_base.insert("version".into(), Value::from(version));
    new_base.insert("key".into(), Value::String(row.annotation_key.clone()));
    let applied_everything = row.sent_gen == Some(row.edit_gen);
    row.set_base(version, new_base);
    row.clear_sent();
    if applied_everything {
        row.local_json = None;
        row.local_deleted = false;
        row.sync_state = SyncState::Synced;
        row.last_error = None;
    } else {
        row.recompute(false);
    }
    RowAction::Save
}

/// DELETE answered 204/404: follow the latest local intent.
fn delete_gone(row: &mut AnnotationRow) -> RowAction {
    let applied_everything = row.sent_gen == Some(row.edit_gen);
    row.clear_sent();
    if applied_everything || row.local_deleted {
        return RowAction::Remove;
    }
    recreate(row)
}

/// The server item no longer exists but the user keeps the annotation: a new
/// key, version 0, no base, the latest desired data.
fn recreate(row: &mut AnnotationRow) -> RowAction {
    let desired = row.desired().cloned().unwrap_or_default();
    let old_key = std::mem::replace(&mut row.annotation_key, wire::generate_key());
    let mut data = desired;
    data.insert("key".into(), Value::String(row.annotation_key.clone()));
    wire::set_trashed(&mut data, false);
    row.base_version = None;
    row.base_json = None;
    row.remote_trashed = false;
    row.local_deleted = false;
    row.local_json = Some(data);
    row.edit_gen += 1;
    row.clear_sent();
    row.clear_conflict();
    row.sync_state = SyncState::Pending;
    RowAction::Rekeyed { old_key }
}

/// A create retried after a lost acknowledgement fails with 412: compare the
/// server object with what was sent, content fields only.
fn create_exists(row: &mut AnnotationRow, version: i64, server: Data) -> RowAction {
    let sent = match row.sent_json.clone() {
        Some(Sent::Create { data }) => data,
        _ => row.local_json.clone().unwrap_or_default(),
    };
    let applied_everything = row.sent_gen == Some(row.edit_gen);
    row.clear_sent();
    if wire::content_eq(&sent, &server) {
        row.set_base(version, server);
        if applied_everything {
            row.local_json = None;
        }
        if row.local_deleted {
            row.sync_state = SyncState::Pending;
            return RowAction::Save;
        }
        row.recompute(false);
        return RowAction::Save;
    }
    // Someone edited the item in that window.
    let local = row.local_json.clone().unwrap_or_else(|| sent.clone());
    let disputed = wire::differing_content_fields(&local, &server);
    row.set_base(version, server.clone());
    if row.local_deleted {
        row.add_existence_conflict(ExistenceReason::Modified);
        return RowAction::Save;
    }
    row.local_json = Some(local);
    for field in disputed {
        row.add_conflict(&field, server.get(&field).cloned().unwrap_or(Value::Null));
    }
    row.recompute(false);
    RowAction::Save
}

/// At startup no request can be live: every outstanding one is uncertain.
pub fn mark_outstanding_uncertain(row: &mut AnnotationRow) -> bool {
    if row.sent_gen.is_some() && !row.sent_uncertain {
        row.sent_uncertain = true;
        return true;
    }
    false
}

/// `waiting_parent` rows go back to `pending` at the start of every drain.
pub fn release_waiting_parent(row: &mut AnnotationRow) -> bool {
    if row.sync_state == SyncState::WaitingParent && row.sent_gen.is_none() {
        row.sync_state = SyncState::Pending;
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Uncertain outcome reconciliation (section 5)
// ---------------------------------------------------------------------------

/// What `GET /items/{key}?includeTrashed=1` returned.
#[derive(Debug, Clone, PartialEq)]
pub enum Observed {
    Found { version: i64, data: Data },
    /// 404, or the key is listed by `/deleted`.
    NotFound,
}

/// Resolves an uncertain request from what the server holds now.
pub fn reconcile(row: &mut AnnotationRow, observed: Observed) -> RowAction {
    let Some(sent) = row.sent_json.clone() else {
        row.sent_uncertain = false;
        return RowAction::Save;
    };
    match sent {
        Sent::Create { .. } => match observed {
            Observed::NotFound => {
                row.clear_sent();
                if row.local_deleted {
                    return RowAction::Remove;
                }
                row.recompute(true);
                RowAction::Save
            }
            Observed::Found { version, data } => create_exists(row, version, data),
        },
        Sent::Update { fields } => match observed {
            Observed::NotFound => {
                row.clear_sent();
                merge_with_base(
                    row,
                    RemoteChange::PermanentlyDeleted,
                    &row.base_json.clone().unwrap_or_default(),
                )
            }
            Observed::Found { version, data } if Some(version) == row.base_version => {
                let _ = data;
                row.clear_sent();
                row.recompute(true);
                RowAction::Save
            }
            Observed::Found { version, data } => {
                // A higher version does not prove the PATCH landed: the merge
                // base takes the sent value only where the server holds it.
                let mut merge_base = row.base_json.clone().unwrap_or_default();
                for (field, value) in &fields {
                    if wire::field_eq(field, data.get(field), Some(value)) {
                        merge_base.insert(field.clone(), value.clone());
                    }
                }
                row.clear_sent();
                merge_with_base(row, RemoteChange::Item { version, data }, &merge_base)
            }
        },
        Sent::Delete => match observed {
            Observed::NotFound => delete_gone(row),
            Observed::Found { version, .. } if Some(version) == row.base_version => {
                row.clear_sent();
                row.recompute(true);
                RowAction::Save
            }
            Observed::Found { version, data } => {
                row.clear_sent();
                merge_with_base(
                    row,
                    RemoteChange::Item { version, data },
                    &row.base_json.clone().unwrap_or_default(),
                )
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Conflict resolution (section 6.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// `__existence`: keep the annotation.
    Keep,
    /// `__existence`: delete it.
    Delete,
    /// A field: keep the local value.
    Local,
    /// A field: take Zotero's value.
    Zotero,
}

/// Applies one decision per entry of `conflict_fields`.
pub fn resolve(
    row: &mut AnnotationRow,
    decisions: &BTreeMap<String, Decision>,
) -> Result<RowAction, MachineError> {
    if !row.in_conflict() {
        return Err(MachineError::NotInConflict);
    }
    for field in &row.conflict_fields {
        if !decisions.contains_key(field) {
            return Err(MachineError::Undecided(field.clone()));
        }
    }
    if let Some(field) = decisions.keys().find(|field| !row.conflict_fields.contains(field)) {
        return Err(MachineError::UnknownDecision(field.clone()));
    }
    let remote_values = row.conflict_json.clone().unwrap_or_default();
    let reason = row.existence_reason();
    let mut recreate_needed = false;
    if let Some(decision) = decisions.get(EXISTENCE) {
        match decision {
            Decision::Delete | Decision::Zotero => {
                row.clear_conflict();
                if reason == Some(ExistenceReason::AttachmentDeleted) || reason == Some(ExistenceReason::Deleted) {
                    // Nothing left on the server to delete.
                    return Ok(RowAction::Remove);
                }
                return Ok(delete_intent(row));
            }
            Decision::Keep | Decision::Local => {
                if reason == Some(ExistenceReason::AttachmentDeleted) {
                    return Err(MachineError::CannotKeepOrphan);
                }
                row.local_deleted = false;
                let mut desired = row.desired().cloned().unwrap_or_default();
                match reason {
                    Some(ExistenceReason::Trashed) => wire::set_trashed(&mut desired, false),
                    Some(ExistenceReason::Deleted) => recreate_needed = true,
                    _ => {}
                }
                row.local_json = Some(desired);
            }
        }
    }
    for (field, decision) in decisions {
        if field == EXISTENCE {
            continue;
        }
        if *decision == Decision::Zotero {
            let local = row.local_json.get_or_insert_with(Data::new);
            match remote_values.get(field) {
                Some(Value::Null) | None => {
                    local.remove(field);
                }
                Some(value) => {
                    local.insert(field.clone(), value.clone());
                }
            }
        }
    }
    row.edit_gen += 1;
    row.clear_conflict();
    if recreate_needed {
        return Ok(recreate(row));
    }
    row.recompute(false);
    Ok(RowAction::Save)
}
