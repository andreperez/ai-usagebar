//! Parallel OAuth credentials from the parallel-cli credential store.
//!
//! Parallel's Account API (balance) only accepts short-lived OAuth access
//! tokens minted by the device flow that `parallel-cli login` performs;
//! standard data API keys are rejected with 401. Instead of implementing the
//! device flow, this module reuses the CLI's own session from
//! `~/.config/parallel-web-tools/auth.json`, refreshing access tokens that
//! expire within the skew window through `platform.parallel.ai` and writing
//! the rotated pair back so the CLI's login stays valid.
//!
//! The refresh protocol mirrors `parallel_web_tools.core.auth`:
//! - `authorization_expires_at` in the past → re-login required.
//! - Access token fresh beyond a 30s skew → use as-is.
//! - Otherwise exchange the refresh token (form-encoded POST with
//!   `grant_type=refresh_token` plus the file's `client_id`) and persist the
//!   new pair under the response's `org_id`, selecting it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::cache::{acquire_lock_async, atomic_write, home_dir};
use crate::error::{AppError, Result};
use crate::vendor::{MAX_BODY_BYTES, read_body_capped};

/// Refresh proactively when the access token is this close to expiring.
pub const ACCESS_TOKEN_SKEW_SECONDS: i64 = 30;
/// Fallback OAuth client id when the store has none (mirrors the CLI).
pub const DEFAULT_CLIENT_ID: &str = "parallel-cli";
pub const RELOGIN_MESSAGE: &str = "Parallel login required — run `parallel-cli login`";

const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);

/// A ready-to-use Account API credential plus the seed for its cache scope.
/// Store-backed credentials keep one stable seed across token rotations so a
/// refresh never invalidates the balance cache.
#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    pub access_token: String,
    pub scope_seed: String,
}

/// parallel-cli (Python `Path.home()`) stores credentials at this relative
/// path on every platform, including Windows.
pub fn default_credentials_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("parallel-web-tools")
        .join("auth.json"))
}

/// Account API access tokens are JWTs; data API keys are opaque. The env var
/// carries a data key in normal setups, so only JWT-shaped values count as an
/// intentional token override.
pub fn looks_like_account_token(token: &str) -> bool {
    let mut segments = token.split('.');
    (0..3).all(|_| segments.next().is_some_and(|s| !s.is_empty())) && segments.next().is_none()
}

/// Explicit override: inline config wins unconditionally (deliberate paste);
/// the env var only when it holds an Account API token rather than a data key.
pub fn explicit_override(inline: Option<&str>, env_name: &str) -> Option<String> {
    if let Some(token) = inline.map(str::trim).filter(|t| !t.is_empty()) {
        return Some(token.to_string());
    }
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && looks_like_account_token(value))
}

/// Resolve the balance credential: explicit override first, then the CLI
/// store with transparent refresh.
pub async fn resolve_token(
    client: &reqwest::Client,
    explicit: Option<&str>,
    store_path: Option<&Path>,
    token_url: &str,
    now: DateTime<Utc>,
) -> Result<ResolvedCredential> {
    if let Some(token) = explicit.map(str::trim).filter(|t| !t.is_empty()) {
        return Ok(ResolvedCredential {
            access_token: token.to_string(),
            scope_seed: token.to_string(),
        });
    }
    let path = match store_path {
        Some(path) => path.to_path_buf(),
        None => default_credentials_path()?,
    };
    if !path.exists() {
        return Err(AppError::Credentials(RELOGIN_MESSAGE.into()));
    }
    let document = read_store(&path)?;
    let now_ts = now.timestamp();
    match classify(&document, now_ts) {
        Classification::Fresh {
            access_token,
            org_id,
        } => Ok(ResolvedCredential {
            access_token,
            scope_seed: format!("oauth:{org_id}"),
        }),
        Classification::Reauthenticate => Err(AppError::Credentials(RELOGIN_MESSAGE.into())),
        Classification::Refresh { .. } => {
            // Serialize refreshes (widget re-runs + the CLI itself) so a rotated
            // single-use refresh token is never spent twice.
            let lock_path = lock_path_for(&path);
            let _lock = acquire_lock_async(&lock_path, LOCK_TIMEOUT).await?;
            // Re-read under the lock: another process may have refreshed
            // between the first read and the lock acquisition.
            let document = read_store(&path)?;
            match classify(&document, now_ts) {
                Classification::Fresh {
                    access_token,
                    org_id,
                } => Ok(ResolvedCredential {
                    access_token,
                    scope_seed: format!("oauth:{org_id}"),
                }),
                Classification::Reauthenticate => {
                    Err(AppError::Credentials(RELOGIN_MESSAGE.into()))
                }
                Classification::Refresh { refresh_token, .. } => {
                    let client_id = document
                        .get("client_id")
                        .and_then(Value::as_str)
                        .unwrap_or(DEFAULT_CLIENT_ID);
                    let response = refresh(client, token_url, &refresh_token, client_id).await?;
                    let org_id = response.org_id.clone();
                    let access_token = response.access_token.clone();
                    persist_refresh(&path, &document, response, now_ts)?;
                    Ok(ResolvedCredential {
                        access_token,
                        scope_seed: format!("oauth:{org_id}"),
                    })
                }
            }
        }
    }
}

