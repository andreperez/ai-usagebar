//! Cache-aware ZenMux PAYG and subscription management fetch.

use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::cache::{Cache, MAX_STALE, acquire_lock_async};
use crate::error::{AppError, Result};
use crate::usage::{
    UsageWindow, ZenMuxPayg, ZenMuxQuota, ZenMuxSnapshot, ZenMuxStatus, ZenMuxSubscription,
    finite_amount,
};
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

use super::types::{PaygEnvelope, SubscriptionEnvelope};

pub const BASE_URL: &str = "https://zenmux.ai/api/v1/management";
pub const SCHEMA_DRIFT_MESSAGE: &str = "ZenMux API schema drift";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const PARTIAL_TTL: Duration = Duration::from_secs(300);
const MAX_TEXT_CHARS: usize = 128;

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub payg: String,
    pub subscription: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            payg: format!("{BASE_URL}/payg/balance"),
            subscription: format!("{BASE_URL}/subscription/detail"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub snapshot: ZenMuxSnapshot,
    pub stale: bool,
    pub last_error: Option<(u16, String)>,
    pub cache_age: Option<Duration>,
}

struct CachedSnapshot {
    snapshot: ZenMuxSnapshot,
    last_error: Option<(u16, String)>,
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

    let (payg, subscription) = tokio::join!(
        fetch_payg(client, &endpoints.payg, api_key),
        fetch_subscription(client, &endpoints.subscription, api_key),
    );
    let (payg, payg_error) = split_result(payg);
    let (subscription, subscription_error) = split_result(subscription);

    if payg.is_none() && subscription.is_none() {
        let error = select_error(
            payg_error.expect("failed PAYG fetch has an error"),
            subscription_error.expect("failed subscription fetch has an error"),
        );
        let pair = error_to_pair(&error);
        cache.mark_stale();
        cache.write_last_error(pair.0, &pair.1);
        return fallback_with_error(cache, &target, pair, error);
    }

