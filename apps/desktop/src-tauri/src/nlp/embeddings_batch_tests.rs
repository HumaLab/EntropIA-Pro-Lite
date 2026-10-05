//! Transport-level tests for batched, bounded-concurrency remote embeddings.
//! A mock OpenAI-compatible `/embeddings` server records every request so the
//! batching, ordering, in-flight and throttling contracts are observable.

use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Instant;

/// What the mock answers for one request.
enum Reply {
    /// 200 with one vector per input.
    Vectors,
    /// A failure status with an optional `Retry-After` header value.
    Status(u16, Option<&'static str>),
}

struct Mock {
    endpoint: String,
    /// Parsed `input` arrays, in arrival order.
    requests: Arc<Mutex<Vec<Vec<String>>>>,
    max_in_flight: Arc<AtomicUsize>,
    /// Peak concurrency among requests numbered 3 and later: what is left
    /// once the first wave of throttles has been answered.
    late_peak: Arc<AtomicUsize>,
}

/// Every text is `"t-<n>"`; its vector is one-hot at `n`, so after the
/// engine's L2 normalisation the argmax still names the input it came from.
fn one_hot(text: &str) -> Vec<f32> {
    let n: usize = text
        .rsplit('-')
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);
    let mut vector = vec![0.0_f32; OPENROUTER_EMBEDDING_DIMENSIONS];
    vector[n % OPENROUTER_EMBEDDING_DIMENSIONS] = 1.0;
    vector
}

fn argmax(vector: &[f32]) -> usize {
    vector
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).expect("finite"))
        .map(|(index, _)| index)
        .expect("non-empty")
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut data = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
        if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while data.len() < header_end + length {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    Some(String::from_utf8_lossy(&data[header_end..header_end + length]).to_string())
}

