//! Wire types for Parallel's documented Account API balance endpoint.

use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::usage::{ParallelSnapshot, finite_amount};

#[derive(Debug, Clone, Deserialize)]
pub struct BalanceResponse {
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub credit_balance_cents: f64,
    #[serde(deserialize_with = "deserialize_f64_or_string")]
    pub pending_debit_balance_cents: f64,
    pub will_invoice: bool,
}

impl BalanceResponse {
    pub fn into_snapshot(self, scope_fingerprint: String) -> Result<ParallelSnapshot> {
        let credit_balance_cents = nonnegative("credit_balance_cents", self.credit_balance_cents)?;
        let pending_debit_balance_cents = nonnegative(
            "pending_debit_balance_cents",
            self.pending_debit_balance_cents,
        )?;
        if self.will_invoice && (credit_balance_cents != 0.0 || pending_debit_balance_cents != 0.0)
        {
            return Err(AppError::Schema(
                "Parallel invoice account returned a prepaid balance".into(),
            ));
        }
        Ok(ParallelSnapshot {
            credit_balance_cents,
            pending_debit_balance_cents,
            will_invoice: self.will_invoice,
            scope_fingerprint,
        })
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
    let value = finite_amount("Parallel", field, value)?;
    if value < 0.0 {
        return Err(AppError::Schema(format!(
            "Parallel {field} must be non-negative"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prepaid_balance_cents() {
        let snapshot = serde_json::from_str::<BalanceResponse>(
            r#"{"org_id":"org-test","credit_balance_cents":1250,"pending_debit_balance_cents":25,"will_invoice":false}"#,
        )
        .unwrap()
        .into_snapshot("scope".into())
        .unwrap();
        assert_eq!(snapshot.credit_balance_cents, 1250.0);
        assert_eq!(snapshot.pending_debit_balance_cents, 25.0);
    }

    #[test]
    fn invoice_accounts_cannot_report_prepaid_balance() {
        let error = serde_json::from_str::<BalanceResponse>(
            r#"{"credit_balance_cents":1,"pending_debit_balance_cents":0,"will_invoice":true}"#,
        )
        .unwrap()
        .into_snapshot("scope".into())
        .unwrap_err();
        assert!(error.to_string().contains("invoice account"));
    }
}
