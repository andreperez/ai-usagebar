//! Cache-aware Parallel Account API balance fetch.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::cache::{Cache, acquire_lock_async};
use crate::error::{AppError, Result};
use crate::usage::ParallelSnapshot;
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

use super::credentials::{ResolvedCredential, scope_fingerprint};
use super::types::BalanceResponse;

pub const BASE_URL: &str = "https://api.parallel.ai/account/service/v1";
pub const TOKEN_URL: &str = "https://platform.parallel.ai/getServiceKeys/token";
pub const SCHEMA_DRIFT_MESSAGE: &str = "Parallel Account API schema drift";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub balance: String,
    pub token: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            balance: format!("{BASE_URL}/balance"),
            token: TOKEN_URL.to_string(),
        }
    }
}

pub type FetchOutcome = crate::outcome::Outcome<ParallelSnapshot>;

pub async fn fetch_snapshot(
    client: &reqwest::Client,
    credential: &ResolvedCredential,
    cache: &Cache,
    endpoints: &Endpoints,
    cache_ttl: Duration,
) -> Result<FetchOutcome> {
    cache.ensure_dir()?;
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    let target = target_key(&endpoints.balance, &credential.scope_seed);
    if let Some(bytes) = cache.maybe_payload()?
        && cache.payload_age().is_some_and(|age| age < cache_ttl)
        && let Ok(snapshot) = parse_cache(&bytes, &target)
    {
        return Ok(crate::outcome::Outcome::cached(snapshot, cache, false));
    }
    match fetch_json::<BalanceResponse>(client, &endpoints.balance, &credential.access_token).await
    {
        Ok(response) => {
            let snapshot = response.into_snapshot(target.clone())?;
            let body = serde_json::to_vec(&serde_json::json!({
                "target": target,
                "credit_balance_cents": snapshot.credit_balance_cents,
                "pending_debit_balance_cents": snapshot.pending_debit_balance_cents,
                "will_invoice": snapshot.will_invoice,
            }))?;
            cache.write_payload(&body)?;
            Ok(crate::outcome::Outcome::fresh(snapshot))
        }
        Err(error) => fallback(cache, &target, error),
    }
}

async fn fetch_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<T> {
    let response = tokio::time::timeout(
        HTTP_TIMEOUT,
        client
            .get(url)
            .header("Accept", "application/json")
            .bearer_auth(access_token)
            .send(),
    )
    .await
    .map_err(|_| AppError::Transport("Parallel Account API request timed out".into()))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        let message = match status.as_u16() {
            401 => "Parallel OAuth token rejected — run `parallel-cli login`".to_string(),
            403 => "Parallel account permission required".to_string(),
            code => format!("Parallel Account API returned HTTP {code}"),
        };
        return Err(AppError::Http {
            status: status.as_u16(),
            body: message,
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("{SCHEMA_DRIFT_MESSAGE}: {error}")))
}

fn fallback(cache: &Cache, target: &str, error: AppError) -> Result<FetchOutcome> {
    let pair = error_to_pair(&error);
    cache.mark_stale();
    cache.write_last_error(pair.0, &pair.1);
    crate::outcome::fallback(cache, Some(pair), error, |bytes| parse_cache(bytes, target))
}

fn error_to_pair(error: &AppError) -> (u16, String) {
    match error {
        AppError::Http { status, body } => (*status, body.clone()),
        AppError::Schema(_) => (0, SCHEMA_DRIFT_MESSAGE.into()),
        _ => (0, error.to_string()),
    }
}

fn target_key(url: &str, scope_seed: &str) -> String {
    format!("{url}|token:{}", scope_fingerprint(scope_seed))
}

fn parse_cache(bytes: &[u8], target: &str) -> Result<ParallelSnapshot> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("target").and_then(serde_json::Value::as_str) != Some(target) {
        return Err(AppError::Schema(
            "Parallel cache scope fingerprint mismatch".into(),
        ));
    }
    let amount = |field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AppError::Schema(format!("Parallel cache missing {field}")))
    };
    let response = BalanceResponse {
        credit_balance_cents: amount("credit_balance_cents")?,
        pending_debit_balance_cents: amount("pending_debit_balance_cents")?,
        will_invoice: value
            .get("will_invoice")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| AppError::Schema("Parallel cache missing will_invoice".into()))?,
    };
    response.into_snapshot(target.into())
}