#[derive(Debug)]
enum Classification {
    Fresh {
        access_token: String,
        org_id: String,
    },
    Refresh {
        refresh_token: String,
    },
    Reauthenticate,
}

fn classify(document: &Value, now_ts: i64) -> Classification {
    let Some(org_id) = document
        .get("selected_org_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Classification::Reauthenticate;
    };
    let Some(control) = document
        .get("orgs")
        .and_then(Value::as_object)
        .and_then(|orgs| orgs.get(&org_id))
        .and_then(|org| org.get("control_api"))
        .filter(|control| control.is_object())
    else {
        return Classification::Reauthenticate;
    };
    let field = |name: &str| control.get(name);
    let integer = |name: &str| field(name).and_then(Value::as_i64);
    if integer("authorization_expires_at").is_some_and(|expires| now_ts >= expires) {
        return Classification::Reauthenticate;
    }
    let Some(access_token) = field("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
    else {
        return Classification::Reauthenticate;
    };
    let fresh = integer("access_token_expires_at")
        .is_none_or(|expires| now_ts < expires - ACCESS_TOKEN_SKEW_SECONDS);
    if fresh {
        return Classification::Fresh {
            access_token,
            org_id,
        };
    }
    let Some(refresh_token) = field("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
    else {
        return Classification::Reauthenticate;
    };
    if integer("refresh_token_expires_at").is_some_and(|expires| now_ts >= expires) {
        return Classification::Reauthenticate;
    }
    Classification::Refresh { refresh_token }
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    refresh_token_expires_in: i64,
    authorization_expires_in: i64,
    org_id: String,
    org_name: Option<String>,
    scope: Option<String>,
}

async fn refresh(
    client: &reqwest::Client,
    token_url: &str,
    refresh_token: &str,
    client_id: &str,
) -> Result<TokenResponse> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    let response = tokio::time::timeout(
        REFRESH_TIMEOUT,
        client
            .post(token_url)
            .header("Accept", "application/json")
            .form(&form)
            .send(),
    )
    .await
    .map_err(|_| AppError::Transport("Parallel token refresh timed out".into()))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        // A rejected refresh grant means the session itself is gone; any other
        // status is ordinary HTTP failure worth surfacing verbatim.
        if status.as_u16() == 400 || status.as_u16() == 401 {
            return Err(AppError::Credentials(RELOGIN_MESSAGE.into()));
        }
        return Err(AppError::Http {
            status: status.as_u16(),
            body: format!("Parallel token refresh returned HTTP {}", status.as_u16()),
        });
    }
    serde_json::from_slice(&body)
        .map_err(|error| AppError::Schema(format!("Parallel token refresh schema drift: {error}")))
}

