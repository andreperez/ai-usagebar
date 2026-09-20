//! Fetch Together AI organization billing usage with Bearer authentication.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::{Cache, acquire_lock_async};
use crate::display::sanitize_untrusted_line;
use crate::error::{AppError, Result};
use crate::usage::TogetherSnapshot;

use super::types::{BillingUsageResponse, UsageAccumulator, validate_snapshot};

pub const USAGE_URL: &str = "https://api.together.ai/v1/billing/usage";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_LIMIT: &str = "1000";
const MAX_PAGES: usize = 12;

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub usage: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            usage: USAGE_URL.into(),
        }
    }
}

pub type FetchOutcome = crate::outcome::Outcome<TogetherSnapshot>;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEnvelope {
    target: String,
    snapshot: TogetherSnapshot,
}

pub async fn fetch_snapshot(
    client: &reqwest::Client,
    api_key: &str,
    cache: &Cache,
    endpoints: &Endpoints,
    cache_ttl: Duration,
) -> Result<FetchOutcome> {
    fetch_snapshot_at(client, api_key, cache, endpoints, cache_ttl, Utc::now()).await
}

async fn fetch_snapshot_at(
    client: &reqwest::Client,
    api_key: &str,
    cache: &Cache,
    endpoints: &Endpoints,
    cache_ttl: Duration,
    now: DateTime<Utc>,
) -> Result<FetchOutcome> {
    cache.ensure_dir()?;
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    let target = target_key(endpoints, api_key);
    let billing_period = now.format("%Y-%m").to_string();

    if let Some(bytes) = cache.fresh_payload(cache_ttl)?
        && let Ok(outcome) = reuse_cache(bytes, cache, false, &target, &billing_period)
    {
        return Ok(outcome);
    }

    match fetch_live(client, &endpoints.usage, api_key, &billing_period).await {
        Ok(snapshot) => {
            cache.write_payload(&serde_json::to_vec(&CacheEnvelope {
                target: target.clone(),
                snapshot: snapshot.clone(),
            })?)?;
            Ok(crate::outcome::Outcome::fresh(snapshot))
        }
        Err(error) if error.is_transient() => {
            fallback_silent(cache, error, &target, &billing_period)
        }
        Err(AppError::Http { status, body }) => {
            cache.mark_stale();
            let last_error = Some(cache.write_last_error(status, &body));
            fallback_with_error(
                cache,
                last_error,
                AppError::Http { status, body },
                &target,
                &billing_period,
            )
        }
        Err(error) => {
            cache.mark_stale();
            let last_error = Some(cache.write_last_error(0, &error.to_string()));
            fallback_with_error(cache, last_error, error, &target, &billing_period)
        }
    }
}

fn target_key(endpoints: &Endpoints, api_key: &str) -> String {
    let digest = Sha256::digest(api_key.as_bytes());
    let fingerprint: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}|{fingerprint}", endpoints.usage)
}

fn parse_cache(bytes: &[u8], target: &str, billing_period: &str) -> Result<TogetherSnapshot> {
    let envelope: CacheEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| AppError::Schema(format!("Together AI cache: {error}")))?;
    if envelope.target != target {
        return Err(AppError::Schema(
            "Together AI cache belongs to a different API key or endpoint".into(),
        ));
    }
    if envelope.snapshot.billing_period != billing_period {
        return Err(AppError::Schema(format!(
            "Together AI cache is for {}, expected {billing_period}",
            envelope.snapshot.billing_period
        )));
    }
    validate_snapshot(envelope.snapshot)
}

fn reuse_cache(
    bytes: Vec<u8>,
    cache: &Cache,
    stale: bool,
    target: &str,
    billing_period: &str,
) -> Result<FetchOutcome> {
    Ok(crate::outcome::Outcome::cached(
        parse_cache(&bytes, target, billing_period)?,
        cache,
        stale,
    ))
}

fn fallback_silent(
    cache: &Cache,
    original: AppError,
    target: &str,
    billing_period: &str,
) -> Result<FetchOutcome> {
    crate::outcome::fallback(cache, None, original, |bytes| {
        parse_cache(bytes, target, billing_period)
    })
}

fn fallback_with_error(
    cache: &Cache,
    last_error: Option<(u16, String)>,
    original: AppError,
    target: &str,
    billing_period: &str,
) -> Result<FetchOutcome> {
    crate::outcome::fallback(cache, last_error, original, |bytes| {
        parse_cache(bytes, target, billing_period)
    })
}

async fn fetch_live(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    billing_period: &str,
) -> Result<TogetherSnapshot> {
    let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| AppError::Other("Together AI API key is not a valid header value".into()))?;
    authorization.set_sensitive(true);

    let mut accumulator = UsageAccumulator::default();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = HashSet::new();

    for _ in 0..MAX_PAGES {
        let mut request = client
            .get(url)
            .header(AUTHORIZATION, authorization.clone())
            .header("Accept", "application/json")
            .query(&[("granularity", "day"), ("limit", PAGE_LIMIT)]);
        if let Some(after) = cursor.as_deref() {
            request = request.query(&[("after", after)]);
        }

        let response = tokio::time::timeout(HTTP_TIMEOUT, request.send())
            .await
            .map_err(|_| AppError::Transport("Together AI billing request timed out".into()))??;
        let status = response.status();
        let bytes =
            crate::vendor::read_body_capped(response, crate::vendor::MAX_BODY_BYTES).await?;
        if !status.is_success() {
            let body = sanitize_untrusted_line(&String::from_utf8_lossy(&bytes))
                .chars()
                .take(200)
                .collect();
            return Err(AppError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let page: BillingUsageResponse = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::Schema(format!("Together AI billing response: {error}")))?;
        cursor = accumulator.push(page)?;
        let Some(next) = cursor.as_ref() else {
            let snapshot = accumulator.finish()?;
            if snapshot.billing_period != billing_period {
                return Err(AppError::Schema(format!(
                    "Together AI billing returned period {}, expected {billing_period}",
                    snapshot.billing_period
                )));
            }
            return Ok(snapshot);
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(AppError::Schema(
                "Together AI billing returned a repeated pagination cursor".into(),
            ));
        }
    }

    Err(AppError::Schema(format!(
        "Together AI billing exceeded {MAX_PAGES} pages"
    )))
}

