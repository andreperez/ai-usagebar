//! Parallel Account API renderer.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::format::{placeholders, substitute, updated_at_hm};
use crate::pacing::PaceSeverity;
use crate::pango::{color_span, escape, severity_color};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::ParallelSnapshot;
use crate::vendor::{RenderOpts, VendorOutcome};
use crate::waybar::{Class, WaybarOutput};

use super::fetch::FetchOutcome;

pub const DEFAULT_FORMAT: &str = "{prl_balance}";

pub fn severity(snapshot: &ParallelSnapshot) -> PaceSeverity {
    if snapshot.will_invoice {
        PaceSeverity::Low
    } else if snapshot.credit_balance_cents < 100.0 {
        PaceSeverity::Critical
    } else if snapshot.credit_balance_cents < 500.0 {
        PaceSeverity::High
    } else if snapshot.credit_balance_cents < 2_000.0 {
        PaceSeverity::Mid
    } else {
        PaceSeverity::Low
    }
}

pub fn build_placeholders(snapshot: &ParallelSnapshot) -> HashMap<&'static str, String> {
    let balance = snapshot
        .prepaid_balance()
        .map(crate::format::usd)
        .unwrap_or_else(|| "Invoice".into());
    placeholders(vec![
        ("icon", "󰢘".into()),
        ("vendor_short", "prl".into()),
        ("plan", "Parallel".into()),
        ("session_pct", "".into()),
        ("session_reset", "".into()),
        ("weekly_pct", "".into()),
        ("weekly_reset", "".into()),
        ("prl_balance", balance),
        (
            "prl_pending_debit",
            snapshot
                .pending_debit()
                .map(crate::format::usd)
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "prl_billing_mode",
            if snapshot.will_invoice {
                "Invoice".into()
            } else {
                "Prepaid".into()
            },
        ),
    ])
}

pub fn render(
    outcome: &VendorOutcome,
    snapshot: &ParallelSnapshot,
    theme: &Theme,
    opts: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let values = build_placeholders(snapshot);
    let mut text = substitute(opts.format.as_deref().unwrap_or(DEFAULT_FORMAT), &values);
    if outcome.stale {
        text.push_str(" ⏸");
    }
    let tooltip = opts
        .tooltip_format
        .as_deref()
        .map(|format| substitute(format, &values))
        .unwrap_or_else(|| tooltip(outcome, snapshot, theme, now));
    WaybarOutput {
        text: color_span(
            severity_color(severity(snapshot), theme),
            &format!(
                "{}{}",
                opts.icon
                    .as_deref()
                    .filter(|icon| !icon.is_empty())
                    .map(|icon| format!("{icon} "))
                    .unwrap_or_default(),
                text
            ),
        ),
        tooltip,
        class: Class::from(severity(snapshot)),
    }
}

fn tooltip(
    outcome: &VendorOutcome,
    snapshot: &ParallelSnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let mut lines = vec![
        TooltipLine::Center(format!(
            "<span font_weight='bold' foreground='{}'>Parallel</span>",
            theme.blue
        )),
        TooltipLine::Sep,
    ];
    if snapshot.will_invoice {
        lines.push(TooltipLine::Body(" Billing mode Invoice".into()));
    } else {
        lines.push(TooltipLine::Body(format!(
            " Prepaid balance {}",
            crate::format::usd(snapshot.prepaid_balance().unwrap_or_default())
        )));
        lines.push(TooltipLine::Body(format!(
            " Pending debit {}",
            crate::format::usd(snapshot.pending_debit().unwrap_or_default())
        )));
    }
    if let Some((code, message)) = &outcome.last_error {
        lines.push(TooltipLine::Sep);
        lines.push(TooltipLine::Body(format!(
            " Warning {code} {}",
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
            snapshot: crate::usage::VendorSnapshot::Parallel(outcome.snapshot),
            stale: outcome.stale,
            last_error: outcome.last_error,
            cache_age: outcome.cache_age,
        }
    }
}
