//! Cache-aware Vercel AI Gateway credits and optional custom-report fetch.

use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::cache::{Cache, DetailCache, MAX_STALE, acquire_lock_async};
use crate::error::{AppError, Result};
use crate::usage::{VercelGatewaySnapshot, VercelReport, finite_amount};
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

use super::types::{CreditsResponse, ReportResponse};

pub const BASE_URL: &str = "https://ai-gateway.vercel.sh/v1";
pub const SCHEMA_DRIFT_MESSAGE: &str = "Vercel AI Gateway API schema drift";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub credits: String,
    pub report: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            credits: format!("{BASE_URL}/credits"),
            report: format!("{BASE_URL}/report"),
        }
    }
}
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub snapshot: VercelGatewaySnapshot,
    pub stale: bool,
    pub last_error: Option<(u16, String)>,
    pub cache_age: Option<Duration>,
}

#[allow(clippy::too_many_arguments)]
pub async fn fetch_snapshot(
    client: &reqwest::Client,
    api_key: &str,
    api_key_env: &str,
    cache: &Cache,
    endpoints: &Endpoints,
    report_enabled: bool,
    report_ttl: Duration,
    cache_ttl: Duration,
    now: DateTime<Utc>,
) -> Result<FetchOutcome> {
    cache.ensure_dir()?;
    let target = target_key(endpoints, api_key, api_key_env);
    let credits = fetch_credits_cached(
        client,
        api_key,
        cache,
        &endpoints.credits,
        &target,
        cache_ttl,
    )
    .await?;
    let mut snapshot = credits.snapshot;
    let mut last_error = credits.last_error;
    if report_enabled {
        let detail = cache.detail("usage_report");
        match fetch_report_cached(
            client,
            api_key,
            &detail,
            &endpoints.report,
            &target,
            report_ttl,
            now,
        )
        .await
        {
            Ok(report) => snapshot.report = Some(report),
            Err(error) if last_error.is_none() => last_error = Some(error_to_pair(&error)),
            Err(_) => {}
        }
    }
    Ok(FetchOutcome {
        snapshot,
        stale: credits.stale,
        last_error,
        cache_age: credits.cache_age,
    })
}

struct CreditsOutcome {
    snapshot: VercelGatewaySnapshot,
    stale: bool,
    last_error: Option<(u16, String)>,
    cache_age: Option<Duration>,
}

async fn fetch_credits_cached(
    client: &reqwest::Client,
    api_key: &str,
    cache: &Cache,
    url: &str,
    target: &str,
    ttl: Duration,
) -> Result<CreditsOutcome> {
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    if let Some(bytes) = cache.maybe_payload()?
        && cache.payload_age().is_some_and(|age| age < ttl)
        && let Ok(snapshot) = parse_credits_cache(&bytes, target)
    {
        return Ok(CreditsOutcome {
            snapshot,
            stale: false,
            last_error: cache.read_last_error(),
            cache_age: cache.payload_age(),
        });
    }
    match fetch_json::<CreditsResponse>(client, url, api_key, &[], false).await {
        Ok(response) => {
            let snapshot = response.into_snapshot(target.to_string())?;
            let body = serde_json::to_vec(
                &serde_json::json!({"target":target,"balance":snapshot.balance,"total_used":snapshot.total_used}),
            )?;
            cache.write_payload(&body)?;
            Ok(CreditsOutcome {
                snapshot,
                stale: false,
                last_error: None,
                cache_age: Some(Duration::ZERO),
            })
        }
        Err(error) => fallback_credits(cache, target, error),
    }
}

