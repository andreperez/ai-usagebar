//! ZenMux renderer — PAYG balance and optional subscription quota windows.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::countdown;
use crate::format::{placeholders, substitute, updated_at_hm};
use crate::pacing::{self, PaceSeverity};
use crate::pango::{color_span, escape, severity_color, severity_for};
use crate::theme::Theme;
use crate::tooltip::{Line as TooltipLine, render_bordered};
use crate::usage::{ZenMuxQuota, ZenMuxSnapshot, ZenMuxStatus};
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

pub const DEFAULT_FORMAT: &str = "{zmx_headline}";

pub fn headline(snapshot: &ZenMuxSnapshot) -> String {
    snapshot.subscription.as_ref().map_or_else(
        || {
            snapshot
                .payg
                .as_ref()
                .map(|payg| crate::format::usd(payg.total_credits))
                .unwrap_or_else(|| "—".into())
        },
        |subscription| {
            format!(
                "{}% / {}%",
                subscription.five_hour.window.utilization_pct,
                subscription.seven_day.window.utilization_pct
            )
        },
    )
}

pub fn build_placeholders(
    snapshot: &ZenMuxSnapshot,
    now: DateTime<Utc>,
) -> HashMap<&'static str, String> {
    build_placeholders_with_tolerance(snapshot, pacing::DEFAULT_TOLERANCE, now)
}

fn build_placeholders_with_tolerance(
    snapshot: &ZenMuxSnapshot,
    pace_tolerance: u32,
    now: DateTime<Utc>,
) -> HashMap<&'static str, String> {
    let subscription = snapshot.subscription.as_ref();
    let five_hour = subscription.map(|subscription| &subscription.five_hour);
    let seven_day = subscription.map(|subscription| &subscription.seven_day);
    let five_pacing = quota_pacing(five_hour, pace_tolerance, now);
    let seven_pacing = quota_pacing(seven_day, pace_tolerance, now);
    let payg = snapshot.payg.as_ref();
    placeholders(vec![
        ("icon", "󰚩".to_string()),
        ("vendor_short", "zmx".to_string()),
        (
            "plan",
            subscription
                .map(|subscription| subscription.tier.clone())
                .unwrap_or_else(|| "ZenMux PAYG".into()),
        ),
        (
            "session_pct",
            five_hour
                .map(|quota| quota.window.utilization_pct.to_string())
                .unwrap_or_default(),
        ),
        (
            "session_reset",
            five_hour
                .map(|quota| countdown::format(quota.window.resets_at, now))
                .unwrap_or_default(),
        ),
        (
            "weekly_pct",
            seven_day
                .map(|quota| quota.window.utilization_pct.to_string())
                .unwrap_or_default(),
        ),
        (
            "weekly_reset",
            seven_day
                .map(|quota| countdown::format(quota.window.resets_at, now))
                .unwrap_or_default(),
        ),
        ("session_elapsed", five_pacing.elapsed.clone()),
        ("weekly_elapsed", seven_pacing.elapsed.clone()),
        ("zmx_headline", headline(snapshot)),
        (
            "zmx_payg",
            payg.map(|payg| crate::format::usd(payg.total_credits))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_topup",
            payg.map(|payg| crate::format::usd(payg.top_up_credits))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_bonus",
            payg.map(|payg| crate::format::usd(payg.bonus_credits))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_tier",
            subscription
                .map(|subscription| subscription.tier.clone())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_status",
            subscription
                .map(|subscription| subscription.status.as_str().to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_five",
            five_hour
                .map(|quota| format!("{}%", quota.window.utilization_pct))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_five_reset",
            five_hour
                .map(|quota| countdown::format(quota.window.resets_at, now))
                .unwrap_or_else(|| "—".into()),
        ),
        ("zmx_five_elapsed", five_pacing.elapsed),
        ("zmx_five_pace", five_pacing.ratio_pace),
        ("zmx_five_pace_indicator", five_pacing.point_pace),
        (
            "zmx_seven",
            seven_day
                .map(|quota| format!("{}%", quota.window.utilization_pct))
                .unwrap_or_else(|| "—".into()),
        ),
        (
            "zmx_seven_reset",
            seven_day
                .map(|quota| countdown::format(quota.window.resets_at, now))
                .unwrap_or_else(|| "—".into()),
        ),
        ("zmx_seven_elapsed", seven_pacing.elapsed),
        ("zmx_seven_pace", seven_pacing.ratio_pace),
        ("zmx_seven_pace_indicator", seven_pacing.point_pace),
        (
            "zmx_expiry",
            subscription
                .map(|subscription| countdown::format(Some(subscription.expires_at), now))
                .unwrap_or_else(|| "—".into()),
        ),
    ])
}

