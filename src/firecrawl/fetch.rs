//! Cache-aware Firecrawl credit fetch with optional historical detail.

use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::cache::{Cache, MAX_STALE, acquire_lock_async};
use crate::error::{AppError, Result};
use crate::usage::FirecrawlSnapshot;
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

use super::types::{CurrentData, CurrentEnvelope, HistoricalEnvelope, HistoricalPeriod};

pub const BASE_URL: &str = "https://api.firecrawl.dev/v2";
pub const SCHEMA_DRIFT_MESSAGE: &str = "Firecrawl API schema drift";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const PARTIAL_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub current: String,
    pub historical: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            current: format!("{BASE_URL}/team/credit-usage"),
            historical: format!("{BASE_URL}/team/credit-usage/historical?byApiKey=false"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub snapshot: FirecrawlSnapshot,
    pub stale: bool,
    pub last_error: Option<(u16, String)>,
    pub cache_age: Option<Duration>,
}

pub async fn fetch_snapshot(
    client: &reqwest::Client,
    api_key: &str,
    cache: &Cache,
    endpoints: &Endpoints,
    cache_ttl: Duration,
) -> Result<FetchOutcome> {
    cache.ensure_dir()?;
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    let target = target_key(endpoints, api_key);

    if let Some(bytes) = cache.maybe_payload()? {
        let partial = cache_is_partial(&bytes);
        // A schema marker means the current binary understands a different
        // upstream shape than the one that created the partial cache. Retry
        // immediately rather than trapping a corrected parser behind the
        // normal five-minute partial backoff.
        let schema_drift = cache
            .read_last_error()
            .is_some_and(|(_, message)| message == SCHEMA_DRIFT_MESSAGE);
        let ttl = if partial { PARTIAL_TTL } else { cache_ttl };
        if !schema_drift
            && cache.payload_age().is_some_and(|age| age < ttl)
            && let Ok(snapshot) = parse_cache(&bytes, &target)
        {
            return Ok(FetchOutcome {
                snapshot,
                stale: false,
                last_error: cache.read_last_error(),
                cache_age: cache.payload_age(),
            });
        }
    }

    let (current, historical) = tokio::join!(
        fetch_current(client, &endpoints.current, api_key),
        fetch_historical(client, &endpoints.historical, api_key),
    );

    let current = match current {
        Ok(current) => current,
        Err(error @ AppError::Transport(_)) => return fallback_silent(cache, &target, error),
        Err(error) => {
            let pair = error_to_pair(&error);
            cache.mark_stale();
            cache.write_last_error(pair.0, &pair.1);
            return fallback_with_error(cache, &target, pair, error);
        }
    };

    let current = match validate_current(current, Utc::now()) {
        Ok(current) => current,
        Err(error) => {
            let pair = error_to_pair(&error);
            cache.mark_stale();
            cache.write_last_error(pair.0, &pair.1);
            return fallback_with_error(cache, &target, pair, error);
        }
    };
    let (period_consumed, secondary_error, partial) = match historical {
        Ok(periods) => match match_period(&current, periods, Utc::now()) {
            Ok(consumed) => (consumed, None, false),
            Err(error) => (None, Some(error_to_pair(&error)), true),
        },
        Err(error) => (None, Some(error_to_pair(&error)), true),
    };

    let snapshot = FirecrawlSnapshot {
        remaining_credits: current.remaining_credits,
        plan_credits: current.plan_credits,
        billing_period_start: current.billing_period_start,
        billing_period_end: current.billing_period_end,
        period_consumed,
        scope_fingerprint: target.clone(),
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "target": target,
        "partial": partial,
        "response": snapshot_repr(&snapshot),
    }))?;
    cache.write_payload(&body)?;
    if let Some((code, message)) = &secondary_error {
        cache.write_last_error(*code, message);
    }
    Ok(FetchOutcome {
        snapshot,
        stale: false,
        last_error: secondary_error,
        cache_age: Some(Duration::ZERO),
    })
}

