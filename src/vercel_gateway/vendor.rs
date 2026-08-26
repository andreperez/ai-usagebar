//! Vercel AI Gateway renderer.

use super::fetch::FetchOutcome;
use crate::format::{placeholders, substitute, updated_at_hm};
use crate::pacing::PaceSeverity;
use crate::pango::{color_span, escape, severity_color};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::VercelGatewaySnapshot;
use crate::vendor::{RenderOpts, VendorOutcome};
use crate::waybar::{Class, WaybarOutput};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub const DEFAULT_FORMAT: &str = "{vag_balance}";
pub fn severity(snapshot: &VercelGatewaySnapshot) -> PaceSeverity {
    if snapshot.balance < 1.0 {
        PaceSeverity::Critical
    } else if snapshot.balance < 5.0 {
        PaceSeverity::High
    } else if snapshot.balance < 20.0 {
        PaceSeverity::Mid
    } else {
        PaceSeverity::Low
    }
}
pub fn build_placeholders(snapshot: &VercelGatewaySnapshot) -> HashMap<&'static str, String> {
    let report = snapshot.report.as_ref();
    placeholders(vec![
        ("icon", "󰍹".into()),
        ("vendor_short", "vag".into()),
        ("plan", "Vercel AI Gateway".into()),
        ("session_pct", "".into()),
        ("session_reset", "".into()),
        ("weekly_pct", "".into()),
        ("weekly_reset", "".into()),
        ("vag_balance", money(snapshot.balance)),
        ("vag_used", money(snapshot.total_used)),
        (
            "vag_mtd",
            report
                .map(|r| money(r.mtd_cost))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "vag_requests",
            report
                .map(|r| r.requests.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "vag_input",
            report
                .map(|r| r.input_tokens.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "vag_output",
            report
                .map(|r| r.output_tokens.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
    ])
}
pub fn render(
    outcome: &VendorOutcome,
    snapshot: &VercelGatewaySnapshot,
    theme: &Theme,
    opts: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let values = build_placeholders(snapshot);
    let mut text = substitute(opts.format.as_deref().unwrap_or(DEFAULT_FORMAT), &values);
    if outcome.stale {
        text.push_str(" ⏸");
    }
    let text = color_span(
        severity_color(severity(snapshot), theme),
        &format!(
            "{}{}",
            opts.icon
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(|v| format!("{v} "))
                .unwrap_or_default(),
            text
        ),
    );
    let tooltip = opts
        .tooltip_format
        .as_deref()
        .map(|f| substitute(f, &values))
        .unwrap_or_else(|| tooltip(outcome, snapshot, theme, now));
    WaybarOutput {
        text,
        tooltip,
        class: Class::from(severity(snapshot)),
    }
}
fn money(value: f64) -> String {
    format!("${value:.2}")
}
fn tooltip(
    outcome: &VendorOutcome,
    s: &VercelGatewaySnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let mut lines = vec![
        TooltipLine::Center(format!(
            "<span font_weight='bold' foreground='{}'>Vercel AI Gateway</span>",
            theme.blue
        )),
        TooltipLine::Sep,
        TooltipLine::Body(format!(
            " Balance {} · lifetime used {}",
            money(s.balance),
            money(s.total_used)
        )),
    ];
    if let Some(r) = &s.report {
        lines.push(TooltipLine::Body(format!(
            " Month to date {} · {} requests",
            money(r.mtd_cost),
            r.requests
        )));
        lines.push(TooltipLine::Body(format!(
            " Tokens input {} · output {}",
            r.input_tokens, r.output_tokens
        )));
    }
    if let Some((code, message)) = &outcome.last_error {
        lines.push(TooltipLine::Sep);
        lines.push(TooltipLine::Body(format!(
            " Warning {} {}",
            code,
            escape(message)
        )));
    }
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body(format!(
        " Updated {}",
        updated_at_hm(now, outcome.cache_age)
    )));
    render_bordered(&lines, theme)
}
impl From<FetchOutcome> for VendorOutcome {
    fn from(outcome: FetchOutcome) -> Self {
        Self {
            snapshot: crate::usage::VendorSnapshot::VercelGateway(outcome.snapshot),
            stale: outcome.stale,
            last_error: outcome.last_error,
            cache_age: outcome.cache_age,
        }
    }
}
