//! Settings overlay — opened from the TUI by pressing `s`. Lets the user pick
//! the primary vendor and paste an API key for any key-authenticated vendor
//! (including Z.AI, Kimi, MiniMax, and the balance vendors) without hand-editing
//! config.toml. Anthropic, OpenAI, Cursor, and Antigravity authenticate through
//! local product state, so they have no key field here.
//!
//! Persistence uses `toml_edit` so the existing config keeps its comments,
//! whitespace, and unrelated fields. Writing a key also flips that vendor's
//! `enabled = true` (the opt-in vendors are disabled by default), so "paste the
//! key and save" is all it takes. Files with inline keys are atomically written
//! and `chmod 600`ed.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui_bubbletea_theme::BubbleTheme;
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, value};

use crate::config::Config;
use crate::error::{AppError, Result};
use crate::theme::Theme;
use crate::tui::style::bubble_theme;
use crate::vendor::VendorId;

/// A vendor that authenticates with an inline API key (vs. OAuth). The order of
/// this table is the tab order of the key fields and the layout of the state's
/// `keys` vec.
pub struct KeyVendor {
    pub id: VendorId,
    pub label: &'static str,
    pub env: &'static str,
    pub section: &'static str,
    /// Extra hint after the env var (e.g. "management key"). Empty for none.
    pub note: &'static str,
}

pub const KEY_VENDORS: &[KeyVendor] = &[
    KeyVendor {
        id: VendorId::AnthropicApi,
        label: "Anthropic API",
        env: "ANTHROPIC_ADMIN_KEY",
        section: "anthropic_api",
        note: "admin key — monthly spend",
    },
    KeyVendor {
        id: VendorId::Zai,
        label: "Z.AI",
        env: "ZAI_API_KEY",
        section: "zai",
        note: "",
    },
    KeyVendor {
        id: VendorId::Openrouter,
        label: "OpenRouter",
        env: "OPENROUTER_API_KEY",
        section: "openrouter",
        note: "",
    },
    KeyVendor {
        id: VendorId::Deepseek,
        label: "DeepSeek",
        env: "DEEPSEEK_API_KEY",
        section: "deepseek",
        note: "",
    },
    KeyVendor {
        id: VendorId::Kimi,
        label: "Kimi",
        env: "KIMI_API_KEY",
        section: "kimi",
        note: "coding-plan usage",
    },
    KeyVendor {
        id: VendorId::Kilo,
        label: "Kilo",
        env: "KILO_API_KEY",
        section: "kilo",
        note: "",
    },
    KeyVendor {
        id: VendorId::Novita,
        label: "Novita",
        env: "NOVITA_API_KEY",
        section: "novita",
        note: "",
    },
    KeyVendor {
        id: VendorId::Moonshot,
        label: "Moonshot",
        env: "MOONSHOT_API_KEY",
        section: "moonshot",
        note: "account balance",
    },
    KeyVendor {
        id: VendorId::Grok,
        label: "Grok",
        env: "XAI_MANAGEMENT_KEY",
        section: "grok",
        note: "management key, not the inference key",
    },
    KeyVendor {
        id: VendorId::Minimax,
        label: "MiniMax",
        env: "MINIMAX_API_KEY",
        section: "minimax",
        note: "Token Plan subscription key",
    },
    KeyVendor {
        id: VendorId::OpenCodeGo,
        label: "OpenCode Go",
        env: "OPENCODE_GO_API_KEY",
        section: "opencode-go",
        note: "usage quota",
    },
    KeyVendor {
        id: VendorId::Tavily,
        label: "Tavily",
        env: "TAVILY_API_KEY",
        section: "tavily",
        note: "usage & quota",
    },
    KeyVendor {
        id: VendorId::Firecrawl,
        label: "Firecrawl",
        env: "FIRECRAWL_API_KEY",
        section: "firecrawl",
        note: "credits & billing period",
    },
    KeyVendor {
        id: VendorId::Requesty,
        label: "Requesty",
        env: "REQUESTY_API_KEY",
        section: "requesty",
        note: "org balance & usage",
    },
    KeyVendor {
        id: VendorId::ZenMux,
        label: "ZenMux",
        env: "ZENMUX_MANAGEMENT_API_KEY",
        section: "zenmux",
        note: "management key — PAYG + subscription",
    },
];

/// Read the inline `api_key` currently in config for a given section, so the
/// field opens pre-filled (masked) when one is already set.
fn config_inline_key<'a>(cfg: &'a Config, section: &str) -> Option<&'a str> {
    match section {
        "anthropic_api" => cfg.anthropic_api.api_key.as_deref(),
        "zai" => cfg.zai.api_key.as_deref(),
        "openrouter" => cfg.openrouter.api_key.as_deref(),
        "deepseek" => cfg.deepseek.api_key.as_deref(),
        "kimi" => cfg.kimi.api_key.as_deref(),
        "kilo" => cfg.kilo.api_key.as_deref(),
        "novita" => cfg.novita.api_key.as_deref(),
        "moonshot" => cfg.moonshot.api_key.as_deref(),
        "grok" => cfg.grok.api_key.as_deref(),
        "minimax" => cfg.minimax.api_key.as_deref(),
        "opencode-go" => cfg.opencode_go.api_key.as_deref(),
        "tavily" => cfg.tavily.api_key.as_deref(),
        "firecrawl" => cfg.firecrawl.api_key.as_deref(),
        "requesty" => cfg.requesty.api_key.as_deref(),
        "zenmux" => cfg.zenmux.api_key.as_deref(),
        _ => None,
    }
}

/// Which control has keyboard focus. `Active(i)` indexes active provider
/// candidates; `Key(i)` indexes into [`KEY_VENDORS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Primary,
    Active(usize),
    Key(usize),
    Save,
}

/// An interactive row recorded for mouse hit-testing during the settings draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    /// A focusable control (primary picker, a key field, or the save row).
    Focus(Focus),
    /// The collapsed "More providers" header — clicking it expands the section.
    MoreHeader,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Focus::Primary => Focus::Key(0),
            Focus::Active(_) => Focus::Key(0),
            Focus::Key(i) if i + 1 < KEY_VENDORS.len() => Focus::Key(i + 1),
            Focus::Key(_) => Focus::Save,
            Focus::Save => Focus::Primary,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Focus::Primary => Focus::Save,
            Focus::Active(_) => Focus::Primary,
            Focus::Key(0) => Focus::Primary,
            Focus::Key(i) => Focus::Key(i - 1),
            Focus::Save => Focus::Key(KEY_VENDORS.len() - 1),
        }
    }
}

/// Per-field text-input state — cursor + buffer + reveal flag.
#[derive(Debug, Clone, Default)]
pub struct KeyInput {
    pub buf: String,
    /// Char-index cursor position (0..=buf.chars().count()).
    pub cursor: usize,
    /// When true, the field renders the actual characters; otherwise `•`.
    pub revealed: bool,
    /// True after the user has typed/edited; only then does save write the
    /// value back (avoids clobbering an existing key with the empty
    /// placeholder the user opened the dialog with).
    pub dirty: bool,
}

impl KeyInput {
    pub fn from_config(initial: Option<&str>) -> Self {
        let buf = initial.unwrap_or("").to_string();
        let cursor = buf.chars().count();
        Self {
            buf,
            cursor,
            revealed: false,
            dirty: false,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_idx = self.char_to_byte(self.cursor);
        self.buf.insert(byte_idx, c);
        self.cursor += 1;
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev_byte = self.char_to_byte(self.cursor - 1);
        let cur_byte = self.char_to_byte(self.cursor);
        self.buf.replace_range(prev_byte..cur_byte, "");
        self.cursor -= 1;
        self.dirty = true;
    }

    pub fn delete(&mut self) {
        let n = self.buf.chars().count();
        if self.cursor >= n {
            return;
        }
        let cur_byte = self.char_to_byte(self.cursor);
        let next_byte = self.char_to_byte(self.cursor + 1);
        self.buf.replace_range(cur_byte..next_byte, "");
        self.dirty = true;
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }
    pub fn move_right(&mut self) {
        if self.cursor < self.buf.chars().count() {
            self.cursor += 1;
        }
    }
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }
    pub fn move_end(&mut self) {
        self.cursor = self.buf.chars().count();
    }
    pub fn toggle_reveal(&mut self) {
        self.revealed = !self.revealed;
    }

    /// Render for display — bullets when masked, raw chars when revealed.
    pub fn display(&self) -> String {
        if self.revealed {
            self.buf.clone()
        } else {
            "•".repeat(self.buf.chars().count())
        }
    }

    fn char_to_byte(&self, char_idx: usize) -> usize {
        self.buf
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(self.buf.len()))
            .nth(char_idx)
            .unwrap_or(self.buf.len())
    }
}

/// Mutable state of the overlay while open.
#[derive(Debug, Clone)]
pub struct SettingsState {
    pub focus: Focus,
    /// Active providers only. The primary selector cannot offer a provider
    /// outside the automatic fetch/display scope.
    pub primary_choices: Vec<VendorId>,
    pub primary: VendorId,
    /// Configured providers eligible for visual activation. API-key providers
    /// with an environment key remain selectable before their config section is
    /// enabled, so Settings can complete activation without manual TOML edits.
    pub active_choices: Vec<VendorId>,
    /// Explicit selected subset, stored as `[ui] active_vendors` on save.
    pub active_vendors: Vec<VendorId>,
    /// One input per [`KEY_VENDORS`] entry, same order.
    pub keys: Vec<KeyInput>,
    /// One-line status displayed in the footer ("saved …", "save failed …").
    pub status: String,
    /// Per [`KEY_VENDORS`] index: does a key resolve today (env var or inline
    /// config value)? Unconfigured rows are grouped under a collapsed
    /// "More providers" section and only revealed by navigating into them.
    pub configured: Vec<bool>,
    /// When `true`, the "More providers" section is expanded and its rows are
    /// rendered and focusable.
    pub show_more: bool,
}

