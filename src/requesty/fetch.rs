//! Cache-aware Requesty organization balance and month-to-date usage fetch.

use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Datelike, SecondsFormat, TimeZone, Timelike, Utc};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::cache::{Cache, MAX_STALE, acquire_lock_async};
use crate::error::{AppError, Result};
use crate::usage::{RequestySnapshot, RequestyUsage, finite_amount};
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

use super::types::{OrganizationResponse, UsageResponse};

pub const BASE_URL: &str = "https://api-v2.requesty.ai/v1/manage";
pub const SCHEMA_DRIFT_MESSAGE: &str = "Requesty API schema drift";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const PARTIAL_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub organization: String,
    pub usage: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            organization: format!("{BASE_URL}/org"),
            usage: format!("{BASE_URL}/org/usage"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub snapshot: RequestySnapshot,
    pub stale: bool,
    pub last_error: Option<(u16, String)>,
    pub cache_age: Option<Duration>,
}

struct CachedSnapshot {
    snapshot: RequestySnapshot,
    last_error: Option<(u16, String)>,
}

pub async fn fetch_snapshot(
    client: &reqwest::Client,
    api_key: &str,
    cache: &Cache,
    endpoints: &Endpoints,
    now: DateTime<Utc>,
    cache_ttl: Duration,
) -> Result<FetchOutcome> {
    cache.ensure_dir()?;
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    let target = target_key(endpoints, api_key);

    if let Some(bytes) = cache.maybe_payload()? {
        let partial = cache_is_partial(&bytes);
        let schema_drift =
            cache_last_error(&bytes).is_some_and(|(_, message)| message == SCHEMA_DRIFT_MESSAGE);
        let ttl = if partial { PARTIAL_TTL } else { cache_ttl };
        if !schema_drift
            && cache.payload_age().is_some_and(|age| age < ttl)
            && let Ok(cached) = parse_cache(&bytes, &target)
        {
            return Ok(FetchOutcome {
                snapshot: cached.snapshot,
                stale: false,
                last_error: cached.last_error,
                cache_age: cache.payload_age(),
            });
        }
    }

    let interval_end = now.with_nanosecond(0).expect("valid UTC timestamp");
    let interval_start = month_start(interval_end);
    let (organization, usage) = tokio::join!(
        fetch_organization(client, &endpoints.organization, api_key),
        fetch_usage(
            client,
            &endpoints.usage,
            api_key,
            interval_start,
            interval_end,
        ),
    );

    let (org_name, balance) = match organization {
        Ok(organization) => organization,
        Err(error @ AppError::Transport(_)) => return fallback_silent(cache, &target, error),
        Err(error) => {
            let pair = error_to_pair(&error);
            cache.mark_stale();
            cache.write_last_error(pair.0, &pair.1);
            return fallback_with_error(cache, &target, pair, error);
        }
    };

    let (usage, secondary_error, partial) = match usage {
        Ok(usage) => (Some(usage), None, false),
        Err(error) => (None, Some(error_to_pair(&error)), true),
    };
    let snapshot = RequestySnapshot {
        org_name,
        balance,
        usage,
        interval_start,
        interval_end,
        scope_fingerprint: target.clone(),
    };
    let cached_error = secondary_error
        .as_ref()
        .map(|(code, message)| serde_json::json!({"code": code, "message": message}));
    let body = serde_json::to_vec(&serde_json::json!({
        "target": target,
        "partial": partial,
        "last_error": cached_error,
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

fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("valid UTC month start")
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
    let mut outcome = reuse_cache(bytes, cache, target, true).or(Err(original))?;
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
    format!("{}|key:{fingerprint}", endpoints.organization)
}

fn cache_is_partial(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("partial").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn cache_last_error(bytes: &[u8]) -> Option<(u16, String)> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| cached_last_error(&value).ok().flatten())
}

fn snapshot_repr(snapshot: &RequestySnapshot) -> serde_json::Value {
    let usage = snapshot.usage.as_ref().map(|usage| {
        serde_json::json!({
            "mtd_spend": usage.mtd_spend,
            "requests": usage.requests,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        })
    });
    serde_json::json!({
        "org_name": snapshot.org_name,
        "balance": snapshot.balance,
        "usage": usage,
        "interval_start": snapshot.interval_start.to_rfc3339(),
        "interval_end": snapshot.interval_end.to_rfc3339(),
    })
}

fn parse_cache(bytes: &[u8], target: &str) -> Result<CachedSnapshot> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target) {
        return Err(AppError::Schema(
            "Requesty cache scope fingerprint mismatch".into(),
        ));
    }
    let response = value
        .get("response")
        .ok_or_else(|| AppError::Schema("Requesty cache missing response".into()))?;
    let interval_start = required_date(response, "interval_start")?;
    let interval_end = required_date(response, "interval_end")?;
    if interval_end < interval_start {
        return Err(AppError::Schema(
            "Requesty cache interval end precedes its start".into(),
        ));
    }
    Ok(CachedSnapshot {
        snapshot: RequestySnapshot {
            org_name: required_string(response, "org_name")?,
            balance: required_amount(response, "balance")?,
            usage: optional_usage(response)?,
            interval_start,
            interval_end,
            scope_fingerprint: target.to_string(),
        },
        last_error: cached_last_error(&value)?,
    })
}

