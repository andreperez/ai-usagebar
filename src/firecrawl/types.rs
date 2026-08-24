//! Wire types for Firecrawl's documented v2 credit endpoints.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentEnvelope {
    pub success: bool,
    #[serde(default)]
    pub data: Option<CurrentData>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentData {
    #[serde(rename = "remainingCredits", deserialize_with = "de_credit_count")]
    pub remaining_credits: u64,
    #[serde(rename = "planCredits", deserialize_with = "de_credit_count")]
    pub plan_credits: u64,
    #[serde(rename = "billingPeriodStart", default)]
    pub billing_period_start: Option<DateTime<Utc>>,
    #[serde(rename = "billingPeriodEnd", default)]
    pub billing_period_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoricalEnvelope {
    pub success: bool,
    #[serde(default)]
    pub periods: Option<Vec<HistoricalPeriod>>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoricalPeriod {
    #[serde(rename = "startDate")]
    pub start_date: DateTime<Utc>,
    #[serde(rename = "endDate", default)]
    pub end_date: Option<DateTime<Utc>>,
    #[serde(rename = "apiKey", default)]
    pub api_key: Option<String>,
    #[serde(
        rename = "totalCredits",
        alias = "creditsUsed",
        deserialize_with = "de_credit_count"
    )]
    pub total_credits: u64,
}

impl CurrentEnvelope {
    pub fn into_data(self) -> Result<CurrentData> {
        if !self.success {
            return Err(AppError::Schema(
                "Firecrawl current usage response reported success=false".into(),
            ));
        }
        self.data.ok_or_else(|| {
            AppError::Schema("Firecrawl current usage response is missing data".into())
        })
    }
}

impl HistoricalEnvelope {
    pub fn into_periods(self) -> Result<Vec<HistoricalPeriod>> {
        if !self.success {
            return Err(AppError::Schema(
                "Firecrawl historical usage response reported success=false".into(),
            ));
        }
        self.periods.ok_or_else(|| {
            AppError::Schema("Firecrawl historical usage response is missing periods".into())
        })
    }
}

fn de_credit_count<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > u64::MAX as f64 {
        return Err(serde::de::Error::custom(
            "credit count must be a finite non-negative integer",
        ));
    }
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_credit_usage_schema() {
        let raw = r#"{
            "success": true,
            "data": {
                "remainingCredits": 8200,
                "planCredits": 10000,
                "billingPeriodStart": "2026-08-01T00:00:00Z",
                "billingPeriodEnd": "2026-09-01T00:00:00Z"
            }
        }"#;
        let data = serde_json::from_str::<CurrentEnvelope>(raw)
            .unwrap()
            .into_data()
            .unwrap();
        assert_eq!(data.remaining_credits, 8200);
        assert_eq!(data.plan_credits, 10000);
        assert_eq!(
            data.billing_period_start.unwrap().to_rfc3339(),
            "2026-08-01T00:00:00+00:00"
        );
    }

    #[test]
    fn parses_historical_periods_schema() {
        let raw = r#"{
            "success": true,
            "periods": [{
                "startDate": "2026-08-01T00:00:00Z",
                "endDate": "2026-09-01T00:00:00Z",
                "apiKey": null,
                "totalCredits": 1800
            }]
        }"#;
        let periods = serde_json::from_str::<HistoricalEnvelope>(raw)
            .unwrap()
            .into_periods()
            .unwrap();
        assert_eq!(periods.len(), 1);
        assert_eq!(
            periods[0].end_date.unwrap().to_rfc3339(),
            "2026-09-01T00:00:00+00:00"
        );
        assert_eq!(periods[0].total_credits, 1800);
        assert!(periods[0].api_key.is_none());
    }

    #[test]
    fn rejects_fractional_or_negative_credits() {
        for value in ["1.5", "-1"] {
            let raw = format!(
                r#"{{"success":true,"data":{{"remainingCredits":{value},"planCredits":10}}}}"#
            );
            assert!(serde_json::from_str::<CurrentEnvelope>(&raw).is_err());
        }
    }

    #[test]
    fn success_false_needs_no_wire_data() {
        let raw = r#"{"success":false,"error":"not available"}"#;
        let err = serde_json::from_str::<CurrentEnvelope>(raw)
            .unwrap()
            .into_data()
            .unwrap_err();
        assert!(err.to_string().contains("success=false"));
    }
}