impl SettingsState {
    pub fn from_config(cfg: &Config) -> Self {
        let keys = KEY_VENDORS
            .iter()
            .map(|kv| KeyInput::from_config(config_inline_key(cfg, kv.section)))
            .collect();
        let configured = KEY_VENDORS
            .iter()
            .map(|kv| {
                std::env::var(kv.env)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
                    || config_inline_key(cfg, kv.section).is_some_and(|k| !k.is_empty())
            })
            .collect();
        let active_choices: Vec<VendorId> = VendorId::all()
            .iter()
            .copied()
            .filter(|vendor| cfg.is_configured(*vendor))
            .collect();
        let selected = cfg
            .ui
            .active_vendors
            .clone()
            .unwrap_or_else(|| cfg.active_vendors());
        let active_vendors: Vec<VendorId> = active_choices
            .iter()
            .copied()
            .filter(|vendor| selected.contains(vendor))
            .collect();
        let primary_choices = active_vendors.clone();
        // A primary outside the active scope is ineffective. Display the first
        // active provider instead; when none are active retain the historical
        // Anthropic fallback in memory without inventing a persisted primary.
        let primary = cfg
            .ui
            .primary
            .filter(|vendor| primary_choices.contains(vendor))
            .or_else(|| primary_choices.first().copied())
            .unwrap_or_else(|| cfg.ui.primary.unwrap_or(VendorId::Anthropic));
        Self {
            focus: Focus::Primary,
            primary_choices,
            primary,
            active_choices,
            active_vendors,
            keys,
            status: String::new(),
            configured,
            show_more: false,
        }
    }

    /// The focused key input, if a key row is focused.
    fn focused_key_mut(&mut self) -> Option<&mut KeyInput> {
        match self.focus {
            Focus::Key(i) => self.keys.get_mut(i),
            _ => None,
        }
    }

    /// Whether an active-provider checkbox is selected.
    pub fn is_active(&self, index: usize) -> bool {
        self.active_choices
            .get(index)
            .is_some_and(|vendor| self.active_vendors.contains(vendor))
    }

    /// Toggle a configured provider in the automatic fetch/display scope.
    /// Selection order always follows `active_choices`, not click order, so
    /// every frontend receives stable canonical ordering.
    pub fn toggle_active(&mut self, index: usize) {
        let Some(vendor) = self.active_choices.get(index).copied() else {
            return;
        };
        if let Some(position) = self.active_vendors.iter().position(|id| *id == vendor) {
            self.active_vendors.remove(position);
        } else {
            self.active_vendors.push(vendor);
        }
        self.active_vendors = self
            .active_choices
            .iter()
            .copied()
            .filter(|id| self.active_vendors.contains(id))
            .collect();
        self.primary_choices = self.active_vendors.clone();
        if !self.primary_choices.contains(&self.primary) {
            self.primary = self
                .primary_choices
                .first()
                .copied()
                .unwrap_or(VendorId::Anthropic);
        }
    }

    /// KEY_VENDORS indices currently visible in the focus ring: configured rows
    /// always; the "More providers" rows only while expanded.
    fn visible_keys(&self) -> Vec<usize> {
        KEY_VENDORS
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                self.show_more || self.configured.get(*index).copied().unwrap_or(false)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Move focus forward through the visible ring. Navigating down past the
    /// last configured row expands the "More providers" section and lands on
    /// its first row; navigating up out of it collapses the section again.
    fn next_focus(&self) -> Focus {
        match self.focus {
            Focus::Primary => {
                if self.active_choices.is_empty() {
                    self.visible_keys()
                        .first()
                        .copied()
                        .map(Focus::Key)
                        .unwrap_or(Focus::Save)
                } else {
                    Focus::Active(0)
                }
            }
            Focus::Active(i) if i + 1 < self.active_choices.len() => Focus::Active(i + 1),
            Focus::Active(_) => match self.visible_keys().first() {
                Some(&i) => Focus::Key(i),
                None => Focus::Save,
            },
            Focus::Key(i) => {
                let visible = self.visible_keys();
                match visible.iter().position(|&v| v == i) {
                    Some(pos) if pos + 1 < visible.len() => Focus::Key(visible[pos + 1]),
                    Some(_) => Focus::Save,
                    None => Focus::Save,
                }
            }
            Focus::Save => self
                .active_choices
                .first()
                .map(|_| Focus::Active(0))
                .or_else(|| self.visible_keys().first().copied().map(Focus::Key))
                .unwrap_or(Focus::Primary),
        }
    }

    fn prev_focus(&self) -> Focus {
        let visible = self.visible_keys();
        match self.focus {
            Focus::Primary => Focus::Save,
            Focus::Active(0) => Focus::Primary,
            Focus::Active(i) => Focus::Active(i - 1),
            Focus::Key(i) => match visible.iter().position(|&v| v == i) {
                Some(0) => self
                    .active_choices
                    .len()
                    .checked_sub(1)
                    .map(Focus::Active)
                    .unwrap_or(Focus::Primary),
                Some(pos) => Focus::Key(visible[pos - 1]),
                None => Focus::Primary,
            },
            Focus::Save => visible
                .last()
                .copied()
                .map(Focus::Key)
                .or_else(|| self.active_choices.len().checked_sub(1).map(Focus::Active))
                .unwrap_or(Focus::Primary),
        }
    }

    /// Expand or collapse the "More providers" section. Expanding moves focus
    /// to the first unconfigured row; collapsing moves focus back to the last
    /// configured row (or Primary when there are none).
    pub fn toggle_more(&mut self) {
        self.show_more = !self.show_more;
        if self.show_more {
            if let Some(i) = self.first_unconfigured() {
                self.focus = Focus::Key(i);
            }
        } else if let Focus::Key(i) = self.focus
            && !self.configured.get(i).copied().unwrap_or(false)
        {
            self.focus = self
                .last_configured()
                .map(Focus::Key)
                .unwrap_or(Focus::Primary);
        }
    }

    /// Index of the first KEY_VENDORS row with no resolvable credential.
    fn first_unconfigured(&self) -> Option<usize> {
        KEY_VENDORS
            .iter()
            .enumerate()
            .find(|(index, _)| !self.configured.get(*index).copied().unwrap_or(false))
            .map(|(index, _)| index)
    }

    /// Index of the last KEY_VENDORS row with a resolvable credential.
    fn last_configured(&self) -> Option<usize> {
        KEY_VENDORS
            .iter()
            .enumerate()
            .rev()
            .find(|(index, _)| self.configured.get(*index).copied().unwrap_or(false))
            .map(|(index, _)| index)
    }
}

/// What the key handler asks the host app to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Stay open, keep listening for keys.
    Continue,
    /// Close the overlay (discard or save already happened).
    Close,
    /// Save just succeeded — caller should refresh affected vendors.
    SavedAndClose,
    /// Quit the host TUI. Ctrl-C remains global even while the overlay owns
    /// keyboard focus.
    Quit,
}

/// Permission note appended to the "saved" status line. The overlay `chmod
/// 600`s the file on Unix; Windows has no such step, so the note is empty there.
#[cfg(unix)]
const PERMS_NOTE: &str = " (chmod 600)";
#[cfg(not(unix))]
const PERMS_NOTE: &str = "";

fn saved_status() -> String {
    format!(
        "saved to {}{}",
        crate::config::config_path_hint(),
        PERMS_NOTE
    )
}

/// Key map. Returns the action to perform after the keypress.
pub fn handle_key(state: &mut SettingsState, code: KeyCode, mods: KeyModifiers) -> Action {
    if matches!(code, KeyCode::Esc) {
        return Action::Close;
    }
    if matches!(code, KeyCode::Char('c')) && mods.contains(KeyModifiers::CONTROL) {
        return Action::Quit;
    }
    // Ctrl-S triggers save from any field.
    if matches!(code, KeyCode::Char('s')) && mods.contains(KeyModifiers::CONTROL) {
        return try_save(state);
    }
    if matches!(code, KeyCode::Char('v')) && mods.contains(KeyModifiers::CONTROL) {
        if let Some(input) = state.focused_key_mut() {
            input.toggle_reveal();
        }
        return Action::Continue;
    }
    match code {
        KeyCode::Tab | KeyCode::Down => {
            // Moving down past the last configured row reveals the collapsed
            // "More providers" section; moving back up past its first row
            // collapses it again, keeping the ring compact by default.
            if matches!(state.focus, Focus::Key(i) if Some(i) == state.last_configured())
                && !state.show_more
                && state.first_unconfigured().is_some()
            {
                state.show_more = true;
                state.focus = Focus::Key(state.first_unconfigured().unwrap());
                return Action::Continue;
            }
            state.focus = state.next_focus();
            return Action::Continue;
        }
        KeyCode::BackTab | KeyCode::Up => {
            if matches!(state.focus, Focus::Key(i) if Some(i) == state.first_unconfigured())
                && state.show_more
            {
                state.show_more = false;
                state.focus = state
                    .last_configured()
                    .map(Focus::Key)
                    .unwrap_or(Focus::Primary);
                return Action::Continue;
            }
            state.focus = state.prev_focus();
            return Action::Continue;
        }
        _ => {}
    }

    // A modifier chord is not text. The overlay swallows every key while open,
    // so every unhandled chord must be ignored rather than corrupting the
    // secret silently. SHIFT is deliberately not rejected — it is how
    // uppercase arrives. Ctrl-C was handled above because it is a global quit.
    if matches!(code, KeyCode::Char(_))
        && mods.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::HYPER
                | KeyModifiers::META,
        )
    {
        return Action::Continue;
    }

    // Field-specific handling.
    match state.focus {
        Focus::Primary => handle_primary(state, code),
        Focus::Active(i) => {
            if matches!(code, KeyCode::Char(' ') | KeyCode::Enter) {
                state.toggle_active(i);
            }
        }
        Focus::Key(i) => {
            if let Some(input) = state.keys.get_mut(i) {
                handle_input(input, code);
            }
        }
        Focus::Save => {
            if matches!(code, KeyCode::Enter) {
                return try_save(state);
            }
        }
    }
    Action::Continue
}