async fn fetch_report_cached(
    client: &reqwest::Client,
    api_key: &str,
    detail: &DetailCache,
    url: &str,
    target: &str,
    ttl: Duration,
    now: DateTime<Utc>,
) -> Result<VercelReport> {
    let _lock = acquire_lock_async(&detail.lock_path(), LOCK_TIMEOUT).await?;
    let start = chrono::Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap();
    let end = now.date_naive();
    if let Some(bytes) = detail.maybe_payload()?
        && detail.payload_age().is_some_and(|age| age < ttl)
        && let Ok(report) = parse_report_cache(&bytes, target, start, end)
    {
        return Ok(report);
    }
    if detail.last_error_age().is_some_and(|age| age < ttl)
        && let Some((status, message)) = detail.read_scoped_last_error(target)
    {
        return Err(cached_report_error(status, message));
    }
    let query = [
        ("start_date", start.format("%Y-%m-%d").to_string()),
        ("end_date", end.format("%Y-%m-%d").to_string()),
        ("group_by", "day".into()),
    ];
    match fetch_json::<ReportResponse>(client, url, api_key, &query, true).await {
        Ok(response) => {
            let report = response.into_report(start, now)?;
            let body = serde_json::to_vec(
                &serde_json::json!({"target":target,"month":start.format("%Y-%m-%d").to_string(),"mtd_cost":report.mtd_cost,"input_tokens":report.input_tokens,"output_tokens":report.output_tokens,"requests":report.requests}),
            )?;
            detail.write_payload(&body)?;
            Ok(report)
        }
        Err(error) if matches!(error, AppError::Http { status: 403, .. }) => {
            let pair = error_to_pair(&error);
            detail.write_scoped_last_error(target, pair.0, &pair.1);
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn cached_report_error(status: u16, message: String) -> AppError {
    if status == 0 {
        AppError::Schema(message)
    } else {
        AppError::Http {
            status,
            body: message,
        }
    }
}

fn fallback_credits(cache: &Cache, target: &str, error: AppError) -> Result<CreditsOutcome> {
    let pair = error_to_pair(&error);
    cache.mark_stale();
    cache.write_last_error(pair.0, &pair.1);
    let Some(bytes) = cache.fallback_payload(MAX_STALE)? else {
        return Err(error);
    };
    let snapshot = parse_credits_cache(&bytes, target).map_err(|_| error)?;
    Ok(CreditsOutcome {
        snapshot,
        stale: true,
        last_error: Some(pair),
        cache_age: cache.payload_age(),
    })
}

fn parse_credits_cache(bytes: &[u8], target: &str) -> Result<VercelGatewaySnapshot> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target) {
        return Err(AppError::Schema(
            "Vercel credits cache scope mismatch".into(),
        ));
    }
    let amount = |field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AppError::Schema(format!("Vercel credits cache missing {field}")))
            .and_then(|v| finite_amount("Vercel credits cache", field, v))
    };
    let balance = amount("balance")?;
    let total_used = amount("total_used")?;
    if balance < 0.0 || total_used < 0.0 {
        return Err(AppError::Schema(
            "Vercel credits cache has negative amount".into(),
        ));
    }
    Ok(VercelGatewaySnapshot {
        balance,
        total_used,
        report: None,
        scope_fingerprint: target.into(),
    })
}

fn parse_report_cache(
    bytes: &[u8],
    target: &str,
    start: DateTime<Utc>,
    end: chrono::NaiveDate,
) -> Result<VercelReport> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target)
        || value.get("month").and_then(serde_json::Value::as_str)
            != Some(&start.format("%Y-%m-%d").to_string())
    {
        return Err(AppError::Schema(
            "Vercel report cache scope or month mismatch".into(),
        ));
    }
    let number = |field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AppError::Schema(format!("Vercel report cache missing {field}")))
            .and_then(|v| finite_amount("Vercel report cache", field, v))
    };
    let integer = |field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AppError::Schema(format!("Vercel report cache missing {field}")))
    };
    Ok(VercelReport {
        mtd_cost: number("mtd_cost")?,
        input_tokens: integer("input_tokens")?,
        output_tokens: integer("output_tokens")?,
        requests: integer("requests")?,
        interval_start: start,
        interval_end: chrono::Utc.from_utc_datetime(&end.and_hms_opt(23, 59, 59).unwrap()),
    })
}

async fn fetch_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    query: &[(&str, String)],
    is_report: bool,
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
    .map_err(|_| AppError::Transport(format!("Vercel request timed out: {url}")))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        return Err(AppError::Http {
            status: status.as_u16(),
            body: match status.as_u16() {
                401 => "Vercel AI Gateway authentication failed".into(),
                403 if is_report => {
                    "Vercel AI Gateway report requires a Pro or Enterprise plan".into()
                }
                403 => "Vercel AI Gateway authorization failed".into(),
                code => format!("Vercel AI Gateway API returned HTTP {code}"),
            },
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("Vercel response schema: {error}")))
}