fn start_mock(delay: Duration, script: impl Fn(usize) -> Reply + Send + Sync + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let late_peak = Arc::new(AtomicUsize::new(0));
    let counter = Arc::new(AtomicUsize::new(0));
    let script = Arc::new(script);
    let (requests_out, max_out, late_out) = (
        Arc::clone(&requests),
        Arc::clone(&max_in_flight),
        Arc::clone(&late_peak),
    );
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let requests = Arc::clone(&requests);
            let in_flight = Arc::clone(&in_flight);
            let max_in_flight = Arc::clone(&max_in_flight);
            let late_peak = Arc::clone(&late_peak);
            let counter = Arc::clone(&counter);
            let script = Arc::clone(&script);
            thread::spawn(move || {
                while let Some(body) = read_request(&mut stream) {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(now, Ordering::SeqCst);
                    let parsed: serde_json::Value =
                        serde_json::from_str(&body).expect("request body is JSON");
                    let inputs: Vec<String> = match &parsed["input"] {
                        serde_json::Value::Array(items) => items
                            .iter()
                            .map(|item| item.as_str().expect("string input").to_string())
                            .collect(),
                        serde_json::Value::String(text) => vec![text.clone()],
                        other => panic!("unexpected input shape: {other}"),
                    };
                    requests.lock().expect("requests").push(inputs.clone());
                    let number = counter.fetch_add(1, Ordering::SeqCst);
                    if number >= 3 {
                        late_peak.fetch_max(now, Ordering::SeqCst);
                    }
                    thread::sleep(delay);
                    let response = match script(number) {
                        Reply::Vectors => {
                            // Out-of-order `index` fields prove the client maps
                            // by index, not by arrival position.
                            let mut data: Vec<serde_json::Value> = inputs
                                .iter()
                                .enumerate()
                                .map(|(index, text)| {
                                    serde_json::json!({
                                        "index": index,
                                        "embedding": one_hot(text),
                                    })
                                })
                                .collect();
                            data.reverse();
                            let body = serde_json::json!({ "data": data }).to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        Reply::Status(code, retry_after) => {
                            let extra = retry_after
                                .map(|value| format!("Retry-After: {value}\r\n"))
                                .unwrap_or_default();
                            let body = "{\"error\":\"throttled\"}";
                            format!(
                                "HTTP/1.1 {code} Status\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                    };
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    if stream.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    Mock {
        endpoint,
        requests: requests_out,
        max_in_flight: max_out,
        late_peak: late_out,
    }
}

fn engine_for(mock: &Mock, policy: EmbeddingRequestPolicy) -> EmbeddingEngine {
    EmbeddingEngine::init_with_endpoint_and_policy(
        EmbeddingConfig::openrouter("sk-test".to_string(), "baai/bge-m3".to_string()),
        mock.endpoint.clone(),
        policy,
    )
    .expect("engine")
}

fn policy(max_inputs: usize, max_in_flight: usize) -> EmbeddingRequestPolicy {
    EmbeddingRequestPolicy {
        max_inputs,
        max_chars: usize::MAX,
        max_in_flight,
        max_attempts: 4,
        base_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(500),
    }
}

fn texts(count: usize) -> Vec<String> {
    (0..count).map(|n| format!("t-{n}")).collect()
}

fn refs(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

#[test]
fn n_texts_take_ceil_n_over_b_requests_and_keep_input_order() {
    let mock = start_mock(Duration::ZERO, |_| Reply::Vectors);
    let engine = engine_for(&mock, policy(32, 1));
    let items = texts(70);

    let vectors = engine.embed_batch(&refs(&items)).expect("batch");

    assert_eq!(mock.requests.lock().unwrap().len(), 3, "ceil(70 / 32)");
    assert_eq!(vectors.len(), 70);
    for (n, vector) in vectors.iter().enumerate() {
        assert_eq!(argmax(vector), n, "vector {n} must belong to text {n}");
    }
    let sent: Vec<String> = mock
        .requests
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .cloned()
        .collect();
    let mut sorted = sent.clone();
    sorted.sort();
    let mut expected = items.clone();
    expected.sort();
    assert_eq!(sorted, expected, "every text is sent exactly once");
}

#[test]
fn a_request_never_exceeds_the_character_budget() {
    let mock = start_mock(Duration::ZERO, |_| Reply::Vectors);
    let mut limited = policy(32, 1);
    // "t-N" is 3 chars: 3 texts per request at most.
    limited.max_chars = 9;
    let engine = engine_for(&mock, limited);
    let items = texts(10);

    let vectors = engine.embed_batch(&refs(&items)).expect("batch");

    let requests = mock.requests.lock().unwrap();
    assert!(requests.len() >= 4, "10 texts / 3 per request");
    for request in requests.iter() {
        let chars: usize = request.iter().map(|text| text.chars().count()).sum();
        assert!(chars <= 9, "request carried {chars} chars");
    }
    for (n, vector) in vectors.iter().enumerate() {
        assert_eq!(argmax(vector), n);
    }
}

#[test]
fn in_flight_requests_stay_within_the_bound() {
    let mock = start_mock(Duration::from_millis(80), |_| Reply::Vectors);
    let engine = engine_for(&mock, policy(2, 3));
    let items = texts(24);

    let vectors = engine.embed_batch(&refs(&items)).expect("batch");

    assert_eq!(mock.requests.lock().unwrap().len(), 12);
    let peak = mock.max_in_flight.load(Ordering::SeqCst);
    assert!(peak <= 3, "never more than 3 in flight, saw {peak}");
    assert!(peak >= 2, "requests must actually overlap, saw {peak}");
    for (n, vector) in vectors.iter().enumerate() {
        assert_eq!(argmax(vector), n, "order survives concurrency");
    }
}

#[test]
fn a_429_with_retry_after_is_retried_and_then_succeeds() {
    let mock = start_mock(Duration::ZERO, |number| {
        if number < 2 {
            Reply::Status(429, Some("0"))
        } else {
            Reply::Vectors
        }
    });
    let engine = engine_for(&mock, policy(32, 1));
    let items = texts(5);

    let vectors = engine.embed_batch(&refs(&items)).expect("retried batch");

    assert_eq!(vectors.len(), 5);
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        3,
        "two throttled attempts plus the success"
    );
    for (n, vector) in vectors.iter().enumerate() {
        assert_eq!(argmax(vector), n);
    }
}

#[test]
fn a_persistent_429_surfaces_a_retryable_error_with_its_delay() {
    let mock = start_mock(Duration::ZERO, |_| Reply::Status(429, Some("0")));
    let engine = engine_for(&mock, policy(32, 1));
    let items = texts(3);

    let error = engine
        .embed_batch(&refs(&items))
        .expect_err("a persistent throttle must fail");

    assert!(error.contains("429"), "{error}");
    assert!(error.contains("[retry_after_ms=0]"), "{error}");
    assert_eq!(mock.requests.lock().unwrap().len(), 4, "max_attempts");
}

#[test]
fn a_long_retry_after_is_handed_to_the_scheduler_not_slept_in_process() {
    let mock = start_mock(Duration::ZERO, |_| Reply::Status(429, Some("120")));
    let engine = engine_for(&mock, policy(32, 1));
    let items = texts(3);
    let started = Instant::now();

    let error = engine
        .embed_batch(&refs(&items))
        .expect_err("a long throttle must fail fast");

    assert!(error.contains("[retry_after_ms=120000]"), "{error}");
    assert_eq!(
        mock.requests.lock().unwrap().len(),
        1,
        "no in-process retry"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_non_retryable_status_is_not_retried() {
    let mock = start_mock(Duration::ZERO, |_| Reply::Status(401, None));
    let engine = engine_for(&mock, policy(32, 1));
    let items = texts(3);

    let error = engine.embed_batch(&refs(&items)).expect_err("401");

    assert!(error.contains("401"), "{error}");
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_failed_batch_stops_new_requests_and_fails_the_whole_call() {
    let mock = start_mock(Duration::from_millis(30), |number| {
        if number == 0 {
            Reply::Status(401, None)
        } else {
            Reply::Vectors
        }
    });
    let engine = engine_for(&mock, policy(1, 2));
    let items = texts(40);

    let error = engine.embed_batch(&refs(&items)).expect_err("fails");

    assert!(error.contains("401"), "{error}");
    assert!(
        mock.requests.lock().unwrap().len() < 40,
        "an aborted call must not fan out the remaining requests"
    );
}

/// Long-running: 400 chunks against a server that sleeps ~300 ms per request.
/// `cargo test bench_400_chunks -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_400_chunks_at_300ms_per_request() {
    let items = texts(400);

    let mock = start_mock(Duration::from_millis(300), |_| Reply::Vectors);
    let engine = engine_for(&mock, EmbeddingRequestPolicy::default());
    let started = Instant::now();
    let new_path = engine.embed_batch(&refs(&items)).expect("batched");
    let after = started.elapsed();
    let new_requests = mock.requests.lock().unwrap().len();

    // The previous behaviour: one blocking request per chunk.
    let legacy = start_mock(Duration::from_millis(300), |_| Reply::Vectors);
    let engine = engine_for(&legacy, policy(1, 1));
    let started = Instant::now();
    for item in &items {
        engine.embed_text(item).expect("single");
    }
    let before = started.elapsed();
    let old_requests = legacy.requests.lock().unwrap().len();

    println!(
        "BENCH 400 chunks @300ms/request: before = {:.1}s ({old_requests} requests), after = {:.1}s ({new_requests} requests)",
        before.as_secs_f64(),
        after.as_secs_f64()
    );
    assert_eq!(new_path.len(), 400);
    assert!(after < before / 4);
}

#[test]
fn a_429_shrinks_the_concurrency_for_the_rest_of_the_call() {
    // The first wave of three requests is throttled; every later request
    // succeeds. The retries sit well after the replies, so what overlaps from
    // request 3 on reflects the concurrency the call settled on.
    let mock = start_mock(Duration::from_millis(20), |number| {
        if number < 3 {
            Reply::Status(429, None)
        } else {
            Reply::Vectors
        }
    });
    let mut adaptive = policy(1, 3);
    adaptive.base_backoff = Duration::from_millis(150);
    adaptive.max_backoff = Duration::from_millis(600);
    let engine = engine_for(&mock, adaptive);
    let items = texts(12);

    let vectors = engine.embed_batch(&refs(&items)).expect("recovers");

    for (n, vector) in vectors.iter().enumerate() {
        assert_eq!(argmax(vector), n, "order survives the shrink");
    }
    assert!(
        mock.max_in_flight.load(Ordering::SeqCst) >= 2,
        "the call starts at full concurrency"
    );
    let late = mock.late_peak.load(Ordering::SeqCst);
    assert_eq!(late, 1, "after a 429 only one request flies, saw {late}");
}

#[test]
fn backoff_without_retry_after_grows_and_is_jittered_within_bounds() {
    let base = Duration::from_millis(100);
    let cap = Duration::from_millis(1_000);
    for attempt in 1..=6_usize {
        let nominal = base.saturating_mul(1_u32 << (attempt - 1)).min(cap);
        let low = nominal.mul_f64(0.74);
        let high = nominal.mul_f64(1.26).min(cap.mul_f64(1.26));
        for _ in 0..50 {
            let delay = jittered_backoff(base, cap, attempt);
            assert!(
                delay >= low && delay <= high,
                "attempt {attempt}: {delay:?}"
            );
        }
    }
    let samples: std::collections::HashSet<_> = (0..50)
        .map(|_| jittered_backoff(base, cap, 3).as_micros())
        .collect();
    assert!(samples.len() > 1, "jitter must vary the delay");
}