#[derive(Default)]
struct QuotaPacing {
    elapsed: String,
    ratio_pace: String,
    point_pace: String,
}

fn quota_pacing(
    quota: Option<&ZenMuxQuota>,
    pace_tolerance: u32,
    now: DateTime<Utc>,
) -> QuotaPacing {
    let Some(quota) = quota else {
        return QuotaPacing::default();
    };
    let pacing = pacing::calc(
        quota.window.utilization_pct,
        quota.window.resets_at,
        now,
        quota.window.window_duration,
        pace_tolerance,
    );
    QuotaPacing {
        elapsed: pacing.elapsed_pct.to_string(),
        ratio_pace: pacing.ratio_pace.glyph().to_string(),
        point_pace: pacing.point_pace.glyph().to_string(),
    }
}

pub fn severity(snapshot: &ZenMuxSnapshot) -> PaceSeverity {
    let Some(subscription) = &snapshot.subscription else {
        return snapshot
            .payg
            .as_ref()
            .map(|payg| balance_severity(payg.total_credits))
            .unwrap_or(PaceSeverity::Critical);
    };
    let quota_severity = severity_for(
        subscription
            .five_hour
            .window
            .utilization_pct
            .max(subscription.seven_day.window.utilization_pct),
    );
    worse(status_severity(&subscription.status), quota_severity)
}

fn balance_severity(balance: f64) -> PaceSeverity {
    if balance < 1.0 {
        PaceSeverity::Critical
    } else if balance < 5.0 {
        PaceSeverity::High
    } else if balance < 20.0 {
        PaceSeverity::Mid
    } else {
        PaceSeverity::Low
    }
}

fn status_severity(status: &ZenMuxStatus) -> PaceSeverity {
    match status {
        ZenMuxStatus::Healthy => PaceSeverity::Low,
        ZenMuxStatus::Monitored | ZenMuxStatus::Other(_) => PaceSeverity::Mid,
        ZenMuxStatus::Abusive => PaceSeverity::High,
        ZenMuxStatus::Suspended | ZenMuxStatus::Banned => PaceSeverity::Critical,
    }
}

fn worse(left: PaceSeverity, right: PaceSeverity) -> PaceSeverity {
    if severity_rank(left) >= severity_rank(right) {
        left
    } else {
        right
    }
}

fn severity_rank(severity: PaceSeverity) -> u8 {
    match severity {
        PaceSeverity::Low => 0,
        PaceSeverity::Mid => 1,
        PaceSeverity::High => 2,
        PaceSeverity::Critical => 3,
    }
}

