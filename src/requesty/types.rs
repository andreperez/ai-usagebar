//! Wire types for Requesty's documented organization management endpoints.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

use crate::error::{AppError, Result};
use crate::usage::{RequestyUsage, finite_amount};

#[derive(Debug, Clone, Deserialize)]
pub struct OrganizationResponse {
    pub name: String,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub balance: f64,
}

fn deserialize_f64_or_string<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: Deserializer<'de>,
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

impl OrganizationResponse {
    pub fn into_organization(self) -> Result<(String, f64)> {
        if self.name.trim().is_empty() {
            return Err(AppError::Schema(
                "Requesty organization response has an empty name".into(),
            ));
        }
        Ok((
            self.name,
            finite_amount("Requesty", "organization.balance", self.balance)?,
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageResponse {
    pub usage: BTreeMap<String, UsageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageEntry {
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub spend: f64,
    pub total_requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl UsageResponse {
    pub fn into_usage(self) -> Result<RequestyUsage> {
        self.usage
            .into_iter()
            .try_fold(RequestyUsage::default(), |mut total, (period, entry)| {
                let spend = finite_amount("Requesty", "usage.spend", entry.spend)?;
                if spend < 0.0 {
                    return Err(AppError::Schema(format!(
                        "Requesty usage has negative spend for {period}"
                    )));
                }
                total.mtd_spend =
                    finite_amount("Requesty", "usage total spend", total.mtd_spend + spend)?;
                total.requests = total
                    .requests
                    .checked_add(entry.total_requests)
                    .ok_or_else(|| {
                        AppError::Schema("Requesty usage request count overflows u64".into())
                    })?;
                total.input_tokens = total
                    .input_tokens
                    .checked_add(entry.input_tokens)
                    .ok_or_else(|| {
                        AppError::Schema("Requesty usage input token count overflows u64".into())
                    })?;
                total.output_tokens = total
                    .output_tokens
                    .checked_add(entry.output_tokens)
                    .ok_or_else(|| {
                        AppError::Schema("Requesty usage output token count overflows u64".into())
                    })?;
                total.total_tokens = total
                    .total_tokens
                    .checked_add(entry.total_tokens)
                    .ok_or_else(|| {
                        AppError::Schema("Requesty usage total token count overflows u64".into())
                    })?;
                Ok(total)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_schema_accepts_numeric_balance() {
        let organization =
            serde_json::from_str::<OrganizationResponse>(r#"{"name":"Acme Corp","balance":42.5}"#)
                .unwrap()
                .into_organization()
                .unwrap();
        assert_eq!(organization.0, "Acme Corp");
        assert_eq!(organization.1, 42.5);
    }

    #[test]
    fn organization_schema_accepts_string_balance() {
        let organization =
            serde_json::from_str::<OrganizationResponse>(r#"{"name":"Acme Corp","balance":"0"}"#)
                .unwrap()
                .into_organization()
                .unwrap();
        assert_eq!(organization.1, 0.0);
    }

    #[test]
    fn usage_map_aggregates_documented_daily_fields() {
        let usage = serde_json::from_str::<UsageResponse>(
            r#"{"usage":{"2026-08-01":{"spend":1.25,"total_requests":10,"input_tokens":1200,"output_tokens":800,"total_tokens":2000},"2026-08-02":{"spend":2.5,"total_requests":7,"input_tokens":900,"output_tokens":600,"total_tokens":1500}}}"#,
        )
        .unwrap()
        .into_usage()
        .unwrap();
        assert_eq!(usage.mtd_spend, 3.75);
        assert_eq!(usage.requests, 17);
        assert_eq!(usage.input_tokens, 2100);
        assert_eq!(usage.output_tokens, 1400);
        assert_eq!(usage.total_tokens, 3500);
    }

    #[test]
    fn usage_schema_accepts_string_spend() {
        let usage = serde_json::from_str::<UsageResponse>(
            r#"{"usage":{"2026-08-01":{"spend":"1.25","total_requests":10,"input_tokens":1200,"output_tokens":800,"total_tokens":2000}}}"#,
        )
        .unwrap()
        .into_usage()
        .unwrap();
        assert_eq!(usage.mtd_spend, 1.25);
    }

    #[test]
    fn empty_usage_map_is_a_zero_month_to_date_total() {
        let usage = serde_json::from_str::<UsageResponse>(r#"{"usage":{}}"#)
            .unwrap()
            .into_usage()
            .unwrap();
        assert_eq!(usage, RequestyUsage::default());
    }

    #[test]
    fn negative_usage_spend_is_schema_drift() {
        let error = serde_json::from_str::<UsageResponse>(
            r#"{"usage":{"2026-08-01":{"spend":-1,"total_requests":0,"input_tokens":0,"output_tokens":0,"total_tokens":0}}}"#,
        )
        .unwrap()
        .into_usage()
        .unwrap_err();
        assert!(error.to_string().contains("negative spend"));
    }

    #[test]
    fn usage_counter_overflow_is_schema_drift() {
        let raw = format!(
            r#"{{"usage":{{"2026-08-01":{{"spend":0,"total_requests":{},"input_tokens":0,"output_tokens":0,"total_tokens":0}},"2026-08-02":{{"spend":0,"total_requests":1,"input_tokens":0,"output_tokens":0,"total_tokens":0}}}}}}"#,
            u64::MAX
        );
        let error = serde_json::from_str::<UsageResponse>(&raw)
            .unwrap()
            .into_usage()
            .unwrap_err();
        assert!(error.to_string().contains("request count overflows"));
    }
}