fn fallback_silent(cache: &Cache, target: &str, original: AppError) -> Result<FetchOutcome> {
    let Some(bytes) = cache.fallback_payload(MAX_STALE)? else {
        return Err(original);
    };
    reuse_cache(bytes, cache, target, true).or(Err(original))
}

fn fallback_with_error(
    cache: &Cache,
    target: &str,
    error_pair: (u16, String),
    original: AppError,
) -> Result<FetchOutcome> {
    let Some(bytes) = cache.fallback_payload(MAX_STALE)? else {
        return Err(original);
    };
    let mut outcome = reuse_cache(bytes, cache, target, true)?;
    outcome.last_error = Some(error_pair);
    Ok(outcome)
}

fn error_to_pair(error: &AppError) -> (u16, String) {
    match error {
        AppError::Http { status, body } => (*status, body.clone()),
        AppError::Schema(_) => (0, SCHEMA_DRIFT_MESSAGE.to_string()),
        _ => (0, error.to_string()),
    }
}

fn target_key(endpoints: &Endpoints, api_key: &str) -> String {
    let digest = Sha256::digest(api_key.as_bytes());
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(fingerprint, "{byte:02x}");
    }
    format!("{}|key:{fingerprint}", endpoints.current)
}

fn cache_is_partial(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("partial").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn snapshot_repr(snapshot: &FirecrawlSnapshot) -> serde_json::Value {
    serde_json::json!({
        "remaining_credits": snapshot.remaining_credits,
        "plan_credits": snapshot.plan_credits,
        "billing_period_start": snapshot.billing_period_start.map(|date| date.to_rfc3339()),
        "billing_period_end": snapshot.billing_period_end.map(|date| date.to_rfc3339()),
        "period_consumed": snapshot.period_consumed,
    })
}

fn parse_cache(bytes: &[u8], target: &str) -> Result<FirecrawlSnapshot> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target) {
        return Err(AppError::Schema(
            "Firecrawl cache scope fingerprint mismatch".into(),
        ));
    }
    let response = value
        .get("response")
        .ok_or_else(|| AppError::Schema("Firecrawl cache missing response".into()))?;
    Ok(FirecrawlSnapshot {
        remaining_credits: required_u64(response, "remaining_credits")?,
        plan_credits: required_u64(response, "plan_credits")?,
        billing_period_start: optional_date(response, "billing_period_start")?,
        billing_period_end: optional_date(response, "billing_period_end")?,
        period_consumed: optional_u64(response, "period_consumed")?,
        scope_fingerprint: target.to_string(),
    })
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::Schema(format!("Firecrawl cache missing {field}")))
}

fn optional_u64(value: &serde_json::Value, field: &str) -> Result<Option<u64>> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| AppError::Schema(format!("Firecrawl cache invalid {field}"))),
    }
}

fn optional_date(value: &serde_json::Value, field: &str) -> Result<Option<DateTime<Utc>>> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .ok_or_else(|| AppError::Schema(format!("Firecrawl cache invalid {field}")))?
            .parse::<DateTime<Utc>>()
            .map(Some)
            .map_err(|error| AppError::Schema(format!("Firecrawl cache invalid {field}: {error}"))),
    }
}

fn validate_current(data: CurrentData, now: DateTime<Utc>) -> Result<CurrentData> {
    if let Some(start) = data.billing_period_start {
        if start > now {
            return Err(AppError::Schema(
                "Firecrawl billing period starts in the future".into(),
            ));
        }
        if let Some(end) = data.billing_period_end
            && end <= start
        {
            return Err(AppError::Schema(
                "Firecrawl billing period end must be after start".into(),
            ));
        }
    }
    Ok(data)
}

