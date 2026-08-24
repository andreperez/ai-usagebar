//! Firecrawl renderer — plan credits, remaining credits, and billing period.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::countdown;
use crate::format::{placeholders, substitute, updated_at_hm};
use crate::pacing::PaceSeverity;
use crate::pango::{color_span, escape, severity_color, severity_for};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::FirecrawlSnapshot;
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

pub const DEFAULT_FORMAT: &str = "{fcw_headline}";

pub fn headline(snap: &FirecrawlSnapshot) -> String {
    match snap.period_pct() {
        Some(pct) => format!("{pct}%"),
        None => format!("{} remaining", snap.remaining_credits),
    }
}

pub fn build_placeholders(
    snap: &FirecrawlSnapshot,
    now: DateTime<Utc>,
) -> HashMap<&'static str, String> {
    let pct = snap.period_pct();
    let pct_text = pct
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".into());
    let reset = countdown::format(snap.billing_period_end, now);
    placeholders(vec![
        ("icon", "󰚩".to_string()),
        ("vendor_short", "fcw".to_string()),
        ("plan", "Firecrawl".to_string()),
        ("session_pct", pct_text.clone()),
        ("session_reset", reset.clone()),
        ("weekly_pct", pct_text),
        ("weekly_reset", reset.clone()),
        ("fcw_headline", headline(snap)),
        ("fcw_remaining", snap.remaining_credits.to_string()),
        ("fcw_plan", snap.plan_credits.to_string()),
        (
            "fcw_used",
            snap.period_consumed
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "fcw_pct",
            pct.map(|value| value.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        ("fcw_reset", reset),
    ])
}

pub fn severity(snap: &FirecrawlSnapshot) -> PaceSeverity {
    severity_for(snap.period_pct().unwrap_or(0))
}

pub fn render(
    outcome: &VendorOutcome,
    snap: &FirecrawlSnapshot,
    theme: &Theme,
    opts: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let class = Class::from(severity(snap));
    let format = opts
        .format
        .clone()
        .unwrap_or_else(|| DEFAULT_FORMAT.to_string());
    let values = build_placeholders(snap, now);
    let mut pango_values = values.clone();
    if let Some(value) = pango_values.get_mut("plan") {
        *value = escape(value);
    }
    let mut text = substitute(&format, &pango_values);
    if outcome.stale {
        text.push_str(" ⏸");
    }
    let wrapper_color = severity_color(severity(snap), theme).to_string();
    let icon_prefix = match opts.icon.as_deref() {
        Some(icon) if !icon.is_empty() => format!("{icon} "),
        _ => String::new(),
    };
    let bar_text = color_span(&wrapper_color, &format!("{icon_prefix}{text}"));
    let tooltip = if let Some(format) = opts.tooltip_format.as_deref() {
        substitute(format, &pango_values)
    } else {
        render_tooltip(outcome, snap, theme, now)
    };
    WaybarOutput {
        text: bar_text,
        tooltip,
        class,
    }
}

fn render_tooltip(
    outcome: &VendorOutcome,
    snap: &FirecrawlSnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let blue = &theme.blue;
    let dim = &theme.dim;
    let fg = &theme.fg;
    let color = severity_color(severity(snap), theme);
    let mut lines = vec![TooltipLine::Center(format!(
        "<span font_weight='bold' foreground='{blue}'>Firecrawl</span>"
    ))];
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body("".into()));
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{fg}'>  󰇀  Plan credits</span>"
    )));
    if let Some(pct) = snap.period_pct() {
        lines.push(TooltipLine::Body(format!(
            "   <span font_weight='bold' foreground='{color}'>{pct}%</span>  ({used} / {plan})",
            used = snap.period_consumed.unwrap_or_default(),
            plan = snap.plan_credits
        )));
    } else {
        lines.push(TooltipLine::Body(format!(
            "   <span font_weight='bold' foreground='{color}'>{remaining}</span> remaining",
            remaining = snap.remaining_credits
        )));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{dim}'>     period usage unavailable</span>"
        )));
    }
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{dim}'>     {remaining} remaining · plan {plan}</span>",
        remaining = snap.remaining_credits,
        plan = snap.plan_credits
    )));
    if let Some(end) = snap.billing_period_end {
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{dim}'>     reset {}</span>",
            countdown::format(Some(end), now)
        )));
    }
    if let Some((code, message)) = outcome.last_error.as_ref() {
        let (label, icon, error_color) = match warning_kind(*code, message) {
            WarningKind::SchemaDrift => (
                "Firecrawl API schema drift".to_string(),
                "󰅚",
                theme.red.as_str(),
            ),
            WarningKind::Other => ("Firecrawl warning".to_string(), "󰀪", theme.orange.as_str()),
            WarningKind::Http(code) if code >= 500 => {
                (format!("HTTP {code}"), "󰅚", theme.red.as_str())
            }
            WarningKind::Http(code) => (format!("HTTP {code}"), "󰀪", theme.orange.as_str()),
        };
        lines.push(TooltipLine::Body("".into()));
        lines.push(TooltipLine::Sep);
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{error_color}'>  {icon}  {label}</span>"
        )));
        if message != &label {
            lines.push(TooltipLine::Body(format!(
                "     <span foreground='{dim}'>{}</span>",
                escape(message)
            )));
        }
    }
    lines.push(TooltipLine::Body("".into()));
    lines.push(TooltipLine::Sep);
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{dim}'>  󰅐  Updated {}</span>",
        updated_at_hm(now, outcome.cache_age)
    )));
    render_bordered(&lines, theme)
}

