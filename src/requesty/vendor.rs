//! Requesty renderer — organization balance and optional month-to-date usage.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::format::{placeholders, substitute, updated_at_hm};
use crate::pacing::PaceSeverity;
use crate::pango::{color_span, escape, severity_color};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::RequestySnapshot;
use crate::vendor::{RenderOpts, VendorOutcome};
use crate::waybar::{Class, WaybarOutput};

use super::fetch::{FetchOutcome, SCHEMA_DRIFT_MESSAGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    Http(u16),
    SchemaDrift,
    Other,
}

pub fn warning_kind(code: u16, message: &str) -> WarningKind {
    if code != 0 {
        WarningKind::Http(code)
    } else if message == SCHEMA_DRIFT_MESSAGE {
        WarningKind::SchemaDrift
    } else {
        WarningKind::Other
    }
}

pub const DEFAULT_FORMAT: &str = "{rqy_balance}";

pub fn headline(snapshot: &RequestySnapshot) -> String {
    format_money(snapshot.balance)
}

pub fn build_placeholders(snapshot: &RequestySnapshot) -> HashMap<&'static str, String> {
    let usage = snapshot.usage.as_ref();
    placeholders(vec![
        ("icon", "󰧑".to_string()),
        ("vendor_short", "rqy".to_string()),
        ("plan", snapshot.org_name.clone()),
        ("session_pct", "0".to_string()),
        ("session_reset", "—".to_string()),
        ("weekly_pct", "0".to_string()),
        ("weekly_reset", "—".to_string()),
        ("rqy_headline", headline(snapshot)),
        ("rqy_org", snapshot.org_name.clone()),
        ("rqy_balance", format_money(snapshot.balance)),
        (
            "rqy_mtd",
            usage
                .map(|usage| format_money(usage.mtd_spend))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "rqy_requests",
            usage
                .map(|usage| usage.requests.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "rqy_tokens",
            usage
                .map(|usage| usage.total_tokens.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "rqy_input",
            usage
                .map(|usage| usage.input_tokens.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "rqy_output",
            usage
                .map(|usage| usage.output_tokens.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
    ])
}

pub fn severity(snapshot: &RequestySnapshot) -> PaceSeverity {
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

pub fn render(
    outcome: &VendorOutcome,
    snapshot: &RequestySnapshot,
    theme: &Theme,
    opts: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let class = Class::from(severity(snapshot));
    let format = opts
        .format
        .clone()
        .unwrap_or_else(|| DEFAULT_FORMAT.to_string());
    let values = build_placeholders(snapshot);
    let mut pango_values = values.clone();
    if let Some(plan) = pango_values.get_mut("plan") {
        *plan = escape(plan);
    }
    if let Some(org) = pango_values.get_mut("rqy_org") {
        *org = escape(org);
    }
    let mut text = substitute(&format, &pango_values);
    if outcome.stale {
        text.push_str(" ⏸");
    }
    let icon_prefix = match opts.icon.as_deref() {
        Some(icon) if !icon.is_empty() => format!("{icon} "),
        _ => String::new(),
    };
    let bar_text = color_span(
        severity_color(severity(snapshot), theme),
        &format!("{icon_prefix}{text}"),
    );
    let tooltip = if let Some(format) = opts.tooltip_format.as_deref() {
        substitute(format, &pango_values)
    } else {
        render_tooltip(outcome, snapshot, theme, now)
    };
    WaybarOutput {
        text: bar_text,
        tooltip,
        class,
    }
}

fn format_money(amount: f64) -> String {
    format!("${amount:.2}")
}

fn render_tooltip(
    outcome: &VendorOutcome,
    snapshot: &RequestySnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let color = severity_color(severity(snapshot), theme);
    let mut lines = vec![TooltipLine::Center(format!(
        "<span font_weight='bold' foreground='{}'>Requesty — {}</span>",
        theme.blue,
        escape(&snapshot.org_name)
    ))];
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body("".into()));
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{}'>  󰢗  Balance</span>",
        theme.fg
    )));
    lines.push(TooltipLine::Body(format!(
        "   <span font_weight='bold' foreground='{color}'>{}</span>",
        format_money(snapshot.balance)
    )));
    if let Some(usage) = &snapshot.usage {
        lines.push(TooltipLine::Body("".into()));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>  󰏖  Month to date</span>",
            theme.fg
        )));
        lines.push(TooltipLine::Body(format!(
            "   <span foreground='{}'>{} · {} requests</span>",
            theme.dim,
            format_money(usage.mtd_spend),
            usage.requests
        )));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>     input {} · output {} · total {}</span>",
            theme.dim, usage.input_tokens, usage.output_tokens, usage.total_tokens
        )));
    }
    append_warning(&mut lines, outcome, theme);
    lines.push(TooltipLine::Body("".into()));
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{}'>  󰅐  Updated {}</span>",
        theme.dim,
        updated_at_hm(now, outcome.cache_age)
    )));
    render_bordered(&lines, theme)
}