fn match_period(
    current: &CurrentData,
    periods: Vec<HistoricalPeriod>,
    now: DateTime<Utc>,
) -> Result<Option<u64>> {
    for period in &periods {
        if period.start_date > now || period.end_date.is_some_and(|end| end <= period.start_date) {
            return Err(AppError::Schema(
                "Firecrawl historical billing period is invalid".into(),
            ));
        }
    }
    let Some(start) = current.billing_period_start else {
        return Ok(None);
    };
    let matches: Vec<&HistoricalPeriod> = periods
        .iter()
        .filter(|period| {
            period.start_date <= start && period.end_date.map(|end| start < end).unwrap_or(true)
        })
        .collect();
    if matches.len() > 1 {
        return Err(AppError::Schema(
            "Firecrawl historical usage contains duplicate billing periods".into(),
        ));
    }
    Ok(matches.first().map(|period| period.total_credits))
}

async fn fetch_current(client: &reqwest::Client, url: &str, api_key: &str) -> Result<CurrentData> {
    let envelope: CurrentEnvelope = fetch_one(client, url, api_key).await?;
    envelope.into_data()
}

async fn fetch_historical(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
) -> Result<Vec<HistoricalPeriod>> {
    let envelope: HistoricalEnvelope = fetch_one(client, url, api_key).await?;
    envelope.into_periods()
}