pub fn render(
    outcome: &VendorOutcome,
    snapshot: &ZenMuxSnapshot,
    theme: &Theme,
    opts: &RenderOpts,
    now: DateTime<Utc>,
) -> WaybarOutput {
    let class = Class::from(severity(snapshot));
    let format = opts
        .format
        .clone()
        .unwrap_or_else(|| DEFAULT_FORMAT.to_string());
    let values = build_placeholders_with_tolerance(snapshot, opts.pace_tolerance, now);
    let mut pango_values = values.clone();
    for key in ["plan", "zmx_tier", "zmx_status"] {
        if let Some(value) = pango_values.get_mut(key) {
            *value = escape(value);
        }
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

fn format_flows(flows: f64) -> String {
    if flows.fract() == 0.0 {
        format!("{flows:.0}")
    } else {
        format!("{flows:.2}")
    }
}

fn render_tooltip(
    outcome: &VendorOutcome,
    snapshot: &ZenMuxSnapshot,
    theme: &Theme,
    now: DateTime<Utc>,
) -> String {
    let title = snapshot.subscription.as_ref().map_or_else(
        || "ZenMux PAYG".to_string(),
        |subscription| format!("ZenMux — {}", subscription.tier),
    );
    let mut lines = vec![TooltipLine::Center(format!(
        "<span font_weight='bold' foreground='{}'>{}</span>",
        theme.blue,
        escape(&title)
    ))];
    lines.push(TooltipLine::Sep);
    if let Some(payg) = &snapshot.payg {
        lines.push(TooltipLine::Body("".into()));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>  󰢗  PAYG balance</span>",
            theme.fg
        )));
        lines.push(TooltipLine::Body(format!(
            "   <span font_weight='bold' foreground='{}'>{}</span>",
            severity_color(balance_severity(payg.total_credits), theme),
            crate::format::usd(payg.total_credits)
        )));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>     top-up {} · bonus {}</span>",
            theme.dim,
            crate::format::usd(payg.top_up_credits),
            crate::format::usd(payg.bonus_credits)
        )));
    }
    if let Some(subscription) = &snapshot.subscription {
        lines.push(TooltipLine::Body("".into()));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>  󰔟  Status {}</span>",
            theme.fg,
            escape(subscription.status.as_str())
        )));
        push_quota(&mut lines, "5h quota", &subscription.five_hour, theme, now);
        push_quota(&mut lines, "7d quota", &subscription.seven_day, theme, now);
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>     monthly max {} flows · {}</span>",
            theme.dim,
            format_flows(subscription.monthly_max_flows),
            crate::format::usd(subscription.monthly_max_value_usd)
        )));
        lines.push(TooltipLine::Body(format!(
            " <span foreground='{}'>     expires {}</span>",
            theme.dim,
            countdown::format(Some(subscription.expires_at), now)
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

fn push_quota(
    lines: &mut Vec<TooltipLine>,
    label: &str,
    quota: &ZenMuxQuota,
    theme: &Theme,
    now: DateTime<Utc>,
) {
    let color = severity_color(severity_for(quota.window.utilization_pct), theme);
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{}'>     {label}</span>  <span font_weight='bold' foreground='{color}'>{}%</span>",
        theme.dim, quota.window.utilization_pct
    )));
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{}'>       {} / {} flows · {} / {}</span>",
        theme.dim,
        format_flows(quota.used_flows),
        format_flows(quota.max_flows),
        crate::format::usd(quota.used_value_usd),
        crate::format::usd(quota.max_value_usd)
    )));
    lines.push(TooltipLine::Body(format!(
        " <span foreground='{}'>       reset {}</span>",
        theme.dim,
        countdown::format(quota.window.resets_at, now)
    )));
}