fn cached_last_error(value: &serde_json::Value) -> Result<Option<(u16, String)>> {
    let Some(error) = value.get("last_error") else {
        return Ok(None);
    };
    if error.is_null() {
        return Ok(None);
    }
    let error = error
        .as_object()
        .ok_or_else(|| AppError::Schema("Requesty cache last_error is not an object".into()))?;
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::Schema("Requesty cache last_error missing code".into()))?
        .try_into()
        .map_err(|_| AppError::Schema("Requesty cache last_error code exceeds u16".into()))?;
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema("Requesty cache last_error missing message".into()))?;
    Ok(Some((code, message.to_string())))
}

fn required_string(value: &serde_json::Value, field: &str) -> Result<String> {
    let text = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema(format!("Requesty cache missing {field}")))?;
    if text.trim().is_empty() {
        return Err(AppError::Schema(format!(
            "Requesty cache has an empty {field}"
        )));
    }
    Ok(text.to_string())
}

fn required_amount(value: &serde_json::Value, field: &str) -> Result<f64> {
    let amount = value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| AppError::Schema(format!("Requesty cache missing {field}")))?;
    finite_amount("Requesty cache", field, amount)
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::Schema(format!("Requesty cache missing {field}")))
}

fn required_date(value: &serde_json::Value, field: &str) -> Result<DateTime<Utc>> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema(format!("Requesty cache missing {field}")))?
        .parse::<DateTime<Utc>>()
        .map_err(|error| AppError::Schema(format!("Requesty cache invalid {field}: {error}")))
}

fn optional_usage(response: &serde_json::Value) -> Result<Option<RequestyUsage>> {
    let Some(value) = response.get("usage") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if !value.is_object() {
        return Err(AppError::Schema(
            "Requesty cache usage is not an object".into(),
        ));
    }
    let mtd_spend = required_amount(value, "mtd_spend")?;
    if mtd_spend < 0.0 {
        return Err(AppError::Schema(
            "Requesty cache usage has negative mtd_spend".into(),
        ));
    }
    Ok(Some(RequestyUsage {
        mtd_spend,
        requests: required_u64(value, "requests")?,
        input_tokens: required_u64(value, "input_tokens")?,
        output_tokens: required_u64(value, "output_tokens")?,
        total_tokens: required_u64(value, "total_tokens")?,
    }))
}

async fn fetch_organization(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
) -> Result<(String, f64)> {
    fetch_json::<OrganizationResponse>(client, url, api_key, &[])
        .await?
        .into_organization()
}