    let secondary_error = payg_error
        .as_ref()
        .or(subscription_error.as_ref())
        .map(error_to_pair);
    let snapshot = ZenMuxSnapshot {
        payg,
        subscription,
        scope_fingerprint: target.clone(),
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "target": target,
        "partial": secondary_error.is_some(),
        "last_error": secondary_error.as_ref().map(|(code, message)| {
            serde_json::json!({"code": code, "message": message})
        }),
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

fn split_result<T>(result: Result<T>) -> (Option<T>, Option<AppError>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

fn select_error(first: AppError, second: AppError) -> AppError {
    if matches!(
        first,
        AppError::Http {
            status: 401 | 403,
            ..
        }
    ) {
        first
    } else if matches!(
        second,
        AppError::Http {
            status: 401 | 403,
            ..
        }
    ) || matches!(first, AppError::Transport(_))
    {
        second
    } else {
        first
    }
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
    format!(
        "{}|{}|key:{fingerprint}",
        endpoints.payg, endpoints.subscription
    )
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

fn snapshot_repr(snapshot: &ZenMuxSnapshot) -> serde_json::Value {
    serde_json::json!({
        "payg": snapshot.payg.as_ref().map(payg_repr),
        "subscription": snapshot.subscription.as_ref().map(subscription_repr),
    })
}

fn payg_repr(payg: &ZenMuxPayg) -> serde_json::Value {
    serde_json::json!({
        "total_credits": payg.total_credits,
        "top_up_credits": payg.top_up_credits,
        "bonus_credits": payg.bonus_credits,
    })
}

fn subscription_repr(subscription: &ZenMuxSubscription) -> serde_json::Value {
    serde_json::json!({
        "tier": subscription.tier,
        "plan_amount_usd": subscription.plan_amount_usd,
        "expires_at": subscription.expires_at.to_rfc3339(),
        "status": subscription.status.as_str(),
        "base_usd_per_flow": subscription.base_usd_per_flow,
        "effective_usd_per_flow": subscription.effective_usd_per_flow,
        "five_hour": quota_repr(&subscription.five_hour),
        "seven_day": quota_repr(&subscription.seven_day),
        "monthly_max_flows": subscription.monthly_max_flows,
        "monthly_max_value_usd": subscription.monthly_max_value_usd,
    })
}

fn quota_repr(quota: &ZenMuxQuota) -> serde_json::Value {
    serde_json::json!({
        "utilization_pct": quota.window.utilization_pct,
        "resets_at": quota.window.resets_at.map(|at| at.to_rfc3339()),
        "max_flows": quota.max_flows,
        "used_flows": quota.used_flows,
        "remaining_flows": quota.remaining_flows,
        "used_value_usd": quota.used_value_usd,
        "max_value_usd": quota.max_value_usd,
    })
}

fn parse_cache(bytes: &[u8], target: &str) -> Result<CachedSnapshot> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target) {
        return Err(AppError::Schema(
            "ZenMux cache scope fingerprint mismatch".into(),
        ));
    }
    let response = value
        .get("response")
        .ok_or_else(|| AppError::Schema("ZenMux cache missing response".into()))?;
    let payg = optional_payg(response)?;
    let subscription = optional_subscription(response)?;
    if payg.is_none() && subscription.is_none() {
        return Err(AppError::Schema(
            "ZenMux cache has no usable response block".into(),
        ));
    }
    Ok(CachedSnapshot {
        snapshot: ZenMuxSnapshot {
            payg,
            subscription,
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
        .ok_or_else(|| AppError::Schema("ZenMux cache last_error is not an object".into()))?;
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::Schema("ZenMux cache last_error missing code".into()))?
        .try_into()
        .map_err(|_| AppError::Schema("ZenMux cache last_error code exceeds u16".into()))?;
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema("ZenMux cache last_error missing message".into()))?
        .trim();
    if message.is_empty() || message.chars().count() > MAX_TEXT_CHARS {
        return Err(AppError::Schema(
            "ZenMux cache last_error has an invalid message".into(),
        ));
    }
    Ok(Some((code, message.to_string())))
}

fn optional_payg(response: &serde_json::Value) -> Result<Option<ZenMuxPayg>> {
    let Some(value) = response.get("payg") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(ZenMuxPayg {
        total_credits: required_amount(value, "total_credits")?,
        top_up_credits: required_amount(value, "top_up_credits")?,
        bonus_credits: required_amount(value, "bonus_credits")?,
    }))
}

fn optional_subscription(response: &serde_json::Value) -> Result<Option<ZenMuxSubscription>> {
    let Some(value) = response.get("subscription") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(ZenMuxSubscription {
        tier: required_string(value, "tier")?,
        plan_amount_usd: required_amount(value, "plan_amount_usd")?,
        expires_at: required_date(value, "expires_at")?,
        status: required_status(value, "status")?,
        base_usd_per_flow: required_amount(value, "base_usd_per_flow")?,
        effective_usd_per_flow: required_amount(value, "effective_usd_per_flow")?,
        five_hour: required_quota(value, "five_hour", chrono::Duration::hours(5))?,
        seven_day: required_quota(value, "seven_day", chrono::Duration::days(7))?,
        monthly_max_flows: required_amount(value, "monthly_max_flows")?,
        monthly_max_value_usd: required_amount(value, "monthly_max_value_usd")?,
    }))
}

fn required_quota(
    value: &serde_json::Value,
    field: &str,
    window_duration: chrono::Duration,
) -> Result<ZenMuxQuota> {
    let value = value
        .get(field)
        .ok_or_else(|| AppError::Schema(format!("ZenMux cache missing {field}")))?;
    let utilization_pct = value
        .get("utilization_pct")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| AppError::Schema(format!("ZenMux cache missing {field}.utilization_pct")))?;
    if !(0..=100).contains(&utilization_pct) {
        return Err(AppError::Schema(format!(
            "ZenMux cache invalid {field}.utilization_pct"
        )));
    }
    Ok(ZenMuxQuota {
        window: UsageWindow {
            utilization_pct: utilization_pct as i32,
            resets_at: optional_date(value, "resets_at")?,
            window_duration,
        },
        max_flows: required_amount(value, "max_flows")?,
        used_flows: required_amount(value, "used_flows")?,
        remaining_flows: required_amount(value, "remaining_flows")?,
        used_value_usd: required_amount(value, "used_value_usd")?,
        max_value_usd: required_amount(value, "max_value_usd")?,
    })
}

fn required_status(value: &serde_json::Value, field: &str) -> Result<ZenMuxStatus> {
    let status = required_string(value, field)?;
    Ok(match status.as_str() {
        "healthy" => ZenMuxStatus::Healthy,
        "monitored" => ZenMuxStatus::Monitored,
        "abusive" => ZenMuxStatus::Abusive,
        "suspended" => ZenMuxStatus::Suspended,
        "banned" => ZenMuxStatus::Banned,
        _ => ZenMuxStatus::Other(status),
    })
}

fn required_string(value: &serde_json::Value, field: &str) -> Result<String> {
    let text = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema(format!("ZenMux cache missing {field}")))?
        .trim();
    if text.is_empty()
        || text.chars().count() > MAX_TEXT_CHARS
        || text.chars().any(char::is_control)
    {
        return Err(AppError::Schema(format!("ZenMux cache invalid {field}")));
    }
    Ok(text.to_string())
}