impl From<FetchOutcome> for VendorOutcome {
    fn from(outcome: FetchOutcome) -> Self {
        Self {
            snapshot: crate::usage::VendorSnapshot::Firecrawl(outcome.snapshot),
            stale: outcome.stale,
            last_error: outcome.last_error,
            cache_age: outcome.cache_age,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> FirecrawlSnapshot {
        FirecrawlSnapshot {
            remaining_credits: 8200,
            plan_credits: 10000,
            billing_period_start: Some("2026-08-01T00:00:00Z".parse().unwrap()),
            billing_period_end: Some("2026-09-01T00:00:00Z".parse().unwrap()),
            period_consumed: Some(1800),
            scope_fingerprint: String::new(),
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
    fn placeholders_preserve_overage_and_reset() {
        let mut snap = snapshot();
        snap.period_consumed = Some(12000);
        let values = build_placeholders(&snap, Utc::now());
        assert_eq!(values["vendor_short"], "fcw");
        assert_eq!(values["fcw_pct"], "120");
        assert_ne!(values["fcw_reset"], "—");
    }

    #[test]
    fn no_period_usage_has_no_fake_percentage() {
        let mut snap = snapshot();
        snap.period_consumed = None;
        assert_eq!(headline(&snap), "8200 remaining");
        assert_eq!(severity(&snap), PaceSeverity::Low);
        let out = render(
            &VendorOutcome {
                snapshot: crate::usage::VendorSnapshot::Firecrawl(snap.clone()),
                stale: false,
                last_error: None,
                cache_age: None,
            },
            &snap,
            &Theme::default(),
            &opts(),
            Utc::now(),
        );
        assert!(out.text.contains("8200 remaining"));
        assert!(!out.text.contains('%'));
    }

    #[test]
    fn render_shows_period_metric_and_reset() {
        let snap = snapshot();
        let out = render(
            &VendorOutcome {
                snapshot: crate::usage::VendorSnapshot::Firecrawl(snap.clone()),
                stale: false,
                last_error: None,
                cache_age: None,
            },
            &snap,
            &Theme::default(),
            &opts(),
            Utc::now(),
        );
        assert!(out.text.contains("18%"));
        assert!(out.tooltip.contains("Plan credits"));
        assert!(out.tooltip.contains("8200 remaining"));
    }
}
