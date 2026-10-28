//! Zotero annotation `data` objects on the wire (plan section 7).
//!
//! The `data` object is kept as JSON so fields this build does not know
//! survive a round trip byte for byte. Only the helpers here know which
//! fields the server owns and how `annotationPosition` compares.

use serde_json::{Map, Value};

/// A Zotero item `data` object.
pub type Data = Map<String, Value>;

/// Fields the server fills in or owns. They never go into a PATCH body and are
/// ignored when deciding whether two objects hold the same content.
pub const SERVER_FIELDS: [&str; 4] = ["key", "version", "dateAdded", "dateModified"];

/// The trash flag of a Zotero item. Merged like any other field.
pub const DELETED_FIELD: &str = "deleted";

/// `annotationPosition` travels as a JSON string; it is compared parsed.
pub const POSITION_FIELD: &str = "annotationPosition";

/// Zotero's key alphabet (no 0, 1, O).
pub const KEY_ALPHABET: &[u8] = b"23456789ABCDEFGHIJKLMNPQRSTUVWXYZ";

/// Length of a Zotero object key.
pub const KEY_LEN: usize = 8;

pub fn is_server_field(field: &str) -> bool {
    SERVER_FIELDS.contains(&field)
}

/// Generates a Zotero object key from random bytes. Each byte picks one
/// alphabet letter; the bias of `% 33` over 256 values is irrelevant for
/// uniqueness at the scale of one library.
pub fn key_from_bytes(bytes: &[u8; KEY_LEN]) -> String {
    bytes
        .iter()
        .map(|byte| KEY_ALPHABET[*byte as usize % KEY_ALPHABET.len()] as char)
        .collect()
}

/// A fresh random Zotero object key.
pub fn generate_key() -> String {
    let uuid = uuid::Uuid::new_v4();
    let mut bytes = [0u8; KEY_LEN];
    bytes.copy_from_slice(&uuid.as_bytes()[..KEY_LEN]);
    key_from_bytes(&bytes)
}

pub fn is_valid_key(key: &str) -> bool {
    key.len() == KEY_LEN && key.bytes().all(|byte| KEY_ALPHABET.contains(&byte))
}

/// Whether two values of `field` hold the same content. `annotationPosition`
/// is compared as parsed JSON, so key order and whitespace do not matter.
pub fn field_eq(field: &str, a: Option<&Value>, b: Option<&Value>) -> bool {
    if field == POSITION_FIELD {
        return parsed_position(a) == parsed_position(b);
    }
    a == b
}

fn parsed_position(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(text) => {
            Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone())))
        }
        other => Some(other.clone()),
    }
}

/// Whether two objects hold the same content, ignoring server-owned fields.
pub fn content_eq(a: &Data, b: &Data) -> bool {
    field_union(a, b)
        .into_iter()
        .filter(|field| !is_server_field(field))
        .all(|field| field_eq(&field, a.get(&field), b.get(&field)))
}

/// The content fields whose values differ between `a` and `b`.
pub fn differing_content_fields(a: &Data, b: &Data) -> Vec<String> {
    field_union(a, b)
        .into_iter()
        .filter(|field| !is_server_field(field))
        .filter(|field| !field_eq(field, a.get(field), b.get(field)))
        .collect()
}

/// Every field present in either object, in a stable order.
pub fn field_union(a: &Data, b: &Data) -> Vec<String> {
    let mut fields: Vec<String> = a.keys().cloned().collect();
    for key in b.keys() {
        if !a.contains_key(key) {
            fields.push(key.clone());
        }
    }
    fields
}

