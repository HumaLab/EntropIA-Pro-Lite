//! One `zotero_annotation_items` row and its invariants (plan section 3).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::wire::{self, Data};

/// Pseudo-field of `conflict_fields` for "does the annotation still exist".
pub const EXISTENCE: &str = "__existence";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Synced,
    Pending,
    WaitingParent,
    Conflict,
    Error,
}

impl SyncState {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncState::Synced => "synced",
            SyncState::Pending => "pending",
            SyncState::WaitingParent => "waiting_parent",
            SyncState::Conflict => "conflict",
            SyncState::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "synced" => SyncState::Synced,
            "pending" => SyncState::Pending,
            "waiting_parent" => SyncState::WaitingParent,
            "conflict" => SyncState::Conflict,
            "error" => SyncState::Error,
            _ => return None,
        })
    }

    /// `pending`, `waiting_parent` and `error` hold an unsent local intent.
    pub fn is_dirty_state(self) -> bool {
        matches!(
            self,
            SyncState::Pending | SyncState::WaitingParent | SyncState::Error
        )
    }
}

/// Why `__existence` is in dispute. Stored as the `__existence` entry of
/// `conflict_json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistenceReason {
    /// Moved to the trash in Zotero while a field edit was pending.
    Trashed,
    /// Modified in Zotero after a local delete.
    Modified,
    /// Permanently deleted in Zotero while a field edit was pending.
    Deleted,
    /// Its parent attachment was deleted; the row can only be discarded.
    AttachmentDeleted,
}

/// The body of the request in flight, kept as `sent_json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Sent {
    /// POST with `version: 0`; the full object.
    Create { data: Data },
    /// PATCH; only the changed fields.
    Update { fields: Data },
    /// DELETE.
    Delete,
}

/// One annotation row. Field names follow the table columns.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationRow {
    pub annotation_key: String,
    pub parent_key: String,
    pub base_version: Option<i64>,
    pub base_json: Option<Data>,
    pub remote_trashed: bool,
    pub local_json: Option<Data>,
    pub local_deleted: bool,
    pub edit_gen: i64,
    pub sent_gen: Option<i64>,
    pub sent_json: Option<Sent>,
    pub sent_uncertain: bool,
    pub sync_state: SyncState,
    pub conflict_fields: Vec<String>,
    pub conflict_json: Option<Data>,
    pub last_error: Option<String>,
}

impl AnnotationRow {
    /// A row for an annotation the server already holds.
    pub fn from_server(key: &str, version: i64, data: Data) -> Self {
        let parent_key = parent_of(&data);
        AnnotationRow {
            annotation_key: key.to_string(),
            parent_key,
            base_version: Some(version),
            remote_trashed: wire::is_trashed(&data),
            base_json: Some(data),
            local_json: None,
            local_deleted: false,
            edit_gen: 0,
            sent_gen: None,
            sent_json: None,
            sent_uncertain: false,
            sync_state: SyncState::Synced,
            conflict_fields: Vec::new(),
            conflict_json: None,
            last_error: None,
        }
    }

    /// A row for an annotation created locally and not yet sent.
    pub fn new_local(key: &str, data: Data) -> Self {
        let parent_key = parent_of(&data);
        AnnotationRow {
            annotation_key: key.to_string(),
            parent_key,
            base_version: None,
            base_json: None,
            remote_trashed: false,
            local_json: Some(data),
            local_deleted: false,
            edit_gen: 1,
            sent_gen: None,
            sent_json: None,
            sent_uncertain: false,
            sync_state: SyncState::Pending,
            conflict_fields: Vec::new(),
            conflict_json: None,
            last_error: None,
        }
    }

    /// The row holds an unsent local intent.
    pub fn is_dirty(&self) -> bool {
        self.local_deleted || self.local_json.is_some()
    }

    pub fn in_conflict(&self) -> bool {
        !self.conflict_fields.is_empty()
    }

    /// The data the user currently wants: the pending edit, or the base.
    pub fn desired(&self) -> Option<&Data> {
        self.local_json.as_ref().or(self.base_json.as_ref())
    }

    pub fn existence_reason(&self) -> Option<ExistenceReason> {
        let value = self.conflict_json.as_ref()?.get(EXISTENCE)?;
        serde_json::from_value(value.clone()).ok()
    }