fn try_save(state: &mut SettingsState) -> Action {
    match save_to_config_default(state) {
        Ok(()) => {
            state.status = saved_status();
            Action::SavedAndClose
        }
        Err(e) => {
            state.status = format!("save failed: {e}");
            Action::Continue
        }
    }
}

fn handle_primary(state: &mut SettingsState, code: KeyCode) {
    // Left/Right cycles the primary-vendor radio over active providers only.
    let choices = &state.primary_choices;
    let Some(idx) = choices.iter().position(|v| *v == state.primary) else {
        return;
    };
    let step = match code {
        KeyCode::Left => -1,
        KeyCode::Right | KeyCode::Char(' ') => 1,
        _ => return,
    };
    state.primary = choices[((idx as i32 + step).rem_euclid(choices.len() as i32)) as usize];
}

fn handle_input(input: &mut KeyInput, code: KeyCode) {
    match code {
        KeyCode::Char(c) => input.insert_char(c),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Left => input.move_left(),
        KeyCode::Right => input.move_right(),
        KeyCode::Home => input.move_home(),
        KeyCode::End => input.move_end(),
        _ => {}
    }
}

/// Save to the platform config path (creating it). On success, signal a running
/// Waybar (`SIGRTMIN+13`) so a `signal: 13` module refreshes immediately.
fn save_to_config_default(state: &SettingsState) -> Result<()> {
    let path = default_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io_at(parent, e))?;
    }
    save_to_path(state, &path)?;
    crate::waybar::request_refresh();
    Ok(())
}

/// Same as `save_to_config_default` but with an explicit path — exposed for
/// tests. Writing a non-empty key also sets that vendor's `enabled = true`.
pub fn save_to_path(state: &SettingsState, path: &Path) -> Result<()> {
    let original = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(AppError::io_at(path, error)),
    };
    let mut doc: DocumentMut = if original.trim().is_empty() {
        DocumentMut::new()
    } else {
        original.parse().map_err(|e: toml_edit::TomlError| {
            AppError::Other(format!("config.toml not parseable: {e}"))
        })?
    };
    materialize_missing_default_sections(&mut doc)?;

    let mut active_vendors = state.active_vendors.clone();

    for (i, kv) in KEY_VENDORS.iter().enumerate() {
        let Some(input) = state.keys.get(i) else {
            continue;
        };
        update_key(&mut doc, kv.section, input)?;
        // Pasting a key is an explicit selection signal: keep the provider in
        // the active fetch/display scope. Clearing a key removes it because it
        // cannot be automatically fetched any longer.
        if input.dirty {
            if input.buf.is_empty() {
                active_vendors.retain(|id| *id != kv.id);
            } else if !active_vendors.contains(&kv.id) {
                active_vendors.push(kv.id);
            }
        }
    }

    // Persist canonical ordering rather than click order, so all frontends
    // receive stable provider ordering.
    active_vendors = VendorId::all()
        .iter()
        .copied()
        .filter(|id| active_vendors.contains(id))
        .collect();
    for vendor in &active_vendors {
        set_bool(&mut doc, vendor_config_section(*vendor), "enabled", true)?;
    }
    set_vendor_list(&mut doc, "ui", "active_vendors", &active_vendors)?;
    if active_vendors.contains(&state.primary) {
        set_string(&mut doc, "ui", "primary", state.primary.slug())?;
    } else {
        remove_field(&mut doc, "ui", "primary")?;
    }

    let bytes = doc.to_string();
    crate::cache::atomic_write(path, bytes.as_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    Ok(())
}

/// Add every currently known default section once Settings owns a config save.
/// Existing tables and their comments remain untouched; only absent sections
/// are copied from the serialized defaults. This keeps newly added providers
/// discoverable in the user's config without overwriting choices or keys.
fn materialize_missing_default_sections(doc: &mut DocumentMut) -> Result<()> {
    let defaults = toml::to_string(&Config::default()).map_err(|error| {
        AppError::Other(format!("default config could not be serialized: {error}"))
    })?;
    let defaults = defaults.parse::<DocumentMut>().map_err(|error| {
        AppError::Other(format!("default config could not be materialized: {error}"))
    })?;
    for (key, item) in defaults.as_table().iter() {
        if !doc.as_table().contains_key(key) {
            doc.as_table_mut().insert(key, item.clone());
        }
    }
    Ok(())
}

fn vendor_config_section(vendor: VendorId) -> &'static str {
    match vendor {
        VendorId::Anthropic => "anthropic",
        VendorId::AnthropicApi => "anthropic_api",
        VendorId::Openai => "openai",
        VendorId::Zai => "zai",
        VendorId::Openrouter => "openrouter",
        VendorId::Deepseek => "deepseek",
        VendorId::Kimi => "kimi",
        VendorId::Kilo => "kilo",
        VendorId::Novita => "novita",
        VendorId::Moonshot => "moonshot",
        VendorId::Grok => "grok",
        VendorId::Supergrok => "supergrok",
        VendorId::Antigravity => "antigravity",
        VendorId::Cursor => "cursor",
        VendorId::Minimax => "minimax",
        VendorId::Kiro => "kiro",
        VendorId::NousResearch => "nous",
        VendorId::OpenCodeGo => "opencode-go",
        VendorId::Tavily => "tavily",
        VendorId::Firecrawl => "firecrawl",
        VendorId::Requesty => "requesty",
        VendorId::ZenMux => "zenmux",
        VendorId::VercelGateway => "vercel-ai-gateway",
    }
}

/// Apply one key field to the document. Untouched fields are left alone; a
/// field the user cleared is *removed*, so an inline secret can be deleted
/// from the overlay rather than lingering in the file. Writing a non-empty key
/// also opts the vendor in — the opt-in vendors would otherwise never fetch.
fn update_key(doc: &mut DocumentMut, section: &str, input: &KeyInput) -> Result<()> {
    if !input.dirty {
        return Ok(());
    }
    if input.buf.is_empty() {
        if let Some(table) = doc.get_mut(section).and_then(toml_edit::Item::as_table_mut) {
            table.remove("api_key");
        }
        return Ok(());
    }
    set_string(doc, section, "api_key", &input.buf)?;
    set_bool(doc, section, "enabled", true)
}

/// Set or update a string field in a TOML section, preserving comments and
/// formatting of unaffected nodes.
fn set_string(doc: &mut DocumentMut, section: &str, key: &str, new_value: &str) -> Result<()> {
    let table = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .ok_or_else(|| AppError::Other(format!("config.toml: [{section}] is not a table")))?;

    if let Some(item) = table.get_mut(key)
        && let Some(v) = item.as_value_mut()
    {
        *v = toml_edit::Value::from(new_value);
        v.decor_mut().set_prefix(" ");
        return Ok(());
    }
    table.insert(key, value(new_value));
    Ok(())
}

/// Same as [`set_string`] for a boolean field.
fn set_bool(doc: &mut DocumentMut, section: &str, key: &str, new_value: bool) -> Result<()> {
    let table = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .ok_or_else(|| AppError::Other(format!("config.toml: [{section}] is not a table")))?;

    if let Some(item) = table.get_mut(key)
        && let Some(v) = item.as_value_mut()
    {
        *v = toml_edit::Value::from(new_value);
        v.decor_mut().set_prefix(" ");
        return Ok(());
    }
    table.insert(key, value(new_value));
    Ok(())
}

/// Persist a stable vendor-id list in a TOML array.
fn set_vendor_list(
    doc: &mut DocumentMut,
    section: &str,
    key: &str,
    vendors: &[VendorId],
) -> Result<()> {
    let table = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .ok_or_else(|| AppError::Other(format!("config.toml: [{section}] is not a table")))?;
    let mut values = toml_edit::Array::new();
    for vendor in vendors {
        values.push(vendor.slug());
    }
    table.insert(key, value(values));
    Ok(())
}

/// Remove one field when its state would otherwise reference a provider outside
/// the active scope. This intentionally preserves the surrounding table and
/// its comments.
fn remove_field(doc: &mut DocumentMut, section: &str, key: &str) -> Result<()> {
    let Some(table) = doc.get_mut(section) else {
        return Ok(());
    };
    let table = table
        .as_table_mut()
        .ok_or_else(|| AppError::Other(format!("config.toml: [{section}] is not a table")))?;
    table.remove(key);
    Ok(())
}

fn default_config_path() -> Result<PathBuf> {
    // Save back to the same file Config::load() selected. On macOS this may be
    // the legacy ~/.config path when the canonical Application Support file is
    // absent; writing a new canonical file would shadow the existing config on
    // the next load and silently discard all settings the overlay did not copy.
    crate::config::resolved_path()
        .ok_or_else(|| AppError::Other("could not resolve config dir".into()))
}

// ─── Native frontend bridge ───────────────────────────────────────────────

/// Versioned, non-secret description consumed by native desktop frontends.
/// Inline key values are deliberately represented only as booleans: a
/// long-lived shell process never needs to receive credentials just to draw a
/// settings form.
#[derive(Debug, Serialize)]
struct SettingsSnapshot {
    schema_version: u8,
    primary: String,
    primary_choices: Vec<PrimaryChoice>,
    active_vendors: Vec<String>,
    active_choices: Vec<PrimaryChoice>,
    keys: Vec<KeyStatus>,
}