fn append_warning(lines: &mut Vec<TooltipLine>, outcome: &VendorOutcome, theme: &Theme) {
    let Some((code, message)) = outcome.last_error.as_ref() else {
        return;
    };
    let (label, icon, color) = match warning_kind(*code, message) {
        WarningKind::SchemaDrift => (
            "Requesty API schema drift".to_string(),
            "󰅚",
            theme.red.as_str(),
        ),
        WarningKind::Other => ("Requesty warning".to_string(), "󰀪", theme.orange.as_str()),
        WarningKind::Http(code) if code >= 500 => (format!("HTTP {code}"), "󰅚", theme.red.as_str()),
        WarningKind::Http(code) => (format!("HTTP {code}"), "󰀪", theme.orange.as_str()),
    };
    lines.push(TooltipLine::Body("".into()));
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{color}'>  {icon}  {label}</span>"
    )));
    if message != &label {
        lines.push(TooltipLine::Body(format!(
            "     <span foreground='{}'>{}</span>",
            theme.dim,
            escape(message)
        )));
    }
}

impl From<FetchOutcome> for VendorOutcome {
    fn from(outcome: FetchOutcome) -> Self {
        Self {
            snapshot: crate::usage::VendorSnapshot::Requesty(outcome.snapshot),
            stale: outcome.stale,
            last_error: outcome.last_error,
            cache_age: outcome.cache_age,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{RequestyUsage, VendorSnapshot};

    fn snapshot() -> RequestySnapshot {
        RequestySnapshot {
            org_name: "Acme Corp".into(),
            balance: 42.5,
            usage: Some(RequestyUsage {
                mtd_spend: 3.75,
                requests: 17,
                input_tokens: 2100,
                output_tokens: 1400,
                total_tokens: 3500,
            }),
            interval_start: "2026-08-01T00:00:00Z".parse().unwrap(),
            interval_end: "2026-08-24T12:34:56Z".parse().unwrap(),
            scope_fingerprint: String::new(),
        }
    }

    fn outcome(snapshot: RequestySnapshot) -> VendorOutcome {
        VendorOutcome {
            snapshot: VendorSnapshot::Requesty(snapshot),
            stale: false,
            last_error: None,
            cache_age: None,
        }
    }

    fn opts() -> RenderOpts {
        RenderOpts {
            format: None,
            tooltip_format: None,
            icon: None,
            pace_tolerance: 5,
            format_pace_color: false,
            tooltip_pace_pts: false,
        }
    }

    #[test]
    fn placeholders_expose_org_balance_and_usage_without_a_percentage() {
        let values = build_placeholders(&snapshot());
        assert_eq!(values["vendor_short"], "rqy");
        assert_eq!(values["plan"], "Acme Corp");
        assert_eq!(values["rqy_balance"], "$42.50");
        assert_eq!(values["rqy_mtd"], "$3.75");
        assert_eq!(values["rqy_tokens"], "3500");
        assert!(!headline(&snapshot()).contains('%'));
    }

    #[test]
    fn unavailable_usage_stays_absent_in_placeholders_and_tooltip() {
        let mut snapshot = snapshot();
        snapshot.usage = None;
        let values = build_placeholders(&snapshot);
        assert_eq!(values["rqy_mtd"], "—");
        let output = render(
            &VendorOutcome {
                last_error: Some((403, "Requesty authentication failed".into())),
                ..outcome(snapshot.clone())
            },
            &snapshot,
            &Theme::default(),
            &opts(),
            Utc::now(),
        );
        assert!(output.tooltip.contains("Balance"));
        assert!(output.tooltip.contains("HTTP 403"));
        assert!(!output.tooltip.contains("Month to date"));
    }

    #[test]
    fn low_balance_sets_balance_severity() {
        let mut snapshot = snapshot();
        snapshot.balance = 0.5;
        assert_eq!(severity(&snapshot), PaceSeverity::Critical);
    }
}