fn required_amount(value: &serde_json::Value, field: &str) -> Result<f64> {
    let amount = value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| AppError::Schema(format!("ZenMux cache missing {field}")))?;
    let amount = finite_amount("ZenMux cache", field, amount)?;
    if amount < 0.0 {
        return Err(AppError::Schema(format!(
            "ZenMux cache has negative {field}"
        )));
    }
    Ok(amount)
}

fn required_date(value: &serde_json::Value, field: &str) -> Result<DateTime<Utc>> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::Schema(format!("ZenMux cache missing {field}")))?
        .parse::<DateTime<Utc>>()
        .map_err(|error| AppError::Schema(format!("ZenMux cache invalid {field}: {error}")))
}

fn optional_date(value: &serde_json::Value, field: &str) -> Result<Option<DateTime<Utc>>> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .ok_or_else(|| AppError::Schema(format!("ZenMux cache invalid {field}")))?
            .parse::<DateTime<Utc>>()
            .map(Some)
            .map_err(|error| AppError::Schema(format!("ZenMux cache invalid {field}: {error}"))),
    }
}

async fn fetch_payg(client: &reqwest::Client, url: &str, api_key: &str) -> Result<ZenMuxPayg> {
    fetch_json::<PaygEnvelope>(client, url, api_key)
        .await?
        .into_payg()
}

async fn fetch_subscription(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
) -> Result<ZenMuxSubscription> {
    fetch_json::<SubscriptionEnvelope>(client, url, api_key)
        .await?
        .into_subscription()
}