#[derive(Debug, Serialize)]
struct PrimaryChoice {
    id: String,
    label: String,
}

#[derive(Debug, Serialize)]
struct KeyStatus {
    id: String,
    label: String,
    environment: String,
    note: String,
    configured: bool,
    inline_configured: bool,
    environment_configured: bool,
}

/// Additive patch accepted on stdin by `ai-usagebar settings apply`.
/// Missing keys remain byte-for-byte untouched. `clear` explicitly removes an
/// inline key, matching the TUI overlay's existing empty-dirty-field behavior.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    schema_version: u8,
    primary: Option<String>,
    #[serde(default)]
    active_vendors: Option<Vec<String>>,
    #[serde(default)]
    keys: BTreeMap<String, KeyMutation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
enum KeyMutation {
    Set { value: String },
    Clear,
}

const SETTINGS_SCHEMA_VERSION: u8 = 1;
const MAX_SETTINGS_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_API_KEY_BYTES: usize = 16 * 1024;

fn configured_key_env<'a>(cfg: &'a Config, section: &str, fallback: &'a str) -> &'a str {
    match section {
        "anthropic_api" => &cfg.anthropic_api.api_key_env,
        "zai" => &cfg.zai.api_key_env,
        "openrouter" => &cfg.openrouter.api_key_env,
        "deepseek" => &cfg.deepseek.api_key_env,
        "kimi" => &cfg.kimi.api_key_env,
        "kilo" => &cfg.kilo.api_key_env,
        "novita" => &cfg.novita.api_key_env,
        "moonshot" => &cfg.moonshot.api_key_env,
        "grok" => &cfg.grok.api_key_env,
        "minimax" => &cfg.minimax.api_key_env,
        "opencode-go" => &cfg.opencode_go.api_key_env,
        "tavily" => &cfg.tavily.api_key_env,
        "firecrawl" => &cfg.firecrawl.api_key_env,
        "requesty" => &cfg.requesty.api_key_env,
        "zenmux" => &cfg.zenmux.api_key_env,
        _ => fallback,
    }
}

fn snapshot_from_config_with(
    cfg: &Config,
    environment_configured: impl Fn(&str) -> bool,
) -> SettingsSnapshot {
    let state = SettingsState::from_config(cfg);
    let primary_choices = state
        .primary_choices
        .iter()
        .map(|id| PrimaryChoice {
            id: id.slug().to_string(),
            label: id.display_name().to_string(),
        })
        .collect();
    let active_choices = state
        .active_choices
        .iter()
        .map(|id| PrimaryChoice {
            id: id.slug().to_string(),
            label: id.display_name().to_string(),
        })
        .collect();
    let keys = KEY_VENDORS
        .iter()
        .map(|vendor| {
            let environment = configured_key_env(cfg, vendor.section, vendor.env);
            let inline_configured =
                config_inline_key(cfg, vendor.section).is_some_and(|v| !v.is_empty());
            let environment_configured = environment_configured(environment);
            KeyStatus {
                id: vendor.id.slug().to_string(),
                label: vendor.label.to_string(),
                environment: environment.to_string(),
                note: vendor.note.to_string(),
                configured: inline_configured || environment_configured,
                inline_configured,
                environment_configured,
            }
        })
        .collect();
    SettingsSnapshot {
        schema_version: SETTINGS_SCHEMA_VERSION,
        primary: state.primary.slug().to_string(),
        primary_choices,
        active_vendors: state
            .active_vendors
            .iter()
            .map(|id| id.slug().to_string())
            .collect(),
        active_choices,
        keys,
    }
}

fn settings_snapshot_json(cfg: &Config) -> Result<String> {
    Ok(serde_json::to_string(&snapshot_from_config_with(
        cfg,
        |environment| std::env::var_os(environment).is_some_and(|value| !value.is_empty()),
    ))?)
}

#[cfg(test)]
fn settings_snapshot_json_with(
    cfg: &Config,
    environment_configured: impl Fn(&str) -> bool,
) -> Result<String> {
    Ok(serde_json::to_string(&snapshot_from_config_with(
        cfg,
        environment_configured,
    ))?)
}

fn vendor_from_slug(slug: &str) -> Option<VendorId> {
    VendorId::all().iter().copied().find(|id| id.slug() == slug)
}

fn state_from_apply_request(cfg: &Config, raw: &str) -> Result<SettingsState> {
    let request: ApplyRequest = serde_json::from_str(raw)?;
    if request.schema_version != SETTINGS_SCHEMA_VERSION {
        return Err(AppError::Other(format!(
            "unsupported settings schema version {}",
            request.schema_version
        )));
    }

    let mut state = SettingsState::from_config(cfg);
    if let Some(active_vendors) = request.active_vendors {
        let mut selected = Vec::new();
        for slug in active_vendors {
            let id = vendor_from_slug(&slug)
                .ok_or_else(|| AppError::Other(format!("unknown active vendor {slug:?}")))?;
            if !state.active_choices.contains(&id) {
                return Err(AppError::Other(format!(
                    "active vendor {slug:?} is not enabled and configured"
                )));
            }
            if selected.contains(&id) {
                return Err(AppError::Other(format!(
                    "active_vendors contains duplicate vendor {slug:?}"
                )));
            }
            selected.push(id);
        }
        state.active_vendors = state
            .active_choices
            .iter()
            .copied()
            .filter(|id| selected.contains(id))
            .collect();
        state.primary_choices = state.active_vendors.clone();
        if !state.primary_choices.contains(&state.primary) {
            state.primary = state
                .primary_choices
                .first()
                .copied()
                .unwrap_or(VendorId::Anthropic);
        }
    }
    if let Some(primary) = request.primary {
        let id = vendor_from_slug(&primary)
            .ok_or_else(|| AppError::Other(format!("unknown primary vendor {primary:?}")))?;
        if !state.primary_choices.contains(&id) {
            return Err(AppError::Other(format!(
                "primary vendor {primary:?} is not active"
            )));
        }
        state.primary = id;
    }

    for (id, mutation) in request.keys {
        let index = KEY_VENDORS
            .iter()
            .position(|vendor| vendor.id.slug() == id)
            .ok_or_else(|| AppError::Other(format!("unknown API-key vendor {id:?}")))?;
        let input = &mut state.keys[index];
        match mutation {
            KeyMutation::Set { value } => {
                if value.is_empty() {
                    return Err(AppError::Other(format!(
                        "API key for {id:?} is empty; use the clear action to remove it"
                    )));
                }
                if value.len() > MAX_API_KEY_BYTES {
                    return Err(AppError::Other(format!(
                        "API key for {id:?} exceeds {MAX_API_KEY_BYTES} bytes"
                    )));
                }
                if value.chars().any(char::is_control) {
                    return Err(AppError::Other(format!(
                        "API key for {id:?} contains control characters"
                    )));
                }
                input.buf = value;
            }
            KeyMutation::Clear => input.buf.clear(),
        }
        input.cursor = input.buf.chars().count();
        input.dirty = true;
        input.revealed = false;
    }
    Ok(state)
}

#[cfg(test)]
fn apply_settings_json_to_path(cfg: &Config, raw: &str, path: &Path) -> Result<()> {
    let state = state_from_apply_request(cfg, raw)?;
    save_to_path(&state, path)
}

