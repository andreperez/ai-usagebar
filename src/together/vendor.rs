//! Together AI monthly-spend renderer.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::format::{money, placeholders, substitute, updated_at_hm};
use crate::pacing::PaceSeverity;
use crate::pango::{color_span, escape, severity_color};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::TogetherSnapshot;
use crate::vendor::{RenderOpts, VendorId, VendorOutcome};
use crate::waybar::{Class, WaybarOutput};

use super::fetch::FetchOutcome;

pub const DEFAULT_FORMAT: &str = "{together_spend}";

pub fn build_placeholders(snapshot: &TogetherSnapshot) -> HashMap<&'static str, String> {
    let spend = money(snapshot.monthly_spend, &snapshot.currency);
    placeholders(vec![
        ("icon", VendorId::Together.short_name().to_string()),
        ("vendor_short", VendorId::Together.short_name().to_string()),
        ("session_pct", "0".into()),
        ("session_reset", "—".into()),
        ("weekly_pct", "0".into()),
        ("weekly_reset", "—".into()),
        ("plan", format!("Together AI — {spend}")),
        ("together_spend", spend),
        ("together_period", snapshot.billing_period.clone()),
        ("currency", snapshot.currency.clone()),
    ])
}

pub fn render(
    outcome: &VendorOutcome,
    snapshot: &TogetherSnapshot,
    theme: &Theme,
    options: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let severity = PaceSeverity::Low;
    let values = build_placeholders(snapshot);
    let format = options
        .format
        .clone()
        .unwrap_or_else(|| DEFAULT_FORMAT.into());
    let mut text = substitute(&format, &values);
    if outcome.stale {
        text.push_str(" ⏸");
    }
    let icon = options
        .icon
        .as_deref()
        .filter(|icon| !icon.is_empty())
        .map_or_else(String::new, |icon| format!("{icon} "));
    let color = severity_color(severity, theme);
    let tooltip = options.tooltip_format.as_deref().map_or_else(
        || render_tooltip(outcome, snapshot, theme, now),
        |template| substitute(template, &values),
    );
    WaybarOutput {
        text: color_span(color, &format!("{icon}{text}")),
        tooltip,
        class: Class::from(severity),
    }
}

fn render_tooltip(
    outcome: &VendorOutcome,
    snapshot: &TogetherSnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let mut lines = vec![
        TooltipLine::Center(format!(
            "<span font_weight='bold' foreground='{blue}'>Together AI</span>",
            blue = theme.blue
        )),
        TooltipLine::Sep,
        TooltipLine::Body(String::new()),
        TooltipLine::Body(format!(
            " <span foreground='{fg}'>  Monthly spend</span>",
            fg = theme.fg
        )),
        TooltipLine::Body(format!(
            "   <span font_weight='bold' foreground='{color}'>{}</span>",
            escape(&money(snapshot.monthly_spend, &snapshot.currency)),
            color = severity_color(PaceSeverity::Low, theme)
        )),
        TooltipLine::Body(format!(
            " <span foreground='{dim}'>  Billing period {}</span>",
            escape(&snapshot.billing_period),
            dim = theme.dim
        )),
    ];
    if let Some(end) = snapshot.latest_window_end {
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{dim}'>  Finalized through {}</span>",
            escape(&end.format("%Y-%m-%d %H:%M UTC").to_string()),
            dim = theme.dim
        )));
    }
    if !snapshot.products.is_empty() {
        lines.push(TooltipLine::Body(String::new()));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{fg}'>  Products</span>",
            fg = theme.fg
        )));
        for product in snapshot.products.iter().take(5) {
            lines.push(TooltipLine::Body(format!(
                "   <span foreground='{dim}'>{}: {}</span>",
                escape(&product.name),
                escape(&money(product.cost, &snapshot.currency)),
                dim = theme.dim
            )));
        }
    }
    if let Some((status, message)) = &outcome.last_error {
        lines.push(TooltipLine::Body(String::new()));
        lines.push(TooltipLine::Sep);
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{orange}'>  HTTP {status}: {}</span>",
            escape(message),
            orange = theme.orange
        )));
    }
    lines.push(TooltipLine::Body(String::new()));
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{dim}'>  Updated {}</span>",
        updated_at_hm(now, outcome.cache_age),
        dim = theme.dim
    )));
    render_bordered(&lines, theme)
}

impl From<FetchOutcome> for VendorOutcome {
    fn from(outcome: FetchOutcome) -> Self {
        outcome.map(crate::usage::VendorSnapshot::Together)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::TogetherProductCost;

    fn snapshot() -> TogetherSnapshot {
        TogetherSnapshot {
            organization_id: "org-test".into(),
            billing_period: "2026-09".into(),
            currency: "USD".into(),
            monthly_spend: 0.31,
            products: vec![TogetherProductCost {
                name: "Serverless Inference".into(),
                cost: 0.31,
            }],
            earliest_window_start: None,
            latest_window_end: None,
        }
    }

    #[test]
    fn placeholders_and_default_render_show_monthly_spend() {
        let snapshot = snapshot();
        let values = build_placeholders(&snapshot);
        assert_eq!(values["together_spend"], "$0.31");
        let outcome = VendorOutcome {
            snapshot: crate::usage::VendorSnapshot::Together(snapshot.clone()),
            stale: false,
            last_error: None,
            cache_age: None,
        };
        let output = render(
            &outcome,
            &snapshot,
            &Theme::default(),
            &RenderOpts {
                format: None,
                tooltip_format: None,
                icon: None,
                pace_tolerance: 5,
                format_pace_color: false,
                tooltip_pace_pts: false,
            },
            Utc::now(),
        );
        assert!(output.text.contains("$0.31"));
        assert!(output.tooltip.contains("Monthly spend"));
        assert!(output.tooltip.contains("Serverless Inference"));
    }
}