fn error_to_pair(error: &AppError) -> (u16, String) {
    match error {
        AppError::Http { status, body } => (*status, body.clone()),
        AppError::Schema(_) => (0, SCHEMA_DRIFT_MESSAGE.into()),
        _ => (0, error.to_string()),
    }
}
fn target_key(endpoints: &Endpoints, api_key: &str, env: &str) -> String {
    let digest = Sha256::digest(format!("{}|{}|{}", api_key, env, endpoints.credits).as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    format!(
        "{}|{}|env:{env}|key:{hex}",
        endpoints.credits, endpoints.report
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn cache_fixture() -> (TempDir, Cache) {
        let dir = TempDir::new().unwrap();
        let cache = Cache::at(dir.path().join("vercel-ai-gateway"));
        cache.ensure_dir().unwrap();
        (dir, cache)
    }

    fn endpoints(server: &mockito::ServerGuard) -> Endpoints {
        Endpoints {
            credits: format!("{}/v1/credits", server.url()),
            report: format!("{}/v1/report", server.url()),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn report_disabled_never_calls_report_endpoint() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/credits")
            .match_header("authorization", "Bearer vag-test")
            .with_status(200)
            .with_body(r#"{"balance":"95.50","total_used":"4.50"}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "vag-test",
            "AI_GATEWAY_API_KEY",
            &cache,
            &endpoints(&server),
            false,
            Duration::from_secs(21_600),
            Duration::ZERO,
            now(),
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.balance, 95.5);
        assert!(out.snapshot.report.is_none());
    }

    #[tokio::test]
    async fn report_success_aggregates_results_with_calendar_bounds() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/credits")
            .with_status(200)
            .with_body(r#"{"balance":"95.50","total_used":"4.50"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/v1/report")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("start_date".into(), "2026-08-01".into()),
                mockito::Matcher::UrlEncoded("end_date".into(), "2026-08-25".into()),
                mockito::Matcher::UrlEncoded("group_by".into(), "day".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"results":[{"day":"2026-08-01","total_cost":1.25,"input_tokens":100,"output_tokens":20,"request_count":2}]}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "vag-test",
            "AI_GATEWAY_API_KEY",
            &cache,
            &endpoints(&server),
            true,
            Duration::from_secs(21_600),
            Duration::ZERO,
            now(),
        )
        .await
        .unwrap();
        let report = out.snapshot.report.unwrap();
        assert_eq!(report.mtd_cost, 1.25);
        assert_eq!(report.requests, 2);
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn report_forbidden_keeps_live_credits_and_caches_diagnostic() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/credits")
            .with_status(200)
            .with_body(r#"{"balance":"95.50","total_used":"4.50"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/v1/report")
            .match_query(mockito::Matcher::Any)
            .with_status(403)
            .with_body(r#"{"error":{"message":"account identifier"}}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "vag-test",
            "AI_GATEWAY_API_KEY",
            &cache,
            &endpoints(&server),
            true,
            Duration::from_secs(21_600),
            Duration::ZERO,
            now(),
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.balance, 95.5);
        assert!(out.snapshot.report.is_none());
        assert_eq!(
            out.last_error,
            Some((
                403,
                "Vercel AI Gateway report requires a Pro or Enterprise plan".into()
            ))
        );
        let target = target_key(&endpoints(&server), "vag-test", "AI_GATEWAY_API_KEY");
        assert_eq!(
            cache
                .detail("usage_report")
                .read_scoped_last_error(&target)
                .unwrap()
                .0,
            403
        );
    }

    #[test]
    fn env_name_and_key_rotation_change_scope_fingerprint() {
        let endpoints = Endpoints::default();
        assert_ne!(
            target_key(&endpoints, "key-a", "AI_GATEWAY_API_KEY"),
            target_key(&endpoints, "key-a", "VERCEL_OIDC_TOKEN")
        );
        assert_ne!(
            target_key(&endpoints, "key-a", "AI_GATEWAY_API_KEY"),
            target_key(&endpoints, "key-b", "AI_GATEWAY_API_KEY")
        );
    }
}