fn read_settings_request<R: BufRead>(reader: R) -> Result<String> {
    let mut limited = reader.take(MAX_SETTINGS_REQUEST_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_REQUEST_BYTES {
        return Err(AppError::Other(format!(
            "settings request exceeds {MAX_SETTINGS_REQUEST_BYTES} bytes"
        )));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| AppError::Other("settings request is not valid UTF-8".into()))
}

fn apply_settings_from_stdin() -> Result<()> {
    let raw = read_settings_request(std::io::stdin().lock())?;
    let cfg = Config::load()?;
    let state = state_from_apply_request(&cfg, &raw)?;
    save_to_config_default(&state)
}

/// Administrative settings bridge for native frontends. `show` never emits a
/// secret; `apply` accepts its patch only over stdin so keys do not appear in
/// argv or the process environment.
pub fn run_cli(action: &crate::widget::cli::SettingsAction) -> i32 {
    let result = match action {
        crate::widget::cli::SettingsAction::Show => Config::load()
            .and_then(|cfg| settings_snapshot_json(&cfg))
            .map(|json| println!("{json}")),
        crate::widget::cli::SettingsAction::Apply => {
            apply_settings_from_stdin().map(|()| println!(r#"{{"ok":true}}"#))
        }
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("settings: {error}");
            1
        }
    }
}

// ─── Render ────────────────────────────────────────────────────────────────

/// Render the modal overlay over `area`.
pub fn render(
    f: &mut Frame,
    area: Rect,
    state: &SettingsState,
    theme: &Theme,
    hits: &mut Vec<(SettingsRow, Rect)>,
) {
    let modal = centered_rect(74, 88, area);
    f.render_widget(Clear, modal);

    let bubble = bubble_theme(theme);
    let block = bubble.titled_modal_block(" Settings ");
    let inner = block.inner(modal);
    f.render_widget(block, modal);

    // Body (everything but the pinned hint) + a 1-line hint footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // A row's absolute y is the body top plus its line index (each rendered
    // line is exactly one row). Recorded alongside each interactive row so a
    // mouse click can be mapped back to a focus target.
    let row_at = |line: usize| Rect::new(inner.x, chunks[0].y + line as u16, inner.width, 1);

    // — Primary vendor + active providers + API keys —
    let mut lines: Vec<Line> = vec![
        section_header("Primary vendor", "shown first on the bar / TUI", &bubble),
        primary_line(state, &bubble),
        Line::from(""),
        section_header(
            "Dashboard providers",
            "checked providers refresh and appear in the dashboard",
            &bubble,
        ),
    ];
    hits.push((SettingsRow::Focus(Focus::Primary), row_at(1)));

    if state.active_choices.is_empty() {
        lines.push(Line::from(vec![
            bubble.span("     "),
            bubble.muted("No configured providers available"),
        ]));
    } else {
        for (index, vendor) in state.active_choices.iter().enumerate() {
            let focused = state.focus == Focus::Active(index);
            hits.push((
                SettingsRow::Focus(Focus::Active(index)),
                row_at(lines.len()),
            ));
            lines.push(active_provider_row(
                *vendor,
                state.is_active(index),
                focused,
                &bubble,
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(section_header(
        "API keys",
        "configured rows first; more providers below",
        &bubble,
    ));

    for (i, kv) in KEY_VENDORS.iter().enumerate() {
        // Unconfigured rows are grouped under the collapsed "More providers"
        // section and only revealed while it is expanded.
        if !state.configured.get(i).copied().unwrap_or(false) {
            continue;
        }
        let focused = state.focus == Focus::Key(i);
        hits.push((SettingsRow::Focus(Focus::Key(i)), row_at(lines.len())));
        lines.push(key_row(kv, &state.keys[i], focused, &bubble));
    }

    let unconfigured: Vec<usize> = KEY_VENDORS
        .iter()
        .enumerate()
        .filter(|(i, _)| !state.configured.get(*i).copied().unwrap_or(false))
        .map(|(i, _)| i)
        .collect();
    if !unconfigured.is_empty() {
        if state.show_more {
            lines.push(section_header(
                "More providers",
                &format!("{} without a key", unconfigured.len()),
                &bubble,
            ));
            for &i in &unconfigured {
                let focused = state.focus == Focus::Key(i);
                hits.push((SettingsRow::Focus(Focus::Key(i)), row_at(lines.len())));
                lines.push(key_row(&KEY_VENDORS[i], &state.keys[i], focused, &bubble));
            }
        } else {
            // Collapsed header — click (or navigating past the configured
            // rows) expands it.
            hits.push((SettingsRow::MoreHeader, row_at(lines.len())));
            lines.push(Line::from(vec![
                bubble.span(" "),
                Span::styled("More providers", bubble.title.add_modifier(Modifier::BOLD)),
                bubble.muted(format!(
                    "   — {} without a key · ↓/click to expand",
                    unconfigured.len()
                )),
            ]));
        }
    }

    lines.push(Line::from(""));

    // — Save + status —
    hits.push((SettingsRow::Focus(Focus::Save), row_at(lines.len())));
    lines.push(save_line(state.focus == Focus::Save, &bubble));
    if !state.status.is_empty() {
        let ok = state.status.starts_with("saved");
        let mark = if ok { "  ✓ " } else { "  ✗ " };
        let style = if ok { bubble.accent } else { bubble.selected };
        lines.push(Line::from(vec![
            Span::styled(mark, style.add_modifier(Modifier::BOLD)),
            Span::styled(state.status.clone(), bubble.muted),
        ]));
    }

    f.render_widget(Paragraph::new(lines), chunks[0]);

    // Context-aware hint footer.
    let hint = match state.focus {
        Focus::Primary => bubble.help_line([
            ("↑↓/tab", "move"),
            ("←→", "change vendor"),
            ("^S", "save"),
            ("esc", "close"),
        ]),
        Focus::Active(_) => bubble.help_line([
            ("↑↓/tab", "move"),
            ("space/enter", "toggle"),
            ("^S", "save"),
            ("esc", "close"),
        ]),
        Focus::Key(_) => bubble.help_line([
            ("↑↓/tab", "move"),
            ("type", "edit key"),
            ("^V", "reveal"),
            ("^S", "save"),
            ("esc", "close"),
        ]),
        Focus::Save => {
            bubble.help_line([("↑↓/tab", "move"), ("enter/^S", "save"), ("esc", "close")])
        }
    };
    f.render_widget(Paragraph::new(hint), chunks[1]);
}

fn section_header(title: &str, sub: &str, theme: &BubbleTheme) -> Line<'static> {
    Line::from(vec![
        theme.span(" "),
        Span::styled(title.to_string(), theme.title.add_modifier(Modifier::BOLD)),
        theme.muted(format!("   — {sub}")),
    ])
}

fn primary_line(state: &SettingsState, theme: &BubbleTheme) -> Line<'static> {
    let focused = state.focus == Focus::Primary;
    let name = state.primary.display_name().to_string();
    if focused {
        Line::from(vec![
            theme.span("   "),
            Span::styled("▸ ", theme.accent.add_modifier(Modifier::BOLD)),
            Span::styled("◀ ", theme.accent),
            Span::styled(
                format!(" {name} "),
                theme
                    .selected
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD),
            ),
            Span::styled(" ▶", theme.accent),
            theme.muted("    ← → to change"),
        ])
    } else {
        Line::from(vec![theme.span("     "), Span::styled(name, theme.text)])
    }
}

fn active_provider_row(
    vendor: VendorId,
    selected: bool,
    focused: bool,
    theme: &BubbleTheme,
) -> Line<'static> {
    let checkbox = if selected { "[x]" } else { "[ ]" };
    let label = vendor.display_name();
    if focused {
        let checkbox_style = if selected { theme.accent } else { theme.muted };
        Line::from(vec![
            theme.span("  "),
            Span::styled("▸ ", theme.accent.add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{checkbox} "),
                checkbox_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {label} "),
                theme
                    .selected
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD),
            ),
            theme.muted("    Space/Enter to toggle"),
        ])
    } else {
        let checkbox_style = if selected { theme.accent } else { theme.muted };
        let label_style = if selected { theme.text } else { theme.muted };
        Line::from(vec![
            theme.span("     "),
            Span::styled(format!("{checkbox} "), checkbox_style),
            Span::styled(label.to_string(), label_style),
        ])
    }
}

fn key_row(kv: &KeyVendor, input: &KeyInput, focused: bool, theme: &BubbleTheme) -> Line<'static> {
    let label = format!("{:<11}", kv.label);
    let value = value_text(input, focused);

    // Env / status suffix: env-var name, whether an env override is set, note.
    let env_set = std::env::var(kv.env)
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let mut suffix = format!("   {}", kv.env);
    if env_set {
        suffix.push_str(" · env set (overrides)");
    }
    if !kv.note.is_empty() {
        suffix.push_str(&format!(" · {}", kv.note));
    }

    if focused {
        let val_style = if input.buf.is_empty() {
            theme.accent.add_modifier(Modifier::BOLD)
        } else {
            theme.selected.add_modifier(Modifier::REVERSED)
        };
        let mut spans = vec![
            theme.span("  "),
            Span::styled("▸ ", theme.accent.add_modifier(Modifier::BOLD)),
            Span::styled(label, theme.title.add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {value} "), val_style),
        ];
        if input.revealed {
            spans.push(theme.muted("  [revealed]"));
        }
        spans.push(theme.muted(suffix));
        Line::from(spans)
    } else {
        let val_style = if input.buf.is_empty() {
            theme.muted
        } else {
            theme.text
        };
        Line::from(vec![
            theme.span("    "),
            Span::styled(label, theme.text),
            Span::styled(format!(" {value}"), val_style),
            theme.muted(suffix),
        ])
    }
}

/// The value column: `(empty)` / a cursor when focused-empty / masked or
/// revealed buffer with a cursor mark inserted when focused.
fn value_text(input: &KeyInput, focused: bool) -> String {
    if input.buf.is_empty() {
        return if focused {
            "‸".to_string()
        } else {
            "(empty)".to_string()
        };
    }
    let base = input.display();
    if !focused {
        return base;
    }
    let mut chars: Vec<char> = base.chars().collect();
    let pos = input.cursor.min(chars.len());
    chars.insert(pos, '‸');
    chars.into_iter().collect()
}

fn save_line(focused: bool, theme: &BubbleTheme) -> Line<'static> {
    let style = if focused {
        theme
            .selected
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        theme.accent.add_modifier(Modifier::BOLD)
    };
    let marker = if focused { "▸ " } else { "  " };
    Line::from(vec![
        theme.span("   "),
        Span::styled(marker, theme.accent.add_modifier(Modifier::BOLD)),
        Span::styled("  Save  (Ctrl-S)  ", style),
    ])
}

/// Center a rectangle of `percent_x * percent_y` over `r`.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_h = (r.height * percent_y) / 100;
    let popup_w = (r.width * percent_x) / 100;
    Rect {
        x: r.x + (r.width - popup_w) / 2,
        y: r.y + (r.height - popup_h) / 2,
        width: popup_w,
        height: popup_h,
    }
}