async fn fetch_one<T: for<'de> serde::Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
) -> Result<T> {
    let response = tokio::time::timeout(
        HTTP_TIMEOUT,
        client
            .get(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| AppError::Transport(format!("Firecrawl request timed out: {url}")))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        let message = if matches!(status.as_u16(), 401 | 403) {
            "Firecrawl authentication failed".to_string()
        } else {
            format!("Firecrawl API returned HTTP {}", status.as_u16())
        };
        return Err(AppError::Http {
            status: status.as_u16(),
            body: message,
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("Firecrawl response schema: {error}")))
}

fn reuse_cache(bytes: Vec<u8>, cache: &Cache, target: &str, stale: bool) -> Result<FetchOutcome> {
    Ok(FetchOutcome {
        snapshot: parse_cache(&bytes, target)?,
        stale,
        last_error: cache.read_last_error(),
        cache_age: cache.payload_age(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cache_fixture() -> (TempDir, Cache) {
        let dir = TempDir::new().unwrap();
        let cache = Cache::at(dir.path().join("firecrawl"));
        cache.ensure_dir().unwrap();
        (dir, cache)
    }

    fn endpoints(server: &mockito::ServerGuard) -> Endpoints {
        Endpoints {
            current: format!("{}/v2/team/credit-usage", server.url()),
            historical: format!(
                "{}/v2/team/credit-usage/historical?byApiKey=false",
                server.url()
            ),
        }
    }

    fn current_body() -> &'static str {
        r#"{"success":true,"data":{"remainingCredits":8200,"planCredits":10000,"billingPeriodStart":"2026-08-01T00:00:00Z","billingPeriodEnd":"2026-09-01T00:00:00Z"}}"#
    }

    fn historical_body() -> &'static str {
        r#"{"success":true,"periods":[{"startDate":"2026-08-01T00:00:00Z","endDate":"2026-09-01T00:00:00Z","apiKey":null,"totalCredits":1800}]}"#
    }

    #[tokio::test]
    async fn current_and_matching_historical_data_combine() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .match_header("authorization", "Bearer fc-test")
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded(
                "byApiKey".into(),
                "false".into(),
            ))
            .with_status(200)
            .with_body(historical_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "fc-test",
            &cache,
            &endpoints(&server),
            Duration::from_secs(0),
        )
        .await
        .unwrap();
        assert!(!out.stale);
        assert_eq!(out.snapshot.remaining_credits, 8200);
        assert_eq!(out.snapshot.plan_credits, 10000);
        assert_eq!(out.snapshot.period_consumed, Some(1800));
        assert_eq!(out.snapshot.period_pct(), Some(18));
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn current_mid_month_period_matches_ongoing_month_history() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .with_status(200)
            .with_body(r#"{"success":true,"data":{"remainingCredits":1617,"planCredits":1000,"billingPeriodStart":"2026-08-14T02:41:45Z","billingPeriodEnd":"2026-09-14T02:41:45Z"}}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded("byApiKey".into(), "false".into()))
            .with_status(200)
            .with_body(r#"{"success":true,"periods":[{"startDate":"2026-08-01T00:00:00Z","endDate":null,"apiKey":null,"creditsUsed":91}]}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "fc-test",
            &cache,
            &endpoints(&server),
            Duration::from_secs(0),
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.period_consumed, Some(91));
        assert_eq!(out.snapshot.period_pct(), Some(9));
    }

    #[tokio::test]
    async fn historical_failure_keeps_live_primary_as_partial() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded(
                "byApiKey".into(),
                "false".into(),
            ))
            .with_status(503)
            .with_body("temporarily unavailable")
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "fc-test",
            &cache,
            &endpoints(&server),
            Duration::from_secs(0),
        )
        .await
        .unwrap();
        assert!(!out.stale);
        assert_eq!(out.snapshot.remaining_credits, 8200);
        assert!(out.snapshot.period_consumed.is_none());
        assert_eq!(out.last_error.as_ref().map(|(code, _)| *code), Some(503));
        assert!(!cache.is_stale());
        assert!(cache.payload_age().unwrap() < PARTIAL_TTL);
    }

    #[tokio::test]
    async fn unmatched_history_is_absent_not_arbitrarily_selected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded("byApiKey".into(), "false".into()))
            .with_status(200)
            .with_body(r#"{"success":true,"periods":[{"startDate":"2026-07-01T00:00:00Z","endDate":"2026-08-01T00:00:00Z","totalCredits":9500}]}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "fc-test",
            &cache,
            &endpoints(&server),
            Duration::from_secs(0),
        )
        .await
        .unwrap();
        assert!(out.snapshot.period_consumed.is_none());
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn fresh_partial_cache_is_served_for_five_minutes() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .with_status(503)
            .with_body("down")
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let api_key = "fc-test";
        let endpoints = endpoints(&server);
        let first = fetch_snapshot(
            &reqwest::Client::new(),
            api_key,
            &cache,
            &endpoints,
            Duration::from_secs(0),
        )
        .await
        .unwrap();
        assert!(!first.stale);
        let second = fetch_snapshot(
            &reqwest::Client::new(),
            api_key,
            &cache,
            &endpoints,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(!second.stale);
        assert!(second.last_error.is_some());
    }

    #[tokio::test]
    async fn schema_drift_partial_cache_refetches_immediately() {
        let mut server = mockito::Server::new_async().await;
        let current = server
            .mock("GET", "/v2/team/credit-usage")
            .expect(1)
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        let historical = server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded(
                "byApiKey".into(),
                "false".into(),
            ))
            .expect(1)
            .with_status(200)
            .with_body(historical_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        let target = target_key(&endpoints, "fc-test");
        let old = FirecrawlSnapshot {
            remaining_credits: 8200,
            plan_credits: 10000,
            billing_period_start: Some("2026-08-01T00:00:00Z".parse().unwrap()),
            billing_period_end: Some("2026-09-01T00:00:00Z".parse().unwrap()),
            period_consumed: None,
            scope_fingerprint: target.clone(),
        };
        cache
            .write_payload(
                &serde_json::to_vec(&serde_json::json!({
                    "target": target,
                    "partial": true,
                    "response": snapshot_repr(&old),
                }))
                .unwrap(),
            )
            .unwrap();
        cache.write_last_error(0, SCHEMA_DRIFT_MESSAGE);

        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "fc-test",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        current.assert_async().await;
        historical.assert_async().await;
        assert_eq!(out.snapshot.period_consumed, Some(1800));
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn changed_key_does_not_reuse_cache() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v2/team/credit-usage")
            .expect(2)
            .with_status(200)
            .with_body(current_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v2/team/credit-usage/historical")
            .match_query(mockito::Matcher::UrlEncoded(
                "byApiKey".into(),
                "false".into(),
            ))
            .expect(2)
            .with_status(200)
            .with_body(historical_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "old-key",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "new-key",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.period_consumed, Some(1800));
        assert!(out.snapshot.scope_fingerprint.contains("|key:"));
    }
}
