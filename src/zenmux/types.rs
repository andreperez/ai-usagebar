//! Wire types for ZenMux's documented management endpoints.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::usage::{
    UsageWindow, ZenMuxPayg, ZenMuxQuota, ZenMuxStatus, ZenMuxSubscription, finite_amount,
};

const MAX_TEXT_CHARS: usize = 128;

#[derive(Debug, Clone, Deserialize)]
pub struct PaygEnvelope {
    pub success: bool,
    #[serde(default)]
    pub data: Option<PaygData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaygData {
    pub currency: String,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub total_credits: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub top_up_credits: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub bonus_credits: f64,
}

impl PaygEnvelope {
    pub fn into_payg(self) -> Result<ZenMuxPayg> {
        if !self.success {
            return Err(AppError::Schema(
                "ZenMux PAYG response reported success=false".into(),
            ));
        }
        let data = self
            .data
            .ok_or_else(|| AppError::Schema("ZenMux PAYG response is missing data".into()))?;
        if !data.currency.eq_ignore_ascii_case("usd") {
            return Err(AppError::Schema(
                "ZenMux PAYG response uses an unsupported currency".into(),
            ));
        }
        Ok(ZenMuxPayg {
            total_credits: data.total_credits,
            top_up_credits: data.top_up_credits,
            bonus_credits: data.bonus_credits,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionEnvelope {
    pub success: bool,
    #[serde(default)]
    pub data: Option<SubscriptionData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionData {
    pub plan: Plan,
    pub currency: String,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub base_usd_per_flow: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub effective_usd_per_flow: f64,
    pub account_status: String,
    pub quota_5_hour: Quota,
    pub quota_7_day: Quota,
    pub quota_monthly: MonthlyQuota,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    pub tier: String,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub amount_usd: f64,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Quota {
    #[serde(deserialize_with = "de_fraction")]
    pub usage_percentage: f64,
    #[serde(default)]
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub max_flows: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub used_flows: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub remaining_flows: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub used_value_usd: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub max_value_usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonthlyQuota {
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub max_flows: f64,
    #[serde(deserialize_with = "de_nonnegative_amount")]
    pub max_value_usd: f64,
}

impl SubscriptionEnvelope {
    pub fn into_subscription(self) -> Result<ZenMuxSubscription> {
        if !self.success {
            return Err(AppError::Schema(
                "ZenMux subscription response reported success=false".into(),
            ));
        }
        let data = self.data.ok_or_else(|| {
            AppError::Schema("ZenMux subscription response is missing data".into())
        })?;
        if !data.currency.eq_ignore_ascii_case("usd") {
            return Err(AppError::Schema(
                "ZenMux subscription response uses an unsupported currency".into(),
            ));
        }
        let tier = checked_text("subscription plan tier", data.plan.tier)?;
        let status = parse_status(data.account_status)?;
        Ok(ZenMuxSubscription {
            tier,
            plan_amount_usd: data.plan.amount_usd,
            expires_at: data.plan.expires_at,
            status,
            base_usd_per_flow: data.base_usd_per_flow,
            effective_usd_per_flow: data.effective_usd_per_flow,
            five_hour: into_quota(data.quota_5_hour, chrono::Duration::hours(5)),
            seven_day: into_quota(data.quota_7_day, chrono::Duration::days(7)),
            monthly_max_flows: data.quota_monthly.max_flows,
            monthly_max_value_usd: data.quota_monthly.max_value_usd,
        })
    }
}

fn into_quota(quota: Quota, window_duration: chrono::Duration) -> ZenMuxQuota {
    ZenMuxQuota {
        window: UsageWindow {
            utilization_pct: (quota.usage_percentage * 100.0).round() as i32,
            resets_at: quota.resets_at,
            window_duration,
        },
        max_flows: quota.max_flows,
        used_flows: quota.used_flows,
        remaining_flows: quota.remaining_flows,
        used_value_usd: quota.used_value_usd,
        max_value_usd: quota.max_value_usd,
    }
}

fn parse_status(value: String) -> Result<ZenMuxStatus> {
    let value = checked_text("account status", value)?;
    Ok(match value.as_str() {
        "healthy" => ZenMuxStatus::Healthy,
        "monitored" => ZenMuxStatus::Monitored,
        "abusive" => ZenMuxStatus::Abusive,
        "suspended" => ZenMuxStatus::Suspended,
        "banned" => ZenMuxStatus::Banned,
        _ => ZenMuxStatus::Other(value),
    })
}

fn checked_text(field: &str, value: String) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_TEXT_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(AppError::Schema(format!("ZenMux {field} is invalid")));
    }
    Ok(value.to_string())
}

fn de_nonnegative_amount<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    let value =
        finite_amount("ZenMux", "numeric field", value).map_err(serde::de::Error::custom)?;
    if value < 0.0 {
        return Err(serde::de::Error::custom(
            "numeric field must be non-negative",
        ));
    }
    Ok(value)
}

fn de_fraction<'de, D>(deserializer: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = de_nonnegative_amount(deserializer)?;
    if value > 1.0 {
        return Err(serde::de::Error::custom(
            "usage_percentage must be between zero and one",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYG: &str = r#"{"success":true,"data":{"currency":"usd","total_credits":482.74,"top_up_credits":35.0,"bonus_credits":447.74}}"#;
    const SUBSCRIPTION: &str = r#"{
        "success":true,
        "data":{
            "plan":{"tier":"ultra","amount_usd":200,"interval":"month","expires_at":"2026-04-12T08:26:56.000Z"},
            "currency":"usd",
            "base_usd_per_flow":0.03283,
            "effective_usd_per_flow":0.03283,
            "account_status":"healthy",
            "quota_5_hour":{"usage_percentage":0.0715,"resets_at":"2026-03-24T08:35:09.000Z","max_flows":800,"used_flows":57.2,"remaining_flows":742.8,"used_value_usd":1.88,"max_value_usd":26.27},
            "quota_7_day":{"usage_percentage":0.0673,"resets_at":null,"max_flows":6182,"used_flows":416.11,"remaining_flows":5765.89,"used_value_usd":13.66,"max_value_usd":202.99},
            "quota_monthly":{"max_flows":34560,"max_value_usd":1134.33}
        }
    }"#;

    #[test]
    fn payg_envelope_parses_documented_shape() {
        let payg = serde_json::from_str::<PaygEnvelope>(PAYG)
            .unwrap()
            .into_payg()
            .unwrap();
        assert_eq!(payg.total_credits, 482.74);
        assert_eq!(payg.bonus_credits, 447.74);
    }

    #[test]
    fn subscription_envelope_converts_fractional_windows() {
        let subscription = serde_json::from_str::<SubscriptionEnvelope>(SUBSCRIPTION)
            .unwrap()
            .into_subscription()
            .unwrap();
        assert_eq!(subscription.tier, "ultra");
        assert_eq!(subscription.status, ZenMuxStatus::Healthy);
        assert_eq!(subscription.five_hour.window.utilization_pct, 7);
        assert_eq!(subscription.seven_day.window.utilization_pct, 7);
        assert!(subscription.seven_day.window.resets_at.is_none());
    }

    #[test]
    fn unknown_status_stays_visible_without_becoming_healthy() {
        let raw = SUBSCRIPTION.replace("\"healthy\"", "\"grace_period_unknown\"");
        let subscription = serde_json::from_str::<SubscriptionEnvelope>(&raw)
            .unwrap()
            .into_subscription()
            .unwrap();
        assert_eq!(
            subscription.status,
            ZenMuxStatus::Other("grace_period_unknown".into())
        );
    }

    #[test]
    fn invalid_usage_fraction_is_rejected() {
        let raw = SUBSCRIPTION.replace("\"usage_percentage\":0.0715", "\"usage_percentage\":1.01");
        assert!(serde_json::from_str::<SubscriptionEnvelope>(&raw).is_err());
    }

    #[test]
    fn non_usd_payg_is_schema_drift() {
        let raw = PAYG.replace("\"usd\"", "\"eur\"");
        let error = serde_json::from_str::<PaygEnvelope>(&raw)
            .unwrap()
            .into_payg()
            .unwrap_err();
        assert!(error.to_string().contains("unsupported currency"));
    }

    #[test]
    fn success_false_does_not_require_data() {
        let error = serde_json::from_str::<SubscriptionEnvelope>(r#"{"success":false}"#)
            .unwrap()
            .into_subscription()
            .unwrap_err();
        assert!(error.to_string().contains("success=false"));
    }
}