async fn fetch_usage(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    interval_start: DateTime<Utc>,
    interval_end: DateTime<Utc>,
) -> Result<RequestyUsage> {
    let query = [
        (
            "start",
            interval_start.to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
        (
            "end",
            interval_end.to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
        ("resolution", "day".to_string()),
    ];
    fetch_json::<UsageResponse>(client, url, api_key, &query)
        .await?
        .into_usage()
}

async fn fetch_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    query: &[(&str, String)],
) -> Result<T> {
    let response = tokio::time::timeout(
        HTTP_TIMEOUT,
        client
            .get(url)
            .query(query)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| AppError::Transport(format!("Requesty request timed out: {url}")))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        let message = match status.as_u16() {
            401 => "Requesty authentication failed".to_string(),
            403 => "Requesty management permission required".to_string(),
            code => format!("Requesty API returned HTTP {code}"),
        };
        return Err(AppError::Http {
            status: status.as_u16(),
            body: message,
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("Requesty response schema: {error}")))
}

fn reuse_cache(bytes: Vec<u8>, cache: &Cache, target: &str, stale: bool) -> Result<FetchOutcome> {
    let cached = parse_cache(&bytes, target)?;
    Ok(FetchOutcome {
        snapshot: cached.snapshot,
        stale,
        last_error: cached.last_error,
        cache_age: cache.payload_age(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cache_fixture() -> (TempDir, Cache) {
        let dir = TempDir::new().unwrap();
        let cache = Cache::at(dir.path().join("requesty"));
        cache.ensure_dir().unwrap();
        (dir, cache)
    }

    fn endpoints(server: &mockito::ServerGuard) -> Endpoints {
        Endpoints {
            organization: format!("{}/v1/manage/org", server.url()),
            usage: format!("{}/v1/manage/org/usage", server.url()),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-08-24T12:34:56Z".parse().unwrap()
    }

    fn organization_body() -> &'static str {
        r#"{"name":"Acme Corp","balance":"42.5"}"#
    }

    fn usage_body() -> &'static str {
        r#"{"usage":{"2026-08-01":{"spend":1.25,"total_requests":10,"input_tokens":1200,"output_tokens":800,"total_tokens":2000},"2026-08-02":{"spend":2.5,"total_requests":7,"input_tokens":900,"output_tokens":600,"total_tokens":1500}}}"#
    }

    #[test]
    fn month_start_keeps_the_first_day_at_midnight_utc() {
        let start = month_start("2026-08-01T00:00:00.987Z".parse().unwrap());
        assert_eq!(start.to_rfc3339(), "2026-08-01T00:00:00+00:00");
    }

    #[tokio::test]
    async fn organization_and_usage_are_fetched_and_aggregated() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/manage/org")
            .match_header("authorization", "Bearer rqy-test")
            .with_status(200)
            .with_body(organization_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v1/manage/org/usage")
            .match_header("authorization", "Bearer rqy-test")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("start".into(), "2026-08-01T00:00:00Z".into()),
                mockito::Matcher::UrlEncoded("end".into(), "2026-08-24T12:34:56Z".into()),
                mockito::Matcher::UrlEncoded("resolution".into(), "day".into()),
            ]))
            .with_status(200)
            .with_body(usage_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "rqy-test",
            &cache,
            &endpoints(&server),
            now(),
            Duration::ZERO,
        )
        .await
        .unwrap();
        let usage = out.snapshot.usage.unwrap();
        assert_eq!(out.snapshot.org_name, "Acme Corp");
        assert_eq!(out.snapshot.balance, 42.5);
        assert_eq!(usage.mtd_spend, 3.75);
        assert_eq!(usage.requests, 17);
        assert_eq!(usage.total_tokens, 3500);
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn usage_forbidden_keeps_live_balance_as_partial() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/manage/org")
            .with_status(200)
            .with_body(organization_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v1/manage/org/usage")
            .match_query(mockito::Matcher::Any)
            .with_status(403)
            .with_body(r#"{"error":{"message":"scope missing"}}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "rqy-test",
            &cache,
            &endpoints(&server),
            now(),
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.balance, 42.5);
        assert!(out.snapshot.usage.is_none());
        assert_eq!(
            out.last_error,
            Some((403, "Requesty management permission required".into()))
        );
        assert!(!cache.is_stale());
    }

    #[tokio::test]
    async fn unauthorized_organization_body_is_redacted() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/manage/org")
            .with_status(401)
            .with_body(r#"{"error":{"message":"credential-bearing response"}}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let error = fetch_snapshot(
            &reqwest::Client::new(),
            "rqy-test",
            &cache,
            &endpoints(&server),
            now(),
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::Http { status: 401, body }
                if body == "Requesty authentication failed"
        ));
    }

    #[tokio::test]
    async fn fresh_partial_cache_uses_the_longer_retry_horizon() {
        let mut server = mockito::Server::new_async().await;
        let organization = server
            .mock("GET", "/v1/manage/org")
            .expect(1)
            .with_status(200)
            .with_body(organization_body())
            .create_async()
            .await;
        let usage = server
            .mock("GET", "/v1/manage/org/usage")
            .match_query(mockito::Matcher::Any)
            .expect(1)
            .with_status(503)
            .with_body("temporarily unavailable")
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "rqy-test",
            &cache,
            &endpoints,
            now(),
            Duration::ZERO,
        )
        .await
        .unwrap();
        let cached = fetch_snapshot(
            &reqwest::Client::new(),
            "rqy-test",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        organization.assert_async().await;
        usage.assert_async().await;
        assert!(cached.snapshot.usage.is_none());
        assert!(cached.last_error.is_some());
    }

    #[tokio::test]
    async fn changed_key_never_reuses_another_scope() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/manage/org")
            .expect(2)
            .with_status(200)
            .with_body(organization_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v1/manage/org/usage")
            .match_query(mockito::Matcher::Any)
            .expect(2)
            .with_status(200)
            .with_body(usage_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "old-key",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "new-key",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert!(out.snapshot.scope_fingerprint.contains("|key:"));
    }

    #[tokio::test]
    async fn changed_key_failure_keeps_the_original_error_and_cached_key_error_free() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/manage/org")
            .match_header("authorization", "Bearer key-a")
            .expect(1)
            .with_status(200)
            .with_body(organization_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v1/manage/org/usage")
            .match_header("authorization", "Bearer key-a")
            .match_query(mockito::Matcher::Any)
            .expect(1)
            .with_status(200)
            .with_body(usage_body())
            .create_async()
            .await;
        server
            .mock("GET", "/v1/manage/org")
            .match_header("authorization", "Bearer key-b")
            .expect(1)
            .with_status(403)
            .with_body(r#"{"error":{"message":"scope missing"}}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "key-a",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(60),
        )
        .await
        .unwrap();

        let error = fetch_snapshot(
            &reqwest::Client::new(),
            "key-b",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(60),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::Http { status: 403, body }
                if body == "Requesty management permission required"
        ));

        let cached = fetch_snapshot(
            &reqwest::Client::new(),
            "key-a",
            &cache,
            &endpoints,
            now(),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert!(cached.last_error.is_none());
    }
}
