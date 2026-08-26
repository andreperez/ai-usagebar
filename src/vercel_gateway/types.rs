//! Wire types for Vercel AI Gateway's documented REST endpoints.

use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::usage::{VercelGatewaySnapshot, VercelReport, finite_amount};

#[derive(Debug, Clone, Deserialize)]
pub struct CreditsResponse {
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub balance: f64,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub total_used: f64,
}

impl CreditsResponse {
    pub fn into_snapshot(self, scope_fingerprint: String) -> Result<VercelGatewaySnapshot> {
        let balance = nonnegative("credits.balance", self.balance)?;
        let total_used = nonnegative("credits.total_used", self.total_used)?;
        Ok(VercelGatewaySnapshot {
            balance,
            total_used,
            report: None,
            scope_fingerprint,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportResponse {
    pub results: Vec<ReportRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportRow {
    pub day: String,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub total_cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

impl ReportResponse {
    pub fn into_report(
        self,
        interval_start: chrono::DateTime<chrono::Utc>,
        interval_end: chrono::DateTime<chrono::Utc>,
    ) -> Result<VercelReport> {
        if interval_end < interval_start {
            return Err(AppError::Schema(
                "Vercel report interval end precedes its start".into(),
            ));
        }
        self.results.into_iter().try_fold(
            VercelReport {
                mtd_cost: 0.0,
                input_tokens: 0,
                output_tokens: 0,
                requests: 0,
                interval_start,
                interval_end,
            },
            |mut total, row| {
                if chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d").is_err() {
                    return Err(AppError::Schema("Vercel report row has invalid day".into()));
                }
                total.mtd_cost = finite_amount(
                    "Vercel report",
                    "running total_cost",
                    total.mtd_cost + nonnegative("report.total_cost", row.total_cost)?,
                )?;
                total.input_tokens = total
                    .input_tokens
                    .checked_add(row.input_tokens)
                    .ok_or_else(|| {
                        AppError::Schema("Vercel report input token count overflows u64".into())
                    })?;
                total.output_tokens = total
                    .output_tokens
                    .checked_add(row.output_tokens)
                    .ok_or_else(|| {
                        AppError::Schema("Vercel report output token count overflows u64".into())
                    })?;
                total.requests =
                    total
                        .requests
                        .checked_add(row.request_count)
                        .ok_or_else(|| {
                            AppError::Schema("Vercel report request count overflows u64".into())
                        })?;
                Ok(total)
            },
        )
    }
}

fn deserialize_f64_or_string<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumberOrString {
        Number(f64),
        String(String),
    }

    match NumberOrString::deserialize(deserializer)? {
        NumberOrString::Number(value) => Ok(value),
        NumberOrString::String(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

fn nonnegative(field: &str, value: f64) -> Result<f64> {
    let value = finite_amount("Vercel", field, value)?;
    if value < 0.0 {
        return Err(AppError::Schema(format!(
            "Vercel {field} must be non-negative"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn credits_accept_documented_string_amounts() {
        let snapshot =
            serde_json::from_str::<CreditsResponse>(r#"{"balance":"95.50","total_used":"4.50"}"#)
                .unwrap()
                .into_snapshot("scope".into())
                .unwrap();
        assert_eq!(snapshot.balance, 95.5);
        assert_eq!(snapshot.total_used, 4.5);
    }

    #[test]
    fn report_aggregates_documented_results_fields() {
        let report = serde_json::from_str::<ReportResponse>(r#"{"results":[{"day":"2026-08-01","total_cost":1.25,"input_tokens":100,"output_tokens":20,"request_count":2},{"day":"2026-08-02","total_cost":2.5,"input_tokens":200,"output_tokens":40,"request_count":3}]}"#)
            .unwrap()
            .into_report(
                chrono::Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
                chrono::Utc.with_ymd_and_hms(2026, 8, 25, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(report.mtd_cost, 3.75);
        assert_eq!(report.input_tokens, 300);
        assert_eq!(report.output_tokens, 60);
        assert_eq!(report.requests, 5);
    }
}