async fn fetch_json<T: DeserializeOwned>(
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
    .map_err(|_| AppError::Transport(format!("ZenMux request timed out: {url}")))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        let message = match status.as_u16() {
            401 | 403 => "ZenMux management key required".to_string(),
            422 => "ZenMux API rate limit exceeded".to_string(),
            code => format!("ZenMux API returned HTTP {code}"),
        };
        return Err(AppError::Http {
            status: status.as_u16(),
            body: message,
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("ZenMux response schema: {error}")))
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
        let cache = Cache::at(dir.path().join("zenmux"));
        cache.ensure_dir().unwrap();
        (dir, cache)
    }

    fn endpoints(server: &mockito::ServerGuard) -> Endpoints {
        Endpoints {
            payg: format!("{}/api/v1/management/payg/balance", server.url()),
            subscription: format!("{}/api/v1/management/subscription/detail", server.url()),
        }
    }

    fn payg_body() -> &'static str {
        r#"{"success":true,"data":{"currency":"usd","total_credits":42.5,"top_up_credits":30.0,"bonus_credits":12.5}}"#
    }

    fn subscription_body() -> &'static str {
        r#"{"success":true,"data":{"plan":{"tier":"Pro","amount_usd":20,"interval":"month","expires_at":"2026-09-15T00:00:00Z"},"currency":"usd","base_usd_per_flow":0.01,"effective_usd_per_flow":0.01,"account_status":"healthy","quota_5_hour":{"usage_percentage":0.84,"resets_at":"2026-08-24T15:00:00Z","max_flows":1000,"used_flows":840,"remaining_flows":160,"used_value_usd":8.4,"max_value_usd":10},"quota_7_day":{"usage_percentage":0.42,"resets_at":"2026-08-29T12:00:00Z","max_flows":7000,"used_flows":2940,"remaining_flows":4060,"used_value_usd":29.4,"max_value_usd":70},"quota_monthly":{"max_flows":30000,"max_value_usd":300}}}"#
    }

    #[tokio::test]
    async fn payg_and_subscription_combine_concurrently() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .match_header("authorization", "Bearer zmx-test")
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .match_header("authorization", "Bearer zmx-test")
            .with_status(200)
            .with_body(subscription_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints(&server),
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(out.snapshot.payg.unwrap().total_credits, 42.5);
        assert_eq!(
            out.snapshot
                .subscription
                .unwrap()
                .five_hour
                .window
                .utilization_pct,
            84
        );
        assert!(out.last_error.is_none());
    }

    #[tokio::test]
    async fn payg_survives_subscription_management_key_rejection() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .with_status(401)
            .with_body(r#"{"error":"credential material"}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints(&server),
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(out.snapshot.payg.is_some());
        assert!(out.snapshot.subscription.is_none());
        assert_eq!(
            out.last_error,
            Some((401, "ZenMux management key required".into()))
        );
        assert!(cache_is_partial(&cache.maybe_payload().unwrap().unwrap()));
    }

    #[tokio::test]
    async fn subscription_survives_payg_rate_limit() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .with_status(422)
            .with_body(r#"{"error":"rate limit"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .with_status(200)
            .with_body(subscription_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints(&server),
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(out.snapshot.payg.is_none());
        assert!(out.snapshot.subscription.is_some());
        assert_eq!(
            out.last_error,
            Some((422, "ZenMux API rate limit exceeded".into()))
        );
    }

    #[tokio::test]
    async fn both_failed_blocks_do_not_create_a_snapshot() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .with_status(200)
            .with_body(r#"{"success":false}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .with_status(200)
            .with_body(r#"{"success":false}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let error = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints(&server),
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Schema(_)));
    }

    #[tokio::test]
    async fn standard_inference_key_is_reported_as_management_key_required() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .with_status(401)
            .with_body(r#"{"error":"standard key"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .with_status(401)
            .with_body(r#"{"error":"standard key"}"#)
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let error = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints(&server),
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::Http { status: 401, body }
                if body == "ZenMux management key required"
        ));
    }

    #[tokio::test]
    async fn changed_key_never_reuses_another_scope() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/v1/management/payg/balance")
            .expect(2)
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        server
            .mock("GET", "/api/v1/management/subscription/detail")
            .expect(2)
            .with_status(200)
            .with_body(subscription_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "key-a",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "key-b",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert!(out.snapshot.scope_fingerprint.contains("|key:"));
    }

    #[tokio::test]
    async fn fresh_full_cache_skips_both_network_requests() {
        let mut server = mockito::Server::new_async().await;
        let payg = server
            .mock("GET", "/api/v1/management/payg/balance")
            .expect(1)
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        let subscription = server
            .mock("GET", "/api/v1/management/subscription/detail")
            .expect(1)
            .with_status(200)
            .with_body(subscription_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        let cached = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        payg.assert_async().await;
        subscription.assert_async().await;
        assert!(cached.snapshot.payg.is_some());
        assert!(cached.snapshot.subscription.is_some());
    }

    #[tokio::test]
    async fn stale_cache_keeps_transport_diagnostic_when_both_endpoints_fail() {
        let (_dir, cache) = cache_fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let test_endpoints = Endpoints {
            payg: format!("http://127.0.0.1:{port}/payg"),
            subscription: format!("http://127.0.0.1:{port}/subscription"),
        };
        let target = target_key(&test_endpoints, "zmx-test");
        let snapshot = ZenMuxSnapshot {
            payg: Some(ZenMuxPayg {
                total_credits: 42.5,
                top_up_credits: 30.0,
                bonus_credits: 12.5,
            }),
            subscription: None,
            scope_fingerprint: target.clone(),
        };
        let body = serde_json::to_vec(&serde_json::json!({
            "target": target,
            "partial": false,
            "last_error": null,
            "response": snapshot_repr(&snapshot),
        }))
        .unwrap();
        cache.write_payload(&body).unwrap();
        let out = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &test_endpoints,
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(out.stale);
        assert!(
            out.last_error
                .as_ref()
                .is_some_and(|(code, message)| *code == 0 && message.contains("transport"))
        );
        assert!(cache.is_stale());
    }

    #[tokio::test]
    async fn partial_cache_uses_the_longer_retry_horizon() {
        let mut server = mockito::Server::new_async().await;
        let payg = server
            .mock("GET", "/api/v1/management/payg/balance")
            .expect(1)
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        let subscription = server
            .mock("GET", "/api/v1/management/subscription/detail")
            .expect(1)
            .with_status(503)
            .with_body("unavailable")
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::ZERO,
        )
        .await
        .unwrap();
        let cached = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        payg.assert_async().await;
        subscription.assert_async().await;
        assert!(cached.snapshot.payg.is_some());
        assert!(cached.snapshot.subscription.is_none());
    }

    #[tokio::test]
    async fn schema_drift_partial_cache_refetches_immediately() {
        let mut server = mockito::Server::new_async().await;
        let payg = server
            .mock("GET", "/api/v1/management/payg/balance")
            .expect(2)
            .with_status(200)
            .with_body(payg_body())
            .create_async()
            .await;
        let invalid_subscription = server
            .mock("GET", "/api/v1/management/subscription/detail")
            .expect(1)
            .with_status(200)
            .with_body(r#"{"success":false}"#)
            .create_async()
            .await;
        let valid_subscription = server
            .mock("GET", "/api/v1/management/subscription/detail")
            .expect(1)
            .with_status(200)
            .with_body(subscription_body())
            .create_async()
            .await;
        let (_dir, cache) = cache_fixture();
        let endpoints = endpoints(&server);
        let first = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(first.snapshot.subscription.is_none());
        assert_eq!(first.last_error, Some((0, SCHEMA_DRIFT_MESSAGE.into())));
        let second = fetch_snapshot(
            &reqwest::Client::new(),
            "zmx-test",
            &cache,
            &endpoints,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        payg.assert_async().await;
        invalid_subscription.assert_async().await;
        valid_subscription.assert_async().await;
        assert!(second.snapshot.subscription.is_some());
        assert!(second.last_error.is_none());
        assert!(cache.read_last_error().is_none());
    }
}