#[cfg(test)]
mod tests {
    use mockito::Matcher;
    use tempfile::TempDir;

    use super::*;

    fn cache_fixture() -> (TempDir, Cache) {
        let directory = TempDir::new().unwrap();
        let cache = Cache::at(directory.path().join("together"));
        cache.ensure_dir().unwrap();
        (directory, cache)
    }

    fn page(cost: &str, cursor: Option<&str>) -> String {
        format!(
            r#"{{"organization_id":"org-test","billing_period":"2026-09","currency":"USD","earliest_window_start":"2026-09-01T00:00:00Z","latest_window_end":"2026-09-02T00:00:00Z","data":[{{"line_items":[{{"product_name":"Serverless Inference","cost":"{cost}"}}]}}],"next_cursor":{}}}"#,
            cursor.map_or("null".into(), |value| format!("\"{value}\""))
        )
    }

    #[tokio::test]
    async fn paginates_and_caches_the_validated_snapshot() {
        let mut server = mockito::Server::new_async().await;
        let first = server
            .mock("GET", "/v1/billing/usage")
            .match_header("Authorization", "Bearer test-key")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("granularity".into(), "day".into()),
                Matcher::UrlEncoded("limit".into(), "1000".into()),
            ]))
            .with_status(200)
            .with_body(page("0.10", Some("page-2")))
            .create_async()
            .await;
        let second = server
            .mock("GET", "/v1/billing/usage")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("granularity".into(), "day".into()),
                Matcher::UrlEncoded("limit".into(), "1000".into()),
                Matcher::UrlEncoded("after".into(), "page-2".into()),
            ]))
            .with_status(200)
            .with_body(page("0.21", None))
            .create_async()
            .await;

        let (_directory, cache) = cache_fixture();
        let endpoints = Endpoints {
            usage: format!("{}/v1/billing/usage", server.url()),
        };
        let outcome = fetch_snapshot_at(
            &reqwest::Client::new(),
            "test-key",
            &cache,
            &endpoints,
            Duration::from_secs(60),
            "2026-09-15T12:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
        first.assert_async().await;
        second.assert_async().await;
        assert!((outcome.snapshot.monthly_spend - 0.31).abs() < 1e-9);
        assert!(!outcome.stale);
        let cached = cache
            .fresh_payload(Duration::from_secs(60))
            .unwrap()
            .unwrap();
        let target = target_key(&endpoints, "test-key");
        assert!(
            (parse_cache(&cached, &target, "2026-09")
                .unwrap()
                .monthly_spend
                - 0.31)
                .abs()
                < 1e-9
        );
    }

    #[tokio::test]
    async fn unauthorized_refresh_uses_stale_cache() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/billing/usage")
            .with_status(401)
            .with_body(r#"{"error":{"message":"invalid key"}}"#)
            .create_async()
            .await;

        let (_directory, cache) = cache_fixture();
        let mut accumulator = UsageAccumulator::default();
        accumulator
            .push(serde_json::from_str(&page("0.31", None)).unwrap())
            .unwrap();
        let endpoints = Endpoints {
            usage: format!("{}/v1/billing/usage", server.url()),
        };
        let snapshot = accumulator.finish().unwrap();
        cache
            .write_payload(
                &serde_json::to_vec(&CacheEnvelope {
                    target: target_key(&endpoints, "bad-key"),
                    snapshot,
                })
                .unwrap(),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let outcome = fetch_snapshot_at(
            &reqwest::Client::new(),
            "bad-key",
            &cache,
            &endpoints,
            Duration::ZERO,
            "2026-09-15T12:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
        assert!(outcome.stale);
        assert!((outcome.snapshot.monthly_spend - 0.31).abs() < 1e-9);
    }

    #[test]
    fn cache_is_bound_to_key_endpoint_and_current_month() {
        let snapshot = TogetherSnapshot {
            organization_id: "org-test".into(),
            billing_period: "2026-08".into(),
            currency: "USD".into(),
            monthly_spend: 1.0,
            products: Vec::new(),
            earliest_window_start: None,
            latest_window_end: None,
        };
        let endpoints = Endpoints::default();
        let bytes = serde_json::to_vec(&CacheEnvelope {
            target: target_key(&endpoints, "old-key"),
            snapshot,
        })
        .unwrap();
        assert!(parse_cache(&bytes, &target_key(&endpoints, "new-key"), "2026-08").is_err());
        assert!(parse_cache(&bytes, &target_key(&endpoints, "old-key"), "2026-09").is_err());
    }
}