fn append_warning(lines: &mut Vec<TooltipLine>, outcome: &VendorOutcome, theme: &Theme) {
    let Some((code, message)) = outcome.last_error.as_ref() else {
        return;
    };
    let (label, icon, color) = match warning_kind(*code, message) {
        WarningKind::SchemaDrift => (
            "ZenMux API schema drift".to_string(),
            "󰅚",
            theme.red.as_str(),
        ),
        WarningKind::Http(401 | 403) => (
            "Management key required".to_string(),
            "󰅚",
            theme.red.as_str(),
        ),
        WarningKind::Http(422) => ("Rate limit".to_string(), "󰀪", theme.orange.as_str()),
        WarningKind::Http(code) if code >= 500 => (format!("HTTP {code}"), "󰅚", theme.red.as_str()),
        WarningKind::Http(code) => (format!("HTTP {code}"), "󰀪", theme.orange.as_str()),
        WarningKind::Other => ("ZenMux warning".to_string(), "󰀪", theme.orange.as_str()),
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
            snapshot: crate::usage::VendorSnapshot::ZenMux(outcome.snapshot),
            stale: outcome.stale,
            last_error: outcome.last_error,
            cache_age: outcome.cache_age,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{ZenMuxPayg, ZenMuxQuota, ZenMuxSubscription};

    fn quota(pct: i32, duration: chrono::Duration) -> ZenMuxQuota {
        ZenMuxQuota {
            window: crate::usage::UsageWindow {
                utilization_pct: pct,
                resets_at: Some("2026-08-25T17:00:00Z".parse().unwrap()),
                window_duration: duration,
            },
            max_flows: 1000.0,
            used_flows: 840.0,
            remaining_flows: 160.0,
            used_value_usd: 8.4,
            max_value_usd: 10.0,
        }
    }

    fn snapshot() -> ZenMuxSnapshot {
        ZenMuxSnapshot {
            payg: Some(ZenMuxPayg {
                total_credits: 42.5,
                top_up_credits: 30.0,
                bonus_credits: 12.5,
            }),
            subscription: Some(ZenMuxSubscription {
                tier: "Pro".into(),
                plan_amount_usd: 20.0,
                expires_at: "2026-09-15T00:00:00Z".parse().unwrap(),
                status: ZenMuxStatus::Healthy,
                base_usd_per_flow: 0.01,
                effective_usd_per_flow: 0.01,
                five_hour: quota(84, chrono::Duration::hours(5)),
                seven_day: quota(42, chrono::Duration::days(7)),
                monthly_max_flows: 30_000.0,
                monthly_max_value_usd: 300.0,
            }),
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

    fn outcome(snapshot: ZenMuxSnapshot) -> VendorOutcome {
        VendorOutcome {
            snapshot: crate::usage::VendorSnapshot::ZenMux(snapshot),
            stale: false,
            last_error: None,
            cache_age: None,
        }
    }

    #[test]
    fn placeholders_expose_payg_and_subscription_windows() {
        let values = build_placeholders(&snapshot(), Utc::now());
        assert_eq!(values["vendor_short"], "zmx");
        assert_eq!(values["session_pct"], "84");
        assert_eq!(values["weekly_pct"], "42");
        assert_eq!(values["zmx_payg"], "$42.50");
        assert_eq!(values["zmx_five"], "84%");
    }

    #[test]
    fn payg_only_snapshot_leaves_generic_windows_empty() {
        let mut snapshot = snapshot();
        snapshot.subscription = None;
        let values = build_placeholders(&snapshot, Utc::now());
        assert_eq!(values["session_pct"], "");
        assert_eq!(values["weekly_pct"], "");
        assert_eq!(headline(&snapshot), "$42.50");
    }

    #[test]
    fn unknown_status_is_not_healthy() {
        let mut snapshot = snapshot();
        let subscription = snapshot.subscription.as_mut().unwrap();
        subscription.status = ZenMuxStatus::Other("review".into());
        subscription.five_hour.window.utilization_pct = 10;
        subscription.seven_day.window.utilization_pct = 10;
        assert_eq!(severity(&snapshot), PaceSeverity::Mid);
    }

    #[test]
    fn management_key_warning_is_explicit() {
        let snapshot = snapshot();
        let output = render(
            &VendorOutcome {
                last_error: Some((401, "ZenMux management key required".into())),
                ..outcome(snapshot.clone())
            },
            &snapshot,
            &Theme::default(),
            &opts(),
            Utc::now(),
        );
        assert!(output.tooltip.contains("Management key required"));
    }
}