    pub(crate) fn clear_sent(&mut self) {
        self.sent_gen = None;
        self.sent_json = None;
        self.sent_uncertain = false;
    }

    pub(crate) fn set_base(&mut self, version: i64, data: Data) {
        self.remote_trashed = wire::is_trashed(&data);
        self.base_version = Some(version);
        self.base_json = Some(data);
    }

    pub(crate) fn clear_conflict(&mut self) {
        self.conflict_fields.clear();
        self.conflict_json = None;
    }

    pub(crate) fn add_conflict(&mut self, field: &str, remote: Value) {
        if !self
            .conflict_fields
            .iter()
            .any(|existing| existing == field)
        {
            self.conflict_fields.push(field.to_string());
        }
        self.conflict_json
            .get_or_insert_with(Map::new)
            .insert(field.to_string(), remote);
        self.sync_state = SyncState::Conflict;
    }

    pub(crate) fn add_existence_conflict(&mut self, reason: ExistenceReason) {
        let value = serde_json::to_value(reason).unwrap_or(Value::Null);
        self.add_conflict(EXISTENCE, value);
    }

    pub(crate) fn remove_conflict(&mut self, field: &str) {
        self.conflict_fields.retain(|existing| existing != field);
        if let Some(remote) = self.conflict_json.as_mut() {
            remote.remove(field);
        }
    }

    /// Recomputes `sync_state` from the invariants. A dirty row keeps a
    /// blocked state (`error`, `waiting_parent`) only when `keep_block`.
    pub(crate) fn recompute(&mut self, keep_block: bool) {
        if self.in_conflict() {
            self.sync_state = SyncState::Conflict;
            return;
        }
        if self
            .conflict_json
            .as_ref()
            .is_some_and(|remote| remote.is_empty())
        {
            self.conflict_json = None;
        }
        if !self.local_deleted && self.sent_gen.is_none() {
            if let (Some(local), Some(base)) = (&self.local_json, &self.base_json) {
                if wire::content_eq(local, base) {
                    self.local_json = None;
                }
            }
        }
        if !self.is_dirty() {
            self.sync_state = SyncState::Synced;
            self.last_error = None;
        } else if !(keep_block
            && matches!(self.sync_state, SyncState::Error | SyncState::WaitingParent))
        {
            self.sync_state = SyncState::Pending;
        }
    }
}

fn parent_of(data: &Data) -> String {
    data.get("parentItem")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Checks every invariant of plan section 3. Tests call it after each
/// transition; the repository calls it before writing a row.
pub fn check_invariants(row: &AnnotationRow) -> Result<(), String> {
    let dirty = row.is_dirty();
    let conflict = row.in_conflict();
    let state = row.sync_state;
    if (state == SyncState::Synced) != (!conflict && !dirty) {
        return Err(format!(
            "synced <=> clean, got {state:?} dirty={dirty} conflict={conflict}"
        ));
    }
    if state.is_dirty_state() != (!conflict && dirty) {
        return Err(format!(
            "dirty state <=> dirty, got {state:?} dirty={dirty} conflict={conflict}"
        ));
    }
    if (state == SyncState::Conflict) != conflict {
        return Err(format!("conflict <=> conflict_fields, got {state:?}"));
    }
    if row.local_deleted && row.local_json.is_some() {
        return Err("local_deleted implies local_json IS NULL".into());
    }
    // A row in conflict may hold an edit equal to the base until it is
    // resolved; outside a conflict that edit would have collapsed.
    if row.sent_gen.is_none() && !conflict {
        if let Some(local) = &row.local_json {
            if row
                .base_json
                .as_ref()
                .is_some_and(|base| wire::content_eq(local, base))
            {
                return Err("local_json equals base_json with no request in flight".into());
            }
        }
    }
    if row.sent_gen.is_some() != row.sent_json.is_some() {
        return Err("sent_gen and sent_json are set together".into());
    }
    if row.sent_uncertain && row.sent_gen.is_none() {
        return Err("sent_uncertain needs a sent request".into());
    }
    if row.base_version.is_some() != row.base_json.is_some() {
        return Err("base_version and base_json are set together".into());
    }
    Ok(())
}
