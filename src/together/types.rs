//! Wire types and aggregation for Together AI billing usage.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::usage::{TogetherProductCost, TogetherSnapshot, finite_amount, parse_amount};

#[derive(Debug, Clone, Deserialize)]
pub struct BillingUsageResponse {
    pub organization_id: String,
    pub billing_period: String,
    pub currency: String,
    pub earliest_window_start: Option<DateTime<Utc>>,
    pub latest_window_end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub data: Vec<BillingWindow>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BillingWindow {
    #[serde(default)]
    pub line_items: Vec<BillingLineItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BillingLineItem {
    pub product_name: String,
    pub cost: String,
}

#[derive(Debug, Default)]
pub struct UsageAccumulator {
    organization_id: Option<String>,
    billing_period: Option<String>,
    currency: Option<String>,
    monthly_spend: f64,
    products: BTreeMap<String, f64>,
    earliest_window_start: Option<DateTime<Utc>>,
    latest_window_end: Option<DateTime<Utc>>,
}

impl UsageAccumulator {
    pub fn push(&mut self, response: BillingUsageResponse) -> Result<Option<String>> {
        require_nonempty("organization_id", &response.organization_id)?;
        require_billing_period(&response.billing_period)?;
        require_nonempty("currency", &response.currency)?;
        require_consistent(
            "organization_id",
            &mut self.organization_id,
            &response.organization_id,
        )?;
        require_consistent(
            "billing_period",
            &mut self.billing_period,
            &response.billing_period,
        )?;
        require_consistent("currency", &mut self.currency, &response.currency)?;

        self.earliest_window_start =
            min_time(self.earliest_window_start, response.earliest_window_start);
        self.latest_window_end = max_time(self.latest_window_end, response.latest_window_end);

        for item in response
            .data
            .into_iter()
            .flat_map(|window| window.line_items)
        {
            let name = item.product_name.trim();
            if name.is_empty() {
                return Err(AppError::Schema(
                    "Together AI billing line item has an empty product_name".into(),
                ));
            }
            let cost = parse_amount("Together AI", "line_items[].cost", &item.cost)?;
            self.monthly_spend =
                finite_amount("Together AI", "monthly spend", self.monthly_spend + cost)?;
            let product = self.products.entry(name.to_string()).or_default();
            *product = finite_amount("Together AI", "product spend", *product + cost)?;
        }

        Ok(response.next_cursor.filter(|cursor| !cursor.is_empty()))
    }

    pub fn finish(self) -> Result<TogetherSnapshot> {
        let organization_id = self
            .organization_id
            .ok_or_else(|| AppError::Schema("Together AI billing returned no pages".into()))?;
        let billing_period = self.billing_period.ok_or_else(|| {
            AppError::Schema("Together AI billing returned no billing period".into())
        })?;
        let currency = self
            .currency
            .ok_or_else(|| AppError::Schema("Together AI billing returned no currency".into()))?;
        let mut products: Vec<TogetherProductCost> = self
            .products
            .into_iter()
            .map(|(name, cost)| TogetherProductCost { name, cost })
            .collect();
        products.sort_by(|a, b| {
            b.cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(TogetherSnapshot {
            organization_id,
            billing_period,
            currency,
            monthly_spend: self.monthly_spend,
            products,
            earliest_window_start: self.earliest_window_start,
            latest_window_end: self.latest_window_end,
        })
    }
}

pub fn validate_snapshot(snapshot: TogetherSnapshot) -> Result<TogetherSnapshot> {
    require_nonempty("organization_id", &snapshot.organization_id)?;
    require_billing_period(&snapshot.billing_period)?;
    require_nonempty("currency", &snapshot.currency)?;
    finite_amount("Together AI", "monthly spend", snapshot.monthly_spend)?;
    for product in &snapshot.products {
        if product.name.trim().is_empty() {
            return Err(AppError::Schema(
                "Together AI cache has an empty product name".into(),
            ));
        }
        finite_amount("Together AI", "product spend", product.cost)?;
    }
    Ok(snapshot)
}

fn require_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AppError::Schema(format!(
            "Together AI billing response has an empty {field}"
        )));
    }
    Ok(())
}

fn require_billing_period(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit)
        && (1..=12).contains(&((bytes[5] - b'0') * 10 + bytes[6] - b'0'));
    if !valid {
        return Err(AppError::Schema(format!(
            "Together AI billing response has invalid billing_period {value:?}"
        )));
    }
    Ok(())
}

fn require_consistent(field: &str, stored: &mut Option<String>, value: &str) -> Result<()> {
    match stored {
        Some(current) if current != value => Err(AppError::Schema(format!(
            "Together AI billing pages disagree on {field}"
        ))),
        Some(_) => Ok(()),
        None => {
            *stored = Some(value.to_string());
            Ok(())
        }
    }
}

fn min_time(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn max_time(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(raw: &str) -> BillingUsageResponse {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn aggregates_costs_and_sorts_products() {
        let mut acc = UsageAccumulator::default();
        acc.push(response(
            r#"{
                "organization_id":"org-test","billing_period":"2026-09","currency":"USD",
                "earliest_window_start":"2026-09-01T00:00:00Z",
                "latest_window_end":"2026-09-03T00:00:00Z",
                "data":[
                    {"line_items":[{"product_name":"Serverless Input","cost":"0.10"}]},
                    {"line_items":[{"product_name":"Serverless Output","cost":"0.20"},{"product_name":"Serverless Input","cost":"0.01"}]}
                ],"next_cursor":null
            }"#,
        ))
        .unwrap();
        let snapshot = acc.finish().unwrap();
        assert!((snapshot.monthly_spend - 0.31).abs() < 1e-9);
        assert_eq!(snapshot.products[0].name, "Serverless Output");
        assert!((snapshot.products[1].cost - 0.11).abs() < 1e-9);
    }

    #[test]
    fn merges_pages_and_rejects_identity_drift() {
        let page = |organization: &str, cursor: Option<&str>| {
            response(&format!(
                r#"{{"organization_id":"{organization}","billing_period":"2026-09","currency":"USD","earliest_window_start":null,"latest_window_end":null,"data":[],"next_cursor":{}}}"#,
                cursor.map_or("null".into(), |value| format!("\"{value}\""))
            ))
        };
        let mut acc = UsageAccumulator::default();
        assert_eq!(
            acc.push(page("org-test", Some("next"))).unwrap(),
            Some("next".into())
        );
        let error = acc.push(page("org-other", None)).unwrap_err();
        assert!(error.to_string().contains("disagree on organization_id"));
    }

    #[test]
    fn rejects_non_numeric_cost_and_bad_period() {
        let bad_cost = r#"{"organization_id":"org-test","billing_period":"2026-09","currency":"USD","earliest_window_start":null,"latest_window_end":null,"data":[{"line_items":[{"product_name":"Inference","cost":"n/a"}]}],"next_cursor":null}"#;
        let mut acc = UsageAccumulator::default();
        assert!(acc.push(response(bad_cost)).is_err());

        let bad_period = r#"{"organization_id":"org-test","billing_period":"2026-13","currency":"USD","earliest_window_start":null,"latest_window_end":null,"data":[],"next_cursor":null}"#;
        let mut acc = UsageAccumulator::default();
        assert!(acc.push(response(bad_period)).is_err());

        let non_ascii_period = bad_period.replace("2026-13", "éé-000");
        let mut acc = UsageAccumulator::default();
        assert!(acc.push(response(&non_ascii_period)).is_err());
    }
}