/// Write the rotated pair back, preserving every field this program does not
/// own (the document is round-tripped as raw JSON, never rebuilt).
fn persist_refresh(
    path: &Path,
    document: &Value,
    response: TokenResponse,
    now_ts: i64,
) -> Result<()> {
    let mut document = document.clone();
    let orgs = document
        .as_object_mut()
        .ok_or_else(|| AppError::Other("Parallel credentials store is not an object".into()))?
        .entry("orgs")
        .or_insert_with(|| Value::Object(Default::default()));
    let org = orgs
        .as_object_mut()
        .ok_or_else(|| AppError::Other("Parallel credentials orgs is not an object".into()))?
        .entry(response.org_id.clone())
        .or_insert_with(|| serde_json::json!({}));
    let org = org
        .as_object_mut()
        .ok_or_else(|| AppError::Other("Parallel credentials org is not an object".into()))?;
    if let Some(name) = &response.org_name {
        org.insert("org_name".into(), Value::String(name.clone()));
    }
    let control = org
        .entry("control_api")
        .or_insert_with(|| Value::Object(Default::default()));
    let control = control
        .as_object_mut()
        .ok_or_else(|| AppError::Other("Parallel control_api is not an object".into()))?;
    control.insert(
        "access_token".into(),
        Value::String(response.access_token.clone()),
    );
    control.insert(
        "access_token_expires_at".into(),
        Value::from(now_ts + response.expires_in),
    );
    if let Some(scope) = &response.scope {
        let scopes: Vec<Value> = scope
            .split_whitespace()
            .map(|s| Value::String(s.to_string()))
            .collect();
        control.insert("access_token_scopes".into(), Value::Array(scopes));
    }
    control.insert(
        "refresh_token".into(),
        Value::String(response.refresh_token.clone()),
    );
    control.insert(
        "refresh_token_expires_at".into(),
        Value::from(now_ts + response.refresh_token_expires_in),
    );
    control.insert(
        "authorization_expires_at".into(),
        Value::from(now_ts + response.authorization_expires_in),
    );
    let document = document
        .as_object_mut()
        .ok_or_else(|| AppError::Other("Parallel credentials store is not an object".into()))?;
    document.insert(
        "selected_org_id".into(),
        Value::String(response.org_id.clone()),
    );
    let bytes = serde_json::to_vec_pretty(&document)?;
    atomic_write(path, &bytes)
}

fn read_store(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).map_err(|error| AppError::Io {
        path: path.to_path_buf(),
        source: error,
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Credentials(format!("Parallel credentials store is unreadable: {error}"))
    })
}

fn lock_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

