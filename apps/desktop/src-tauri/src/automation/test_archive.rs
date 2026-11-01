//! A migrated archive for the endpoint tests.
//!
//! The schema is the real one — [`crate::db::migrations::apply_migrations`] runs
//! the same registry production runs — so a test can never assert against a
//! hand-written table that has drifted from the shipped schema. The dataset is
//! small and known: two collections, five items in one and two in the other, two
//! assets on the first item, and every item indexed for full-text search.

use std::path::{Path, PathBuf};

use rusqlite::params;

/// First collection: five items, the first one with assets.
pub(crate) const ALFA: &str = "col_alfa";

/// Second collection: two items, all of them matching "revista".
pub(crate) const BETA: &str = "col_beta";

/// Items in [`ALFA`].
pub(crate) const ALFA_ITEMS: usize = 5;

/// Items in [`BETA`].
pub(crate) const BETA_ITEMS: usize = 2;

/// The id of the collection's `index`th item, counting from one.
pub(crate) fn item_id(collection: &str, index: usize) -> String {
    let prefix = if collection == ALFA { "alfa" } else { "beta" };
    format!("ite_{prefix}_{index}")
}

/// Build and seed an archive in `dir`, returning its database path.
pub(crate) fn seeded_archive(dir: &Path) -> PathBuf {
    let db_path = dir.join("entropia.sqlite");
    let mut conn =
        crate::db::open::open_archive_connection(&db_path).expect("open the fixture archive");
    crate::db::migrations::apply_migrations(&mut conn).expect("apply the real migrations");

    conn.execute(
        "INSERT INTO collections (id, name, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![ALFA, "Alfa", "Primera colección", 1_000_i64, 2_000_i64],
    )
    .expect("insert the first collection");
    conn.execute(
        "INSERT INTO collections (id, name, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![BETA, "Beta", Option::<String>::None, 1_100_i64, 2_100_i64],
    )
    .expect("insert the second collection");

    for index in 1..=ALFA_ITEMS {
        let metadata = (index == 1).then_some(r#"{"fuente":"prueba"}"#);
        insert_item(
            &conn,
            &item_id(ALFA, index),
            &format!("Documento {index}"),
            ALFA,
            metadata,
            "documento de prueba alfa",
        );
    }
    for index in 1..=BETA_ITEMS {
        insert_item(
            &conn,
            &item_id(BETA, index),
            &format!("Revista {index}"),
            BETA,
            None,
            "revista de prueba beta",
        );
    }

    for (id, asset_type, path, sort_index, size, created_at) in [
        (
            "ast_alfa_1",
            "pdf",
            "assets/alfa-1.pdf",
            0_i64,
            1_234_i64,
            1_500_i64,
        ),
        (
            "ast_alfa_2",
            "image",
            "assets/alfa-1-cover.jpg",
            1,
            99,
            1_501,
        ),
    ] {
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, sort_index, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                item_id(ALFA, 1),
                path,
                asset_type,
                sort_index,
                size,
                created_at
            ],
        )
        .expect("insert an asset");
    }

    // Every row a collection owns — items first, then their assets — bumps that
    // collection's `updated_at` through the `collection_activity_*` triggers, so
    // the fixture pins it back to a known value once the archive is complete.
    for (id, updated_at) in [(ALFA, 2_000_i64), (BETA, 2_100_i64)] {
        conn.execute(
            "UPDATE collections SET updated_at = ?1 WHERE id = ?2",
            params![updated_at, id],
        )
        .expect("pin the collection revision");
    }

    db_path
}

fn insert_item(
    conn: &rusqlite::Connection,
    id: &str,
    title: &str,
    collection_id: &str,
    metadata: Option<&str>,
    text: &str,
) {
    let index: i64 = id
        .rsplit('_')
        .next()
        .and_then(|index| index.parse().ok())
        .expect("the fixture id ends in its index");
    conn.execute(
        "INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            title,
            collection_id,
            metadata,
            1_000 + index,
            3_000 + index
        ],
    )
    .expect("insert an item");
    crate::nlp::fts::fts_index_item(conn, id, title, "", text).expect("index the item");
}