// crossterm types live behind ratatui; re-exported here for handle_key callers.
pub use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_config(initial: Option<&str>) -> (TempDir, std::path::PathBuf) {
        crate::cache::closed_temp_file("config.toml", initial)
    }

    fn key_index(id: VendorId) -> usize {
        KEY_VENDORS.iter().position(|kv| kv.id == id).unwrap()
    }

    fn blank_state(primary: VendorId) -> SettingsState {
        SettingsState {
            focus: Focus::Primary,
            primary_choices: VendorId::all().to_vec(),
            primary,
            active_choices: VendorId::all().to_vec(),
            active_vendors: VendorId::all().to_vec(),
            keys: KEY_VENDORS.iter().map(|_| KeyInput::default()).collect(),
            status: String::new(),
            // All-configured keeps the classic full ring for tests that do not
            // exercise the "More providers" grouping; grouping tests override.
            configured: vec![true; KEY_VENDORS.len()],
            show_more: false,
        }
    }

    /// State with a Z.AI key and an OpenRouter key, both marked dirty.
    fn state_with(zai: &str, opr: &str, primary: VendorId) -> SettingsState {
        let mut s = blank_state(primary);
        s.keys[key_index(VendorId::Zai)] = KeyInput::from_config(Some(zai));
        s.keys[key_index(VendorId::Zai)].dirty = true;
        s.keys[key_index(VendorId::Openrouter)] = KeyInput::from_config(Some(opr));
        s.keys[key_index(VendorId::Openrouter)].dirty = true;
        s
    }

    #[test]
    fn focus_cycles_through_primary_all_keys_and_save() {
        let mut f = Focus::Primary;
        let mut seen = vec![f];
        // Full cycle = Primary + N key rows + Save.
        for _ in 0..(KEY_VENDORS.len() + 2) {
            f = f.next();
            seen.push(f);
        }
        // Primary, Key(0..n), Save, back to Primary.
        assert_eq!(seen.first(), Some(&Focus::Primary));
        assert_eq!(seen.last(), Some(&Focus::Primary));
        assert!(seen.contains(&Focus::Key(0)));
        assert!(seen.contains(&Focus::Key(KEY_VENDORS.len() - 1)));
        assert!(seen.contains(&Focus::Save));
        // prev() is the inverse of next().
        assert_eq!(Focus::Primary.next().prev(), Focus::Primary);
        assert_eq!(Focus::Save.prev().next(), Focus::Save);
        assert_eq!(Focus::Primary.prev(), Focus::Save);
    }

    #[test]
    fn every_key_vendor_has_a_field() {
        // Every enabled-by-key vendor must be reachable in the form.
        for id in [
            VendorId::Zai,
            VendorId::Openrouter,
            VendorId::Deepseek,
            VendorId::Kilo,
            VendorId::Novita,
            VendorId::Moonshot,
            VendorId::Grok,
            VendorId::Tavily,
            VendorId::Firecrawl,
            VendorId::Requesty,
            VendorId::ZenMux,
        ] {
            assert!(
                KEY_VENDORS.iter().any(|kv| kv.id == id),
                "{id:?} has no key field"
            );
        }
        // OAuth vendors are intentionally absent.
        assert!(!KEY_VENDORS.iter().any(|kv| kv.id == VendorId::Anthropic));
        assert!(!KEY_VENDORS.iter().any(|kv| kv.id == VendorId::Openai));
    }

    #[test]
    fn requesty_key_vendor_uses_the_documented_contract() {
        let kv = KEY_VENDORS
            .iter()
            .find(|kv| kv.id == VendorId::Requesty)
            .unwrap();
        assert_eq!(kv.label, "Requesty");
        assert_eq!(kv.env, "REQUESTY_API_KEY");
        assert_eq!(kv.section, "requesty");
        assert_eq!(kv.note, "org balance & usage");
        // The overlay pre-fills an inline key already in config.
        let mut cfg = Config::default();
        cfg.requesty.api_key = Some("rqy-inline".into());
        let s = SettingsState::from_config(&cfg);
        assert_eq!(s.keys[key_index(VendorId::Requesty)].buf, "rqy-inline");
        assert!(s.configured[key_index(VendorId::Requesty)]);
    }

    #[test]
    fn zenmux_key_vendor_uses_the_management_contract() {
        let kv = KEY_VENDORS
            .iter()
            .find(|kv| kv.id == VendorId::ZenMux)
            .unwrap();
        assert_eq!(kv.label, "ZenMux");
        assert_eq!(kv.env, "ZENMUX_MANAGEMENT_API_KEY");
        assert_eq!(kv.section, "zenmux");
        assert_eq!(kv.note, "management key — PAYG + subscription");
        let mut cfg = Config::default();
        cfg.zenmux.api_key = Some("zmx-inline".into());
        let state = SettingsState::from_config(&cfg);
        assert_eq!(state.keys[key_index(VendorId::ZenMux)].buf, "zmx-inline");
        assert!(state.configured[key_index(VendorId::ZenMux)]);
    }

    #[test]
    fn from_config_prefills_existing_keys() {
        let mut cfg = Config::default();
        cfg.kilo.api_key = Some("sk-kilo".into());
        let s = SettingsState::from_config(&cfg);
        assert_eq!(s.keys[key_index(VendorId::Kilo)].buf, "sk-kilo");
        assert!(!s.keys[key_index(VendorId::Kilo)].dirty);
    }

    #[test]
    fn from_config_marks_inline_keys_as_configured() {
        let mut cfg = Config::default();
        cfg.tavily.api_key = Some("tvly-test".into());
        let s = SettingsState::from_config(&cfg);
        assert!(s.configured[key_index(VendorId::Tavily)]);
        // An unconfigured API-key vendor is only grouped when no env var or
        // inline key resolves it.
        if std::env::var("KIMI_API_KEY")
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            assert!(!s.configured[key_index(VendorId::Kimi)]);
        }
        assert!(!s.show_more);
    }

    #[test]
    fn grouped_ring_expands_at_the_boundary_and_collapses_back() {
        let mut s = blank_state(VendorId::Anthropic);
        // Only the first two key rows are configured; the rest are grouped.
        s.configured = KEY_VENDORS.iter().enumerate().map(|(i, _)| i < 2).collect();
        assert!(!s.show_more);

        // Down at the last configured row expands the section and focuses its
        // first (unconfigured) row.
        s.focus = Focus::Key(1);
        handle_key(&mut s, KeyCode::Down, KeyModifiers::NONE);
        assert!(s.show_more);
        assert_eq!(s.focus, Focus::Key(2));

        // Up at the first unconfigured row collapses the section again.
        handle_key(&mut s, KeyCode::Up, KeyModifiers::NONE);
        assert!(!s.show_more);
        assert_eq!(s.focus, Focus::Key(1));

        // While expanded, Down/Up walk the unconfigured rows normally.
        handle_key(&mut s, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.focus, Focus::Key(2));
        handle_key(&mut s, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.focus, Focus::Key(3));

        // Reaching Save from the last unconfigured row keeps the section
        // expanded (the user is inside it); Save is one more step.
        s.focus = Focus::Key(KEY_VENDORS.len() - 1);
        handle_key(&mut s, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.focus, Focus::Save);
        assert!(s.show_more);
    }

    #[test]
    fn toggle_more_moves_focus_to_first_unconfigured_and_back() {
        let mut s = blank_state(VendorId::Anthropic);
        s.configured = KEY_VENDORS
            .iter()
            .enumerate()
            .map(|(i, _)| i == 0)
            .collect();
        s.focus = Focus::Save;
        s.toggle_more();
        assert!(s.show_more);
        assert_eq!(s.focus, Focus::Key(1));
        s.toggle_more();
        assert!(!s.show_more);
        assert_eq!(s.focus, Focus::Key(0));
    }

    #[test]
    fn from_config_offers_active_vendors_only() {
        let cfg = Config::default();
        let s = SettingsState::from_config(&cfg);
        assert_eq!(s.primary_choices, cfg.active_vendors());
        // Opt-in vendors are disabled by default and must not be offered.
        assert!(!s.primary_choices.contains(&VendorId::Grok));
        if !s.primary_choices.is_empty() {
            assert!(s.primary_choices.contains(&s.primary));
        }
    }

    #[test]
    fn from_config_falls_back_when_configured_primary_is_disabled() {
        // Grok is opt-in; a config naming it as primary without enabling it
        // must display the first enabled vendor instead.
        let mut cfg = Config::default();
        cfg.ui.primary = Some(VendorId::Grok);
        let s = SettingsState::from_config(&cfg);
        assert_ne!(s.primary, VendorId::Grok);
        assert_eq!(Some(s.primary), cfg.active_vendors().first().copied());
    }

    #[test]
    fn key_input_insert_backspace_arrow() {
        let mut k = KeyInput::default();
        k.insert_char('a');
        k.insert_char('b');
        k.insert_char('c');
        assert_eq!(k.buf, "abc");
        assert_eq!(k.cursor, 3);
        assert!(k.dirty);
        k.move_left();
        k.move_left();
        assert_eq!(k.cursor, 1);
        k.insert_char('x');
        assert_eq!(k.buf, "axbc");
        assert_eq!(k.cursor, 2);
        k.backspace();
        assert_eq!(k.buf, "abc");
        assert_eq!(k.cursor, 1);
    }

    #[test]
    fn key_input_masks_by_default_reveals_on_toggle() {
        let mut k = KeyInput::default();
        for c in "secret-key".chars() {
            k.insert_char(c);
        }
        assert_eq!(k.display(), "•".repeat(10));
        k.toggle_reveal();
        assert_eq!(k.display(), "secret-key");
    }

    #[test]
    fn key_input_handles_unicode() {
        let mut k = KeyInput::default();
        k.insert_char('a');
        k.insert_char('→');
        k.insert_char('b');
        assert_eq!(k.buf, "a→b");
        assert_eq!(k.cursor, 3);
        k.move_left();
        k.backspace();
        assert_eq!(k.buf, "ab");
    }

    #[test]
    fn active_provider_checkbox_toggles_and_repairs_primary() {
        let mut state = blank_state(VendorId::Anthropic);
        state.active_choices = vec![VendorId::Anthropic, VendorId::Openai, VendorId::Firecrawl];
        state.active_vendors = vec![VendorId::Anthropic, VendorId::Openai];
        state.primary_choices = state.active_vendors.clone();

        state.focus = Focus::Active(2);
        handle_key(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(
            state.active_vendors,
            vec![VendorId::Anthropic, VendorId::Openai, VendorId::Firecrawl,]
        );

        state.focus = Focus::Active(0);
        handle_key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            state.active_vendors,
            vec![VendorId::Openai, VendorId::Firecrawl]
        );
        assert_eq!(state.primary, VendorId::Openai);
    }

    #[test]
    fn save_persists_explicit_active_vendor_list() {
        let (_dir, path) = temp_config(None);
        let mut state = blank_state(VendorId::Openai);
        state.active_choices = vec![VendorId::Anthropic, VendorId::Openai, VendorId::Firecrawl];
        state.active_vendors = vec![VendorId::Openai, VendorId::Firecrawl];
        state.primary_choices = state.active_vendors.clone();
        save_to_path(&state, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("active_vendors = [\"openai\", \"firecrawl\"]"));
        assert!(raw.contains("primary = \"openai\""));
    }

    #[test]
    fn save_materializes_all_provider_sections_without_enabling_opt_ins() {
        let (_dir, path) = temp_config(Some("[ui]\nprimary = \"anthropic\"\n"));
        let state = SettingsState::from_config(&Config::default());
        save_to_path(&state, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        for vendor in VendorId::all() {
            assert!(
                raw.contains(&format!("[{}]", vendor_config_section(*vendor))),
                "missing [{}] after Settings save",
                vendor_config_section(*vendor)
            );
        }
        let saved = Config::load_from(&path).unwrap();
        assert!(!saved.zenmux.enabled);
        assert!(!saved.requesty.enabled);
        assert!(!saved.tavily.enabled);
    }

    #[test]
    fn configured_disabled_zenmux_can_be_activated_visually() {
        let (_dir, path) = temp_config(None);
        let mut config = Config::default();
        config.zenmux.api_key = Some("zmx-test".into());
        assert!(!config.zenmux.enabled);
        let mut state = SettingsState::from_config(&config);
        let index = state
            .active_choices
            .iter()
            .position(|id| *id == VendorId::ZenMux)
            .expect("configured ZenMux is selectable before enabling");
        assert!(!state.active_vendors.contains(&VendorId::ZenMux));
        state.toggle_active(index);
        save_to_path(&state, &path).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert!(saved.zenmux.enabled);
        assert!(saved.ui.active_vendors.unwrap().contains(&VendorId::ZenMux));
    }

    #[test]
    fn saving_a_new_key_adds_its_provider_to_active_scope() {
        let (_dir, path) = temp_config(None);
        let mut state = blank_state(VendorId::Anthropic);
        state.active_choices = vec![VendorId::Anthropic];
        state.active_vendors = vec![VendorId::Anthropic];
        state.primary_choices = state.active_vendors.clone();
        let index = key_index(VendorId::Firecrawl);
        state.keys[index] = KeyInput::from_config(Some("fc-test"));
        state.keys[index].dirty = true;
        save_to_path(&state, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("active_vendors = [\"anthropic\", \"firecrawl\"]"));
        assert!(raw.contains("[firecrawl]"));
        assert!(raw.contains("enabled = true"));
    }

    #[test]
    fn settings_snapshot_exposes_active_choices_and_selection() {
        let mut config = Config::default();
        config.zai.api_key = Some("never-serialize-this".into());
        config.ui.active_vendors = Some(vec![VendorId::Anthropic, VendorId::Zai]);
        let raw = settings_snapshot_json_with(&config, |_| false).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value["active_vendors"],
            serde_json::json!(["anthropic", "zai"])
        );
        assert!(
            value["active_choices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|choice| choice["id"] == "zai")
        );
        assert!(!raw.contains("never-serialize-this"));
    }

    #[test]
    fn native_patch_applies_active_vendor_selection() {
        let mut config = Config::default();
        config.zai.api_key = Some("test-key".into());
        let request = serde_json::json!({
            "schema_version": 1,
            "active_vendors": ["zai", "anthropic"],
            "keys": {}
        });
        let state = state_from_apply_request(&config, &request.to_string()).unwrap();
        // Stored order is canonical for stable frontends, not request order.
        assert_eq!(
            state.active_vendors,
            vec![VendorId::Anthropic, VendorId::Zai]
        );
        assert_eq!(state.primary_choices, state.active_vendors);
    }

    #[test]
    fn value_text_shows_cursor_and_empty_states() {
        let mut k = KeyInput::default();
        assert_eq!(value_text(&k, false), "(empty)");
        assert_eq!(value_text(&k, true), "‸");
        k.insert_char('a');
        k.insert_char('b');
        // masked + cursor at end
        assert_eq!(value_text(&k, true), "••‸");
        assert_eq!(value_text(&k, false), "••");
    }

    #[test]
    fn save_writes_key_and_enables_vendor() {
        let (_dir, path) = temp_config(None);
        let mut s = blank_state(VendorId::Kilo);
        s.keys[key_index(VendorId::Kilo)] = KeyInput::from_config(Some("sk-kilo"));
        s.keys[key_index(VendorId::Kilo)].dirty = true;
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("primary = \"kilo\""));
        assert!(raw.contains("[kilo]"));
        assert!(raw.contains("api_key = \"sk-kilo\""));
        assert!(raw.contains("enabled = true"));
    }

    #[test]
    fn save_writes_minimal_toml_when_starting_empty() {
        let (_dir, path) = temp_config(None);
        let s = state_with("zk", "ok", VendorId::Zai);
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("primary = \"zai\""));
        assert!(raw.contains("[zai]"));
        assert!(raw.contains("api_key = \"zk\""));
        assert!(raw.contains("[openrouter]"));
        assert!(raw.contains("api_key = \"ok\""));
    }

    #[test]
    fn save_preserves_existing_comments_and_unrelated_fields() {
        let (_dir, path) = temp_config(Some(
            r##"# my comment
[ui]
# pre-existing comment
primary = "anthropic"

[zai]
enabled = true
api_key_env = "ZAI_API_KEY"
# tier comment
plan_tier = "pro"

[openrouter]
enabled = true
api_key_env = "OPENROUTER_API_KEY"

[[openrouter.accounts]]
label = "work"
api_key_env = "OPENROUTER_WORK_API_KEY"
"##,
        ));

        let s = state_with("zk2", "ok2", VendorId::Openrouter);
        save_to_path(&s, &path).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# my comment"));
        assert!(raw.contains("# pre-existing comment"));
        assert!(raw.contains("# tier comment"));
        assert!(raw.contains("api_key_env = \"ZAI_API_KEY\""));
        assert!(raw.contains("[[openrouter.accounts]]"));
        assert!(raw.contains("api_key_env = \"OPENROUTER_WORK_API_KEY\""));
        assert!(raw.contains("plan_tier = \"pro\""));
        assert!(raw.contains("primary = \"openrouter\""));
        assert!(raw.contains("api_key = \"zk2\""));
        assert!(raw.contains("api_key = \"ok2\""));
    }

    #[test]
    fn save_refuses_to_replace_an_unreadable_existing_config() {
        let (_dir, path) = temp_config(None);
        let original = [0xff, 0xfe, 0xfd];
        std::fs::write(&path, original).unwrap();
        let state = state_with("new-secret", "", VendorId::Zai);

        assert!(save_to_path(&state, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn save_does_not_write_empty_key_when_dirty_but_blank() {
        let (_dir, path) = temp_config(None);
        let mut s = blank_state(VendorId::Anthropic);
        // Focus each key, do nothing but mark dirty (blank).
        for k in &mut s.keys {
            k.dirty = true;
        }
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("api_key ="));
    }

    #[test]
    #[cfg(unix)]
    fn save_chmods_to_600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = temp_config(None);
        let s = state_with("zk", "ok", VendorId::Zai);
        save_to_path(&s, &path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn tab_cycles_focus_from_primary_to_first_active_provider() {
        let mut s = blank_state(VendorId::Anthropic);
        assert_eq!(
            handle_key(&mut s, KeyCode::Tab, KeyModifiers::NONE),
            Action::Continue
        );
        assert_eq!(s.focus, Focus::Active(0));
        assert_eq!(
            handle_key(&mut s, KeyCode::BackTab, KeyModifiers::NONE),
            Action::Continue
        );
        assert_eq!(s.focus, Focus::Primary);
    }

    #[test]
    fn esc_closes_without_saving() {
        let mut s = blank_state(VendorId::Anthropic);
        assert_eq!(
            handle_key(&mut s, KeyCode::Esc, KeyModifiers::NONE),
            Action::Close
        );
    }

    #[test]
    fn left_right_cycles_primary_vendor() {
        // Canonical order (VendorId::all): Anthropic, AnthropicApi, Openai, …
        let mut s = blank_state(VendorId::Anthropic);
        handle_key(&mut s, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::AnthropicApi);
        handle_key(&mut s, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::Openai);
        handle_key(&mut s, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::AnthropicApi);
    }

    #[test]
    fn left_right_offers_enabled_vendors_only() {
        // The selector must never land on a vendor the widget cannot use.
        let mut s = blank_state(VendorId::Anthropic);
        s.primary_choices = vec![VendorId::Anthropic, VendorId::Grok];
        handle_key(&mut s, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::Grok);
        // Wraps within the enabled set rather than walking into disabled ones.
        handle_key(&mut s, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::Anthropic);
        handle_key(&mut s, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::Grok);
    }

    #[test]
    fn no_enabled_vendors_leaves_primary_selector_inert() {
        let mut s = blank_state(VendorId::Anthropic);
        s.primary_choices = vec![];
        handle_key(&mut s, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::Anthropic);
    }

    #[test]
    fn save_does_not_write_a_disabled_primary() {
        // Saving must not leave a primary outside the active fetch scope.
        let (_dir, path) = temp_config(Some("[ui]\nprimary = \"anthropic\"\n"));
        let mut s = state_with("zk", "ok", VendorId::Grok);
        s.primary_choices = vec![VendorId::Anthropic];
        s.active_vendors = vec![VendorId::Anthropic];
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("primary ="));
        assert!(!raw.contains("primary = \"grok\""));
        // The keys still saved.
        assert!(raw.contains("zk"));
    }

    #[test]
    fn save_removes_an_inline_key_the_user_cleared() {
        // Clearing the field in the overlay must delete the secret from the
        // file — otherwise there is no way to remove it short of hand-editing.
        let (_dir, path) = temp_config(Some(
            "[zai]\nenabled = true\napi_key = \"old-secret\"\nplan_tier = \"pro\"\n",
        ));
        let mut s = blank_state(VendorId::Zai);
        s.primary_choices = vec![VendorId::Zai];
        s.keys[key_index(VendorId::Zai)] = KeyInput::default();
        s.keys[key_index(VendorId::Zai)].dirty = true;
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("old-secret"));
        assert!(!raw.contains("api_key ="));
        // Unrelated fields in the same section survive.
        assert!(raw.contains("plan_tier = \"pro\""));
    }

    #[test]
    fn untouched_key_field_is_left_alone() {
        // Not dirty => the file's existing secret must survive a save.
        let (_dir, path) = temp_config(Some("[zai]\napi_key = \"keep-me\"\n"));
        let mut s = blank_state(VendorId::Zai);
        s.primary_choices = vec![VendorId::Zai];
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("keep-me"));
    }

    #[test]
    fn typing_edits_the_focused_key_only() {
        let mut s = blank_state(VendorId::Anthropic);
        s.focus = Focus::Key(key_index(VendorId::Grok));
        for c in "xai-abc".chars() {
            handle_key(&mut s, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(s.keys[key_index(VendorId::Grok)].buf, "xai-abc");
        assert!(s.keys[key_index(VendorId::Grok)].dirty);
        // No other field was touched.
        assert!(s.keys[key_index(VendorId::Zai)].buf.is_empty());
    }

    #[test]
    fn ctrl_v_toggles_reveal_on_focused_key_field() {
        let mut s = blank_state(VendorId::Anthropic);
        let zi = key_index(VendorId::Zai);
        s.focus = Focus::Key(zi);
        s.keys[zi] = KeyInput::from_config(Some("secret"));
        assert!(!s.keys[zi].revealed);
        handle_key(&mut s, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert!(s.keys[zi].revealed);
        handle_key(&mut s, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert!(!s.keys[zi].revealed);
    }

    #[test]
    fn control_chorded_chars_do_not_type_into_fields() {
        let mut s = blank_state(VendorId::Anthropic);
        s.focus = Focus::Key(0);
        // Ctrl-A must NOT insert a literal 'a' or mark the field dirty.
        handle_key(&mut s, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert!(s.keys[0].buf.is_empty());
        assert!(!s.keys[0].dirty);
        // Ctrl-C quits the host TUI even while the overlay owns focus.
        assert_eq!(
            handle_key(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Action::Quit
        );
        // A plain char still types normally.
        handle_key(&mut s, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(s.keys[0].buf, "x");
    }

    #[test]
    fn ctrl_v_on_non_key_focus_is_noop() {
        let mut s = blank_state(VendorId::Anthropic);
        s.focus = Focus::Primary;
        // Must not panic when no key field is focused.
        assert_eq!(
            handle_key(&mut s, KeyCode::Char('v'), KeyModifiers::CONTROL),
            Action::Continue
        );
    }

    fn state_focused_on_zai() -> SettingsState {
        let mut state = blank_state(VendorId::Anthropic);
        state.focus = Focus::Key(key_index(VendorId::Zai));
        state
    }

    #[test]
    fn handle_key_ctrl_c_quits_without_typing_into_key_field() {
        let mut s = state_focused_on_zai();
        let zi = key_index(VendorId::Zai);
        assert_eq!(
            handle_key(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Action::Quit
        );
        assert!(s.keys[zi].buf.is_empty());
        // Untouched means save still leaves an existing key on disk alone.
        assert!(!s.keys[zi].dirty);
    }

    #[test]
    fn handle_key_alt_chord_does_not_type_into_key_field() {
        let mut s = state_focused_on_zai();
        let zi = key_index(VendorId::Zai);
        handle_key(&mut s, KeyCode::Char('x'), KeyModifiers::ALT);
        assert!(s.keys[zi].buf.is_empty());
        assert!(!s.keys[zi].dirty);
    }

    #[test]
    fn handle_key_platform_modifier_chords_do_not_type_into_key_field() {
        for modifier in [KeyModifiers::SUPER, KeyModifiers::HYPER, KeyModifiers::META] {
            let mut s = state_focused_on_zai();
            let zi = key_index(VendorId::Zai);
            handle_key(&mut s, KeyCode::Char('x'), modifier);
            assert!(s.keys[zi].buf.is_empty(), "modifier {modifier:?}");
            assert!(!s.keys[zi].dirty, "modifier {modifier:?}");
        }
    }

    #[test]
    fn handle_key_shift_still_types_uppercase() {
        let mut s = state_focused_on_zai();
        let zi = key_index(VendorId::Zai);
        handle_key(&mut s, KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(s.keys[zi].buf, "A");
        assert!(s.keys[zi].dirty);
    }

    #[test]
    fn handle_key_plain_space_still_cycles_primary_vendor() {
        let mut s = blank_state(VendorId::Anthropic);
        handle_key(&mut s, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(s.primary, VendorId::AnthropicApi);
    }

    #[test]
    fn handle_key_ctrl_s_attempts_save_from_any_field() {
        let (_dir, path) = temp_config(None);
        let s = state_with("zk", "ok", VendorId::Zai);
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("api_key = \"zk\""));
    }
    #[test]
    fn save_to_path_writes_kimi_key_when_dirty() {
        let (_dir, path) = temp_config(None);
        let mut s = blank_state(VendorId::Anthropic);
        let kimi = key_index(VendorId::Kimi);
        s.keys[kimi] = KeyInput::from_config(Some("kk"));
        s.keys[kimi].dirty = true;
        save_to_path(&s, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[kimi]"));
        assert!(raw.contains("api_key = \"kk\""));
    }

    #[test]
    fn settings_save_uses_the_same_config_path_as_load() {
        assert_eq!(
            default_config_path().unwrap(),
            crate::config::resolved_path().unwrap()
        );
    }

    #[test]
    fn native_snapshot_reports_key_state_without_serializing_secrets() {
        let mut cfg = Config::default();
        cfg.zai.api_key = Some("never-leak-this-key".into());
        cfg.zai.api_key_env = "CUSTOM_ZAI_KEY".into();
        let raw = settings_snapshot_json_with(&cfg, |name| name == "CUSTOM_ZAI_KEY").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["primary"], "anthropic");
        let zai = parsed["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "zai")
            .unwrap();
        assert_eq!(zai["configured"], true);
        assert_eq!(zai["inline_configured"], true);
        assert_eq!(zai["environment_configured"], true);
        assert_eq!(zai["environment"], "CUSTOM_ZAI_KEY");
        assert!(!raw.contains("never-leak-this-key"));
        assert!(parsed.get("api_key").is_none());
    }

    #[test]
    fn native_key_only_patch_does_not_require_or_replace_primary() {
        let cfg = Config::default();
        let original_primary = SettingsState::from_config(&cfg).primary;
        let request = serde_json::json!({
            "schema_version": 1,
            "keys": {"kimi": {"action": "set", "value": "new-kimi-key"}}
        });

        let state = state_from_apply_request(&cfg, &request.to_string()).unwrap();
        assert_eq!(state.primary, original_primary);
        let kimi_index = KEY_VENDORS
            .iter()
            .position(|vendor| vendor.id == VendorId::Kimi)
            .unwrap();
        assert!(state.keys[kimi_index].dirty);
        assert_eq!(state.keys[kimi_index].buf, "new-kimi-key");
    }

    #[test]
    fn native_patch_reuses_tui_persistence_and_preserves_existing_config() {
        let (_dir, path) = temp_config(Some(
            r#"# keep this comment
[ui]
primary = "anthropic"

[zai]
enabled = true
api_key_env = "ZAI_API_KEY"
plan_tier = "pro"

[openrouter]
enabled = true
"#,
        ));
        let cfg = Config::load_from(&path).unwrap();
        let request = serde_json::json!({
            "schema_version": 1,
            "primary": "openrouter",
            "keys": {
                "zai": {"action": "set", "value": "new-zai-key"}
            }
        });

        apply_settings_json_to_path(&cfg, &request.to_string(), &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# keep this comment"));
        assert!(raw.contains("plan_tier = \"pro\""));
        assert!(raw.contains("api_key_env = \"ZAI_API_KEY\""));
        assert!(raw.contains("primary = \"openrouter\""));
        assert!(raw.contains("api_key = \"new-zai-key\""));
    }

    #[test]
    fn native_patch_distinguishes_clear_from_unchanged() {
        let (_dir, path) = temp_config(Some(
            "[zai]\nenabled = true\napi_key = \"remove-me\"\n\
             [openrouter]\nenabled = true\napi_key = \"keep-me\"\n",
        ));
        let cfg = Config::load_from(&path).unwrap();
        let request = serde_json::json!({
            "schema_version": 1,
            "primary": "zai",
            "keys": {"zai": {"action": "clear"}}
        });

        apply_settings_json_to_path(&cfg, &request.to_string(), &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("remove-me"));
        assert!(raw.contains("keep-me"));
    }

    #[test]
    fn native_patch_errors_never_echo_key_values() {
        let raw = serde_json::json!({
            "schema_version": 1,
            "primary": "anthropic",
            "keys": {
                "zai": {"action": "set", "value": "secret\nwith-control"}
            }
        })
        .to_string();
        let error = state_from_apply_request(&Config::default(), &raw)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"));
        assert!(error.contains("control characters"));
    }

    #[test]
    fn native_patch_input_is_bounded_before_json_parsing() {
        let oversized = vec![b'x'; MAX_SETTINGS_REQUEST_BYTES as usize + 1];
        let error = read_settings_request(std::io::Cursor::new(oversized))
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds"));
    }
}