/// Scope seed for cache keys; hashed by the fetch layer like any credential.
pub fn scope_fingerprint(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(
        access_expires: Option<i64>,
        refresh_expires: Option<i64>,
        authorization_expires: Option<i64>,
    ) -> Value {
        serde_json::json!({
            "access_token": "header.payload.signature",
            "access_token_expires_at": access_expires,
            "access_token_scopes": ["balance:read"],
            "refresh_token": "refresh-secret",
            "refresh_token_expires_at": refresh_expires,
            "authorization_expires_at": authorization_expires,
        })
    }

    fn store(control: Value) -> Value {
        serde_json::json!({
            "version": 1,
            "selected_org_id": "org-1",
            "client_id": "client-abc",
            "unknown_future_field": "keep-me",
            "orgs": { "org-1": { "org_name": "Acme", "control_api": control } },
        })
    }

    #[test]
    fn account_tokens_are_three_non_empty_jwt_segments() {
        assert!(looks_like_account_token("aaa.bbb.ccc"));
        assert!(!looks_like_account_token(
            "data-key-without-dots-0123456789"
        ));
        assert!(!looks_like_account_token("a.b"));
        assert!(!looks_like_account_token("a..c"));
        assert!(!looks_like_account_token(""));
    }

    #[test]
    fn fresh_access_token_is_used_without_refresh() {
        let document = store(control(Some(10_000), Some(20_000), Some(30_000)));
        match classify(&document, 5_000) {
            Classification::Fresh {
                access_token,
                org_id,
            } => {
                assert_eq!(access_token, "header.payload.signature");
                assert_eq!(org_id, "org-1");
            }
            other => panic!("expected fresh, got {other:?}"),
        }
    }

    #[test]
    fn token_inside_skew_window_refreshes() {
        let document = store(control(Some(5_029), Some(90_000), Some(90_000)));
        match classify(&document, 5_000) {
            Classification::Refresh { refresh_token } => {
                assert_eq!(refresh_token, "refresh-secret");
            }
            other => panic!("expected refresh, got {other:?}"),
        }
    }

    #[test]
    fn expired_refresh_or_authorization_requires_relogin() {
        let expired_refresh = store(control(Some(1_000), Some(2_000), Some(90_000)));
        assert!(matches!(
            classify(&expired_refresh, 5_000),
            Classification::Reauthenticate
        ));
        let expired_authorization = store(control(Some(90_000), Some(90_000), Some(4_999)));
        assert!(matches!(
            classify(&expired_authorization, 5_000),
            Classification::Reauthenticate
        ));
        let no_tokens = serde_json::json!({"version": 1, "orgs": {}});
        assert!(matches!(
            classify(&no_tokens, 5_000),
            Classification::Reauthenticate
        ));
    }

    #[test]
    fn persist_preserves_unknown_fields_and_rotates_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = store(control(Some(1_000), Some(90_000), Some(90_000)));
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let response = TokenResponse {
            access_token: "new.access.token".into(),
            refresh_token: "rotated-refresh".into(),
            expires_in: 420,
            refresh_token_expires_in: 2_592_000,
            authorization_expires_in: 2_592_000,
            org_id: "org-1".into(),
            org_name: Some("Acme".into()),
            scope: Some("balance:read balance:add".into()),
        };
        persist_refresh(&path, &original, response, 5_000).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/access_token_expires_at")
                .unwrap(),
            &Value::from(5_420)
        );
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/refresh_token")
                .unwrap(),
            "rotated-refresh"
        );
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/access_token_scopes")
                .unwrap(),
            &serde_json::json!(["balance:read", "balance:add"])
        );
        assert_eq!(saved.get("unknown_future_field").unwrap(), "keep-me");
        assert_eq!(saved.get("client_id").unwrap(), "client-abc");
    }

    fn write_store(dir: &tempfile::TempDir, control: Value) -> std::path::PathBuf {
        let path = dir.path().join("auth.json");
        std::fs::write(&path, serde_json::to_vec(&store(control)).unwrap()).unwrap();
        path
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(5_000, 0).unwrap()
    }

    #[tokio::test]
    async fn explicit_override_skips_the_store() {
        let credential = resolve_token(
            &reqwest::Client::new(),
            Some("header.payload.signature"),
            None,
            "http://127.0.0.1:9/token",
            now(),
        )
        .await
        .unwrap();
        assert_eq!(credential.access_token, "header.payload.signature");
        assert_eq!(credential.scope_seed, "header.payload.signature");
    }

    #[tokio::test]
    async fn fresh_store_token_is_returned_without_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_store(&dir, control(Some(10_000), Some(90_000), Some(90_000)));
        let credential = resolve_token(
            &reqwest::Client::new(),
            None,
            Some(&path),
            "http://127.0.0.1:9/token",
            now(),
        )
        .await
        .unwrap();
        assert_eq!(credential.access_token, "header.payload.signature");
        // Stable across rotations: seeded by org identity, not the JWT.
        assert_eq!(credential.scope_seed, "oauth:org-1");
    }

    #[tokio::test]
    async fn missing_store_reports_relogin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.json");
        let error = resolve_token(
            &reqwest::Client::new(),
            None,
            Some(&path),
            "http://127.0.0.1:9/token",
            now(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Credentials(_)));
        assert!(error.to_string().contains("parallel-cli login"));
    }

    #[tokio::test]
    async fn expired_access_token_refreshes_and_persists_rotation() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/getServiceKeys/token")
            .match_header("Content-Type", "application/x-www-form-urlencoded")
            .match_body(mockito::Matcher::Regex(
                r"(^|&)grant_type=refresh_token(&|$)".into(),
            ))
            .match_body(mockito::Matcher::Regex(
                r"(^|&)client_id=client-abc(&|$)".into(),
            ))
            .match_body(mockito::Matcher::Regex(
                r"(^|&)refresh_token=refresh-secret(&|$)".into(),
            ))
            .with_status(200)
            .with_header("Content-Type", "application/json")
            .with_body(
                serde_json::json!({
                    "access_token": "rotated.access.token",
                    "refresh_token": "rotated-refresh",
                    "expires_in": 420,
                    "refresh_token_expires_in": 2_592_000,
                    "authorization_expires_in": 2_592_000,
                    "org_id": "org-1",
                    "org_name": "Acme",
                    "scope": "balance:read",
                    "token_type": "Bearer",
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_store(&dir, control(Some(1_000), Some(90_000), Some(90_000)));
        let credential = resolve_token(
            &reqwest::Client::new(),
            None,
            Some(&path),
            &format!("{}/getServiceKeys/token", server.url()),
            now(),
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(credential.access_token, "rotated.access.token");
        assert_eq!(credential.scope_seed, "oauth:org-1");
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/access_token")
                .unwrap(),
            "rotated.access.token"
        );
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/refresh_token")
                .unwrap(),
            "rotated-refresh"
        );
        assert_eq!(
            saved
                .pointer("/orgs/org-1/control_api/access_token_expires_at")
                .unwrap(),
            &Value::from(5_420)
        );
    }

    #[tokio::test]
    async fn rejected_refresh_grant_reports_relogin() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/getServiceKeys/token")
            .with_status(400)
            .with_body("{\"error\": \"invalid_grant\"}")
            .create_async()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let path = write_store(&dir, control(Some(1_000), Some(90_000), Some(90_000)));
        let error = resolve_token(
            &reqwest::Client::new(),
            None,
            Some(&path),
            &format!("{}/getServiceKeys/token", server.url()),
            now(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Credentials(_)));
    }
}
