pub mod commands;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

#[derive(Debug)]
struct EntityGeoCandidate {
    entity_id: String,
    value: String,
}

// ---------------------------------------------------------------------------
// Nominatim response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct NominatimResult {
    lat: String,
    lon: String,
    display_name: String,
}

fn parse_nominatim_coordinates(result: &NominatimResult) -> Result<(f64, f64), String> {
    let latitude = result
        .lat
        .parse::<f64>()
        .map_err(|_| "Invalid latitude from Nominatim".to_string())?;
    let longitude = result
        .lon
        .parse::<f64>()
        .map_err(|_| "Invalid longitude from Nominatim".to_string())?;

    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        return Err("Invalid latitude from Nominatim".to_string());
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        return Err("Invalid longitude from Nominatim".to_string());
    }

    Ok((latitude, longitude))
}

// ---------------------------------------------------------------------------
// Job definition
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum GeoJob {
    GeocodeEntity { entity_id: String },
    GeocodeItemEntities { item_id: String },
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct GeoCompletePayload {
    pub entity_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub display_name: String,
}

#[derive(Clone, Serialize)]
pub struct GeoItemCompletePayload {
    pub item_id: String,
    pub geocoded_count: usize,
    pub not_found_count: usize,
}

#[derive(Clone, Serialize)]
pub struct GeoErrorPayload {
    pub id: String,
    pub error: String,
}

fn emit_entity_complete(app_handle: &AppHandle, payload: &GeoCompletePayload) {
    let _ = app_handle.emit("geo:entity-complete", payload.clone());
}

fn emit_item_complete(app_handle: &AppHandle, payload: &GeoItemCompletePayload) {
    let _ = app_handle.emit("geo:item-complete", payload.clone());
}

fn emit_error(app_handle: &AppHandle, id: &str, error: &str) {
    let _ = app_handle.emit(
        "geo:error",
        GeoErrorPayload {
            id: id.to_string(),
            error: error.to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

pub struct GeoQueue {
    sender: mpsc::Sender<GeoJob>,
}

impl GeoQueue {
    pub fn new() -> (Self, mpsc::Receiver<GeoJob>) {
        let (sender, receiver) = mpsc::channel::<GeoJob>(64);
        (Self { sender }, receiver)
    }

    pub fn submit(&self, job: GeoJob) -> Result<(), String> {
        self.sender
            .try_send(job)
            .map_err(|e| format!("Geo queue full or closed: {e}"))
    }

    pub fn start_worker(
        db_path: PathBuf,
        mut receiver: mpsc::Receiver<GeoJob>,
        app_handle: AppHandle,
    ) {
        tauri::async_runtime::spawn(async move {
            let conn = match rusqlite::Connection::open(&db_path) {
                Ok(c) => {
                    let _ = c.execute_batch(
                        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
                    );
                    c
                }
                Err(e) => {
                    eprintln!("[geo] Failed to open worker DB connection: {e}");
                    return;
                }
            };

            let client = match reqwest::Client::builder()
                .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(15))
                .build()
            {
                Ok(client) => client,
                Err(e) => {
                    eprintln!("[geo] Failed to build HTTP client: {e}");
                    return;
                }
            };

            let mut last_nominatim: Option<Instant> = None;
            while let Some(job) = receiver.recv().await {
                match job {
                    GeoJob::GeocodeEntity { entity_id } => {
                        match fetch_geocode_candidate(&conn, &entity_id) {
                            Ok(Some(candidate)) => {
                                match throttled_nominatim_search(
                                    &client,
                                    &candidate.value,
                                    &mut last_nominatim,
                                )
                                .await
                                {
                                    Ok(Some(result)) => {
                                        let (lat, lon) = match parse_nominatim_coordinates(&result)
                                        {
                                            Ok(coordinates) => coordinates,
                                            Err(error) => {
                                                emit_error(&app_handle, &entity_id, &error);
                                                continue;
                                            }
                                        };

                                        match conn.execute(
                                        "UPDATE entities SET latitude = ?1, longitude = ?2, geo_status = 'resolved' WHERE id = ?3",
                                        params![lat, lon, &entity_id],
                                    ) {
                                        Ok(_) => emit_entity_complete(
                                            &app_handle,
                                            &GeoCompletePayload {
                                                entity_id: entity_id.clone(),
                                                latitude: lat,
                                                longitude: lon,
                                                display_name: result.display_name,
                                            },
                                        ),
                                        Err(e) => emit_error(&app_handle, &entity_id, &format!("Failed to update entity coordinates: {e}")),
                                    }
                                    }
                                    Ok(None) => {
                                        let _ = conn.execute(
                                        "UPDATE entities SET geo_status = 'not_found' WHERE id = ?1",
                                        params![&entity_id],
                                    );
                                    }
                                    Err(e) => emit_error(&app_handle, &entity_id, &e),
                                }
                            }
                            Ok(None) => {}
                            Err(e) => emit_error(&app_handle, &entity_id, &e),
                        }
                    }
                    GeoJob::GeocodeItemEntities { item_id } => {
                        let candidates = match fetch_pending_place_candidates(&conn, &item_id) {
                            Ok(candidates) => candidates,
                            Err(e) => {
                                emit_error(&app_handle, &item_id, &e);
                                continue;
                            }
                        };

                        let mut geocoded_count = 0usize;
                        let mut not_found_count = 0usize;

                        for candidate in candidates.iter() {
                            match throttled_nominatim_search(
                                &client,
                                &candidate.value,
                                &mut last_nominatim,
                            )
                            .await
                            {
                                Ok(Some(result)) => {
                                    let (lat, lon) = match parse_nominatim_coordinates(&result) {
                                        Ok(coordinates) => coordinates,
                                        Err(error) => {
                                            emit_error(&app_handle, &candidate.entity_id, &error);
                                            continue;
                                        }
                                    };

                                    match conn.execute(
                                        "UPDATE entities SET latitude = ?1, longitude = ?2, geo_status = 'resolved' WHERE id = ?3",
                                        params![lat, lon, &candidate.entity_id],
                                    ) {
                                        Ok(_) => {
                                            geocoded_count += 1;
                                            emit_entity_complete(
                                                &app_handle,
                                                &GeoCompletePayload {
                                                    entity_id: candidate.entity_id.clone(),
                                                    latitude: lat,
                                                    longitude: lon,
                                                    display_name: result.display_name,
                                                },
                                            );
                                        }
                                        Err(e) => emit_error(&app_handle, &candidate.entity_id, &format!("Failed to update entity coordinates: {e}")),
                                    }
                                }
                                Ok(None) => {
                                    let _ = conn.execute(
                                        "UPDATE entities SET geo_status = 'not_found' WHERE id = ?1",
                                        params![&candidate.entity_id],
                                    );
                                    not_found_count += 1;
                                }
                                Err(e) => emit_error(&app_handle, &candidate.entity_id, &e),
                            }
                        }

                        emit_item_complete(
                            &app_handle,
                            &GeoItemCompletePayload {
                                item_id,
                                geocoded_count,
                                not_found_count,
                            },
                        );
                    }
                }
            }

            eprintln!("[geo] Worker loop ended — channel closed.");
        });
    }
}

// ---------------------------------------------------------------------------
// Geocoding logic
// ---------------------------------------------------------------------------

/// Minimum spacing between Nominatim requests (their usage policy is 1 req/s).
const NOMINATIM_MIN_INTERVAL: Duration = Duration::from_millis(1100);

/// Delay to wait before the next Nominatim request: `min_interval` minus the time
/// already elapsed since the last request, clamped to zero. Pure, testable core of
/// the global throttle (#22).
fn throttle_delay(last: Option<Instant>, now: Instant, min_interval: Duration) -> Duration {
    match last {
        Some(last) => min_interval.saturating_sub(now.duration_since(last)),
        None => Duration::ZERO,
    }
}

/// Calls `nominatim_search` after honouring the global 1 req/s throttle, then
/// records the request time. Routing single + batch geocoding through one shared
/// `last_request` clock spaces single calls, the first of a batch, AND cross-job
/// bursts (#22).
async fn throttled_nominatim_search(
    client: &reqwest::Client,
    query: &str,
    last_request: &mut Option<Instant>,
) -> Result<Option<NominatimResult>, String> {
    let delay = throttle_delay(*last_request, Instant::now(), NOMINATIM_MIN_INTERVAL);
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let result = nominatim_search(client, query).await;
    *last_request = Some(Instant::now());
    result
}

async fn nominatim_search(
    client: &reqwest::Client,
    query: &str,
) -> Result<Option<NominatimResult>, String> {
    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1",
        urlencoding::encode(query)
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Nominatim request failed: {e}"))?;

    let results: Vec<NominatimResult> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Nominatim response: {e}"))?;

    Ok(results.into_iter().next())
}

fn fetch_geocode_candidate(
    conn: &rusqlite::Connection,
    entity_id: &str,
) -> Result<Option<EntityGeoCandidate>, String> {
    let (value, entity_type): (String, String) = conn
        .query_row(
            "SELECT value, entity_type FROM entities WHERE id = ?1",
            params![entity_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("Entity not found: {e}"))?;

    if entity_type != "place" {
        conn.execute(
            "UPDATE entities SET geo_status = 'skipped' WHERE id = ?1",
            params![entity_id],
        )
        .ok();
        return Ok(None);
    }

    Ok(Some(EntityGeoCandidate {
        entity_id: entity_id.to_string(),
        value,
    }))
}

fn fetch_pending_place_candidates(
    conn: &rusqlite::Connection,
    item_id: &str,
) -> Result<Vec<EntityGeoCandidate>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, value FROM entities
             WHERE item_id = ?1
               AND entity_type = 'place'
               AND geo_status = 'pending'
               AND (source IS NULL OR source != 'manual_deleted')",
        )
        .map_err(|e| format!("Failed to query entities: {e}"))?;

    let candidates: Vec<EntityGeoCandidate> = stmt
        .query_map(params![item_id], |row| {
            Ok(EntityGeoCandidate {
                entity_id: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .map_err(|e| format!("Failed to fetch entities: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(candidates)
}

/// Auto-trigger: call this after NER completes for an item.
pub fn enqueue_geocoding_for_item(geo_queue: &GeoQueue, item_id: &str) -> Result<(), String> {
    geo_queue.submit(GeoJob::GeocodeItemEntities {
        item_id: item_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_nominatim_coordinates, throttle_delay, NominatimResult};
    use std::time::{Duration, Instant};

    #[test]
    fn throttle_delay_waits_remaining_interval_and_clamps_to_zero() {
        let min = Duration::from_millis(1100);
        let now = Instant::now();
        // No prior request -> no wait (#22).
        assert_eq!(throttle_delay(None, now, min), Duration::ZERO);
        // 400ms since the last request -> wait the remaining 700ms.
        let recent = now - Duration::from_millis(400);
        assert_eq!(
            throttle_delay(Some(recent), now, min),
            Duration::from_millis(700)
        );
        // More than the interval elapsed -> no wait.
        let stale = now - Duration::from_millis(2000);
        assert_eq!(throttle_delay(Some(stale), now, min), Duration::ZERO);
    }

    #[test]
    fn buenos_aires_coordinates_preserve_southern_and_western_signs() {
        let result = NominatimResult {
            lat: "-34.6037".to_string(),
            lon: "-58.3816".to_string(),
            display_name: "Buenos Aires, Argentina".to_string(),
        };

        assert_eq!(
            parse_nominatim_coordinates(&result),
            Ok((-34.6037, -58.3816))
        );
    }

    #[test]
    fn nominatim_coordinates_reject_non_finite_and_out_of_range_values() {
        let invalid_latitude = NominatimResult {
            lat: "NaN".to_string(),
            lon: "-58.3816".to_string(),
            display_name: String::new(),
        };
        let invalid_longitude = NominatimResult {
            lat: "-34.6037".to_string(),
            lon: "181".to_string(),
            display_name: String::new(),
        };

        assert!(parse_nominatim_coordinates(&invalid_latitude).is_err());
        assert!(parse_nominatim_coordinates(&invalid_longitude).is_err());
    }
}
