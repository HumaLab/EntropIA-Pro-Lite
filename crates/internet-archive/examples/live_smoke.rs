//! Manual smoke test against the real Internet Archive with the production
//! policy. Not part of `cargo test`: run it by hand when the client changes.
//!
//! ```text
//! cargo run --example live_smoke
//! ```
//!
//! Exits non-zero on the first expectation that does not hold. It only issues
//! a few small public GETs and downloads no file.

use internet_archive::{
    resolve_identifier, Availability, Endpoints, FetchPolicy, IaClient, IaError, RestrictedReason,
    SearchQuery,
};

const OPEN_ITEM: &str = "30-festival-notas-para-una-contra-historia-del-cine-argentino";
const LENDING_ITEM: &str = "proceedingsofvic00cong";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("FAIL: {message}");
        std::process::exit(1);
    }
    println!("OK: all live checks passed");
}

async fn run() -> Result<(), String> {
    let client = IaClient::new(FetchPolicy::production());
    let endpoints = Endpoints::production();

    let query = SearchQuery {
        title: Some("Mar del Plata".to_string()),
        ..SearchQuery::default()
    };
    let page = client
        .search(&endpoints, &query, 1, 5)
        .await
        .map_err(|e| format!("search: {e}"))?;
    expect(page.num_found > 0, "search returned no results")?;
    expect(!page.items.is_empty(), "search page has no items")?;
    println!(
        "search: {} found, {} on page 1, {} restricted",
        page.num_found,
        page.items.len(),
        page.items.iter().filter(|item| item.restricted).count()
    );

    // Collection browsing goes through Advanced Search (plan decision D5)
    // with a stable identifier sort. `opensource` is a large, long-lived
    // top-level collection that is guaranteed to contain texts.
    let collection = client
        .list_collection(&endpoints, "opensource", 1, 5)
        .await
        .map_err(|e| format!("list collection: {e}"))?;
    expect(
        collection.num_found > 0,
        "collection listing returned no results",
    )?;
    expect(
        !collection.items.is_empty(),
        "collection listing page has no items",
    )?;
    println!(
        "collection opensource: {} found, {} on page 1",
        collection.num_found,
        collection.items.len()
    );

    let open = client
        .get_item(&endpoints, OPEN_ITEM)
        .await
        .map_err(|e| format!("open item: {e}"))?;
    expect(!open.restricted, "open item reported as restricted")?;
    expect(
        open.pdfs
            .iter()
            .any(|pdf| pdf.availability == Availability::Available && pdf.size.is_some()),
        "open item has no available PDF with a size",
    )?;
    println!("open item: {} PDF(s)", open.pdfs.len());

    let lending = client
        .get_item(&endpoints, LENDING_ITEM)
        .await
        .map_err(|e| format!("lending item: {e}"))?;
    expect(
        lending.restricted,
        "lending item not reported as restricted",
    )?;
    expect(
        lending
            .pdfs
            .iter()
            .all(|pdf| pdf.availability != Availability::Available),
        "lending item offers an available PDF",
    )?;
    expect(
        lending
            .pdfs
            .iter()
            .any(|pdf| pdf.availability == Availability::Restricted(RestrictedReason::Encrypted)),
        "lending item lists no encrypted PDF",
    )?;
    println!(
        "lending item: {} PDF(s), none available",
        lending.pdfs.len()
    );

    let resolved = resolve_identifier(&format!("https://archive.org/details/{OPEN_ITEM}"))
        .map_err(|e| format!("resolve: {e}"))?;
    expect(resolved == OPEN_ITEM, "details URL resolved to another id")?;

    match client
        .get_item(&endpoints, "entropia-no-such-item-0000")
        .await
    {
        Err(IaError::NotFound { .. }) => println!("unknown item: NotFound"),
        other => return Err(format!("unknown item: expected NotFound, got {other:?}")),
    }
    Ok(())
}

fn expect(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}