/// Whether the object is in Zotero's trash. Zotero writes `deleted: 1`; some
/// responses use `true`.
pub fn is_trashed(data: &Data) -> bool {
    match data.get(DELETED_FIELD) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

/// Sets or clears the trash flag the way Zotero expects in a write.
pub fn set_trashed(data: &mut Data, trashed: bool) {
    data.insert(
        DELETED_FIELD.into(),
        Value::from(if trashed { 1 } else { 0 }),
    );
}

/// The PATCH body: only the content fields of `desired` that differ from
/// `base`. A field removed locally is sent as an empty string, which Zotero
/// reads as "clear".
pub fn patch_body(base: &Data, desired: &Data) -> Data {
    let mut body = Data::new();
    for field in differing_content_fields(base, desired) {
        let value = desired
            .get(&field)
            .cloned()
            .unwrap_or(Value::String(String::new()));
        body.insert(field, value);
    }
    body
}

/// `base` with the fields of `patch` applied, as the server holds it after a
/// successful PATCH.
pub fn apply_patch(base: &Data, patch: &Data) -> Data {
    let mut merged = base.clone();
    for (field, value) in patch {
        merged.insert(field.clone(), value.clone());
    }
    merged
}

/// Parses `annotationPosition` into an object, keeping unknown fields.
pub fn parse_position(data: &Data) -> Option<Map<String, Value>> {
    match data.get(POSITION_FIELD)? {
        Value::String(text) => match serde_json::from_str(text).ok()? {
            Value::Object(object) => Some(object),
            _ => None,
        },
        Value::Object(object) => Some(object.clone()),
        _ => None,
    }
}

/// Writes `annotationPosition` back as the JSON string Zotero stores.
pub fn write_position(data: &mut Data, position: &Map<String, Value>) {
    let text = Value::Object(position.clone()).to_string();
    data.insert(POSITION_FIELD.into(), Value::String(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(value: Value) -> Data {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn generated_keys_use_the_zotero_alphabet() {
        for _ in 0..200 {
            let key = generate_key();
            assert!(is_valid_key(&key), "{key}");
        }
        assert_eq!(key_from_bytes(&[0, 1, 2, 3, 32, 33, 255, 7]), "2345Z2R9");
    }

    #[test]
    fn content_comparison_ignores_server_fields_and_parses_the_position() {
        let a = data(json!({"key": "AAAAAAAA", "version": 3, "dateModified": "x",
            "annotationComment": "c", "annotationPosition": "{\"pageIndex\":1,\"rects\":[[1,2,3,4]]}"}));
        let b = data(json!({"key": "AAAAAAAA", "version": 9, "dateModified": "y",
            "annotationComment": "c", "annotationPosition": "{ \"rects\": [[1,2,3,4]], \"pageIndex\": 1 }"}));
        assert!(content_eq(&a, &b));
        let c = data(json!({"annotationComment": "other"}));
        assert_eq!(
            differing_content_fields(&a, &c),
            vec!["annotationComment", "annotationPosition"]
        );
    }

    #[test]
    fn patch_body_holds_only_changed_content_fields() {
        let base =
            data(json!({"version": 3, "annotationComment": "a", "annotationColor": "#ffd400"}));
        let desired = data(
            json!({"version": 3, "annotationComment": "b", "annotationColor": "#ffd400", "deleted": 0}),
        );
        assert_eq!(
            patch_body(&base, &desired),
            data(json!({"annotationComment": "b", "deleted": 0}))
        );
    }

    #[test]
    fn the_trash_flag_reads_numbers_and_booleans() {
        assert!(is_trashed(&data(json!({"deleted": 1}))));
        assert!(is_trashed(&data(json!({"deleted": true}))));
        assert!(!is_trashed(&data(json!({"deleted": 0}))));
        assert!(!is_trashed(&data(json!({}))));
    }

    #[test]
    fn position_round_trip_keeps_unknown_fields() {
        let mut item = data(
            json!({"annotationPosition": "{\"pageIndex\":0,\"rects\":[[1,2,3,4]],\"future\":true}"}),
        );
        let mut position = parse_position(&item).unwrap();
        position.insert("rects".into(), json!([[5, 6, 7, 8]]));
        write_position(&mut item, &position);
        let reread = parse_position(&item).unwrap();
        assert_eq!(reread.get("future"), Some(&json!(true)));
        assert_eq!(reread.get("rects"), Some(&json!([[5, 6, 7, 8]])));
    }
}
