//! Follows the desktop theme.
//!
//! Colors come from the active Omarchy theme (`~/.config/omarchy/current/theme`)
//! and light/dark from the XDG desktop portal's `color-scheme` setting. Both
//! are watched on background threads so a theme switch restyles the app live.
//! Off Omarchy, the app falls back to plain light and dark palettes.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

/// An sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parses `#rrggbb`, `0xrrggbb`, `rrggbb`, `rgb(rrggbb)` or `rgba(rrggbbaa)`.
    /// Alpha is ignored.
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
        let s = s
            .strip_prefix("rgba(")
            .or_else(|| s.strip_prefix("rgb("))
            .map(|s| s.trim_end_matches(')'))
            .unwrap_or(s);
        let hex = s
            .strip_prefix('#')
            .or_else(|| s.strip_prefix("0x"))
            .unwrap_or(s);
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Relative luminance, 0 (black) to 1 (white).
    pub fn luminance(self) -> f32 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Linear-light RGB components, for the GPU.
    pub fn linear(self) -> [f32; 3] {
        let f = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [f(self.0), f(self.1), f(self.2)]
    }

    /// Mixes towards `other` by `t` (0 = self, 1 = other), in sRGB space.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }

    pub fn egui(self) -> egui::Color32 {
        egui::Color32::from_rgb(self.0, self.1, self.2)
    }
}

/// The handful of colors the UI is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    /// Theme name for the status bar, e.g. "tokyo-night".
    pub name: String,
    pub dark: bool,
    pub background: Rgb,
    pub foreground: Rgb,
    /// Highlight color: selection, focused widgets, picked bodies.
    pub accent: Rgb,
    pub error: Rgb,
    pub warning: Rgb,
}

impl Palette {
    pub fn fallback(dark: bool) -> Palette {
        if dark {
            Palette {
                name: "Default dark".into(),
                dark,
                background: Rgb(0x1e, 0x1f, 0x22),
                foreground: Rgb(0xd4, 0xd4, 0xd4),
                accent: Rgb(0x4a, 0x9e, 0xff),
                error: Rgb(0xf0, 0x5d, 0x5d),
                warning: Rgb(0xe5, 0xc0, 0x7b),
            }
        } else {
            Palette {
                name: "Default light".into(),
                dark,
                background: Rgb(0xf3, 0xf3, 0xf3),
                foreground: Rgb(0x1f, 0x1f, 0x1f),
                accent: Rgb(0x00, 0x66, 0xcc),
                error: Rgb(0xc4, 0x2b, 0x1c),
                warning: Rgb(0x9a, 0x6a, 0x00),
            }
        }
    }

    /// Restyles egui from this palette.
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let (bg, fg, accent) = (self.background, self.foreground, self.accent);
        // Panels sit slightly off the background so the viewport stands out.
        let panel = bg.mix(fg, 0.04);
        let raised = bg.mix(fg, 0.10);
        let hover = bg.mix(fg, 0.18);
        v.override_text_color = Some(fg.egui());
        v.panel_fill = panel.egui();
        v.window_fill = panel.egui();
        v.faint_bg_color = bg.mix(fg, 0.02).egui();
        v.extreme_bg_color = bg.egui();
        v.code_bg_color = raised.egui();
        v.hyperlink_color = accent.egui();
        v.error_fg_color = self.error.egui();
        v.warn_fg_color = self.warning.egui();
        v.selection.bg_fill = accent.mix(bg, 0.55).egui();
        v.selection.stroke.color = fg.egui();
        v.window_stroke.color = raised.egui();
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = panel.egui();
        w.noninteractive.weak_bg_fill = panel.egui();
        w.noninteractive.bg_stroke.color = raised.egui();
        w.noninteractive.fg_stroke.color = fg.egui();
        for (state, fill) in [
            (&mut w.inactive, raised),
            (&mut w.hovered, hover),
            (&mut w.active, accent.mix(bg, 0.4)),
            (&mut w.open, raised),
        ] {
            state.bg_fill = fill.egui();
            state.weak_bg_fill = fill.egui();
            state.fg_stroke.color = fg.egui();
        }
        w.hovered.bg_stroke.color = accent.egui();
        w.active.bg_stroke.color = accent.egui();
        v
    }
}

/// The portal's `org.freedesktop.appearance color-scheme` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    NoPreference,
    Dark,
    Light,
}

/// Picks the palette from what the desktop reports. An Omarchy theme wins;
/// otherwise the portal's preference, then `system_dark` (winit's guess).
pub fn resolve(omarchy: Option<&Palette>, scheme: ColorScheme, system_dark: bool) -> Palette {
    if let Some(p) = omarchy {
        return p.clone();
    }
    Palette::fallback(match scheme {
        ColorScheme::Dark => true,
        ColorScheme::Light => false,
        ColorScheme::NoPreference => system_dark,
    })
}

/// A change reported by one of the watcher threads.
#[derive(Debug)]
pub enum ThemeEvent {
    Omarchy(Option<Palette>),
    Scheme(ColorScheme),
}

/// The Omarchy current-theme directory, if this is an Omarchy desktop.
pub fn omarchy_theme_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let dir = config.join("omarchy/current/theme");
    dir.is_dir().then_some(dir)
}

/// Files read from a theme directory, most specific first. Omarchy ships
/// one config per app; we read whichever the theme provides.
const THEME_FILES: [&str; 6] = [
    "colors.toml",
    "alacritty.toml",
    "kitty.conf",
    "ghostty.conf",
    "hyprland.conf",
    "light.mode",
];

/// Theme files are tiny; anything bigger is not a theme file.
const MAX_THEME_FILE: u64 = 256 * 1024;

fn read_small(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_THEME_FILE {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// Reads the palette from an Omarchy theme directory. Returns `None` if no
/// file in it names at least a background and foreground.
pub fn load_omarchy(dir: &Path) -> Option<Palette> {
    let mut colors: HashMap<&str, Rgb> = HashMap::new();
    let mut set = |key: &'static str, value: Option<Rgb>| {
        if let Some(c) = value {
            colors.entry(key).or_insert(c);
        }
    };

    // Omarchy 3.x themes: one flat colors.toml.
    if let Some(text) = read_small(&dir.join("colors.toml")) {
        let kv = parse_toml_ish(&text);
        let get = |k: &str| kv.get(k).and_then(|v| Rgb::parse(v));
        set("background", get("background"));
        set("foreground", get("foreground"));
        set("accent", get("accent"));
        set("red", get("color1"));
        set("yellow", get("color3"));
        set("blue", get("color4"));
    }
    if let Some(text) = read_small(&dir.join("alacritty.toml")) {
        let kv = parse_toml_ish(&text);
        let get = |k: &str| kv.get(k).and_then(|v| Rgb::parse(v));
        set("background", get("colors.primary.background"));
        set("foreground", get("colors.primary.foreground"));
        set("red", get("colors.normal.red"));
        set("yellow", get("colors.normal.yellow"));
        set("blue", get("colors.normal.blue"));
    }
    for (file, sep) in [("kitty.conf", ' '), ("ghostty.conf", '=')] {
        let Some(text) = read_small(&dir.join(file)) else {
            continue;
        };
        let kv = parse_conf(&text, sep);
        let get = |k: &str| kv.get(k).and_then(|v| Rgb::parse(v));
        set("background", get("background"));
        set("foreground", get("foreground"));
        set("red", get("color1").or_else(|| get("palette.1")));
        set("yellow", get("color3").or_else(|| get("palette.3")));
        set("blue", get("color4").or_else(|| get("palette.4")));
    }
    // The window border color is the most "Omarchy" accent there is.
    if let Some(text) = read_small(&dir.join("hyprland.conf")) {
        set("accent", hyprland_active_border(&text));
    }

    let background = *colors.get("background")?;
    let foreground = *colors.get("foreground")?;
    let dark = if dir.join("light.mode").exists() {
        false
    } else {
        background.luminance() < 0.5
    };
    let fallback = Palette::fallback(dark);
    Some(Palette {
        name: theme_name(dir),
        dark,
        background,
        foreground,
        accent: colors
            .get("accent")
            .or(colors.get("blue"))
            .copied()
            .unwrap_or(fallback.accent),
        error: colors.get("red").copied().unwrap_or(fallback.error),
        warning: colors.get("yellow").copied().unwrap_or(fallback.warning),
    })
}

/// The theme's name: the symlink target's directory name if `current/theme`
/// is a link, else Omarchy's `theme.name` file next to it.
fn theme_name(dir: &Path) -> String {
    if let Ok(target) = fs::read_link(dir) {
        if let Some(name) = target.file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    dir.parent()
        .and_then(|p| read_small(&p.join("theme.name")))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Omarchy".into())
}

/// Flattens `[a.b]` sections and `key = value` lines into `a.b.key -> value`.
/// Enough TOML for theme files; anything fancier is skipped.
fn parse_toml_ish(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().trim_matches('[').trim_matches(']').to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = match value.strip_prefix('"').or_else(|| value.strip_prefix('\'')) {
            Some(rest) => rest.split(['"', '\'']).next().unwrap_or(""),
            None => value.split('#').next().unwrap_or("").trim(),
        };
        let key = key.trim().trim_matches('"');
        let full = if section.is_empty() {
            key.to_owned()
        } else {
            format!("{section}.{key}")
        };
        out.insert(full, value.to_owned());
    }
    out
}

/// Parses `key<sep>value` lines (kitty uses spaces, ghostty `=`). Ghostty's
/// `palette = 4=#rrggbb` becomes `palette.4`.
fn parse_conf(text: &str, sep: char) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(sep) else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "palette" {
            if let Some((index, color)) = value.split_once('=') {
                out.insert(format!("palette.{}", index.trim()), color.trim().to_owned());
            }
        } else {
            out.insert(key.to_owned(), value.to_owned());
        }
    }
    out
}

/// First color of Hyprland's active border, from either
/// `$activeBorderColor = rgb(..)` or `col.active_border = rgba(..) rgba(..) 45deg`.
fn hyprland_active_border(text: &str) -> Option<Rgb> {
    let mut variables: HashMap<&str, &str> = HashMap::new();
    let mut border = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.starts_with('$') {
            variables.insert(key, value);
        }
        if key == "col.active_border" || key == "$activeBorderColor" {
            border = Some(value);
        }
    }
    let value = border?;
    let value = variables.get(value).copied().unwrap_or(value);
    value.split_whitespace().find_map(Rgb::parse)
}

/// Cheap fingerprint of a theme directory, to notice a theme switch. Omarchy
/// switches by repointing a symlink, so the resolved path changes; editing a
/// theme in place changes file mtimes.
/// Resolved theme path plus (mtime, size) of each of [`THEME_FILES`].
type Fingerprint = (PathBuf, Vec<Option<(SystemTime, u64)>>);

fn fingerprint(dir: &Path) -> Option<Fingerprint> {
    let path = fs::canonicalize(dir).ok()?;
    let files = THEME_FILES
        .iter()
        .map(|f| {
            let m = fs::metadata(path.join(f)).ok()?;
            Some((m.modified().ok()?, m.len()))
        })
        .collect();
    Some((path, files))
}

/// Starts the watcher threads. Each sends its current value immediately, then
/// on every change, and calls `wake` so the UI repaints.
pub fn watch(tx: Sender<ThemeEvent>, wake: impl Fn() + Send + Clone + 'static) {
    {
        let (tx, wake) = (tx.clone(), wake.clone());
        std::thread::Builder::new()
            .name("omarchy-theme".into())
            .spawn(move || watch_omarchy(tx, wake))
            .ok();
    }
    std::thread::Builder::new()
        .name("portal-theme".into())
        .spawn(move || {
            if let Err(e) = portal::watch(&tx, &wake) {
                eprintln!("note: no desktop portal color-scheme ({e}); using defaults");
            }
        })
        .ok();
}

/// Polls once a second: a readlink and a few stats, far cheaper than a frame.
/// Polling also survives `~/.config/omarchy/current` being recreated, which
/// an inotify watch would not.
fn watch_omarchy(tx: Sender<ThemeEvent>, wake: impl Fn()) {
    let mut last = None;
    let mut first = true;
    loop {
        let dir = omarchy_theme_dir();
        let print = dir.as_deref().and_then(fingerprint);
        if first || print != last {
            let palette = dir.as_deref().and_then(load_omarchy);
            if tx.send(ThemeEvent::Omarchy(palette)).is_err() {
                return;
            }
            wake();
            last = print;
            first = false;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

mod portal {
    use super::{ColorScheme, ThemeEvent};
    use std::sync::mpsc::Sender;
    use zbus::zvariant::{OwnedValue, Value};

    const NAMESPACE: &str = "org.freedesktop.appearance";
    const KEY: &str = "color-scheme";

    fn scheme(value: &Value<'_>) -> ColorScheme {
        match value {
            // `Read` (portal v1) wraps the value in an extra variant.
            Value::Value(inner) => scheme(inner),
            Value::U32(1) => ColorScheme::Dark,
            Value::U32(2) => ColorScheme::Light,
            _ => ColorScheme::NoPreference,
        }
    }

    pub fn watch(tx: &Sender<ThemeEvent>, wake: &dyn Fn()) -> zbus::Result<()> {
        let conn = zbus::blocking::Connection::session()?;
        let proxy = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )?;
        // Subscribe before reading so a change in between isn't lost.
        let changes = proxy.receive_signal("SettingChanged")?;
        let current: OwnedValue = proxy
            .call("ReadOne", &(NAMESPACE, KEY))
            .or_else(|_| proxy.call("Read", &(NAMESPACE, KEY)))?;
        if tx.send(ThemeEvent::Scheme(scheme(&current))).is_err() {
            return Ok(());
        }
        wake();
        for msg in changes {
            let Ok((ns, key, value)) = msg.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if ns == NAMESPACE && key == KEY {
                if tx.send(ThemeEvent::Scheme(scheme(&value))).is_err() {
                    return Ok(());
                }
                wake();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_dir(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sm-theme-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    #[test]
    fn parses_colors() {
        assert_eq!(Rgb::parse("#1a1b26"), Some(Rgb(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("0x1A1B26"), Some(Rgb(0x1a, 0x1b, 0x26)));
        assert_eq!(Rgb::parse("rgba(33ccffee)"), Some(Rgb(0x33, 0xcc, 0xff)));
        assert_eq!(Rgb::parse("\"#ffffff\""), Some(Rgb(255, 255, 255)));
        assert_eq!(Rgb::parse("#fff"), None);
        assert_eq!(Rgb::parse("red"), None);
    }

    #[test]
    fn reads_alacritty_and_hyprland() {
        let dir = theme_dir(&[
            (
                "alacritty.toml",
                "[colors.primary]\nbackground = '#1a1b26'\nforeground = \"#a9b1d6\" # fg\n\n\
                 [colors.normal]\nred = '#f7768e'\nblue = '#7aa2f7'\n",
            ),
            (
                "hyprland.conf",
                "$activeBorderColor = rgb(33ccff)\ngeneral {\n  col.active_border = $activeBorderColor\n}\n",
            ),
        ]);
        let p = load_omarchy(&dir).unwrap();
        assert!(p.dark);
        assert_eq!(p.background, Rgb(0x1a, 0x1b, 0x26));
        assert_eq!(p.foreground, Rgb(0xa9, 0xb1, 0xd6));
        assert_eq!(p.accent, Rgb(0x33, 0xcc, 0xff));
        assert_eq!(p.error, Rgb(0xf7, 0x76, 0x8e));
    }

    #[test]
    fn colors_toml_wins_and_light_mode_flag() {
        let dir = theme_dir(&[
            (
                "colors.toml",
                "accent = \"#ff8800\"\nbackground = \"#fafafa\"\nforeground = \"#202020\"\n",
            ),
            (
                "alacritty.toml",
                "[colors.primary]\nbackground = '#000000'\nforeground = '#ffffff'\n",
            ),
            ("light.mode", ""),
        ]);
        let p = load_omarchy(&dir).unwrap();
        assert!(!p.dark);
        assert_eq!(p.background, Rgb(0xfa, 0xfa, 0xfa));
        assert_eq!(p.accent, Rgb(0xff, 0x88, 0x00));
    }

    #[test]
    fn reads_kitty_and_ghostty() {
        let kitty = theme_dir(&[(
            "kitty.conf",
            "foreground #c0caf5\nbackground  #24283b\ncolor4 #7aa2f7\n",
        )]);
        let p = load_omarchy(&kitty).unwrap();
        assert_eq!(p.background, Rgb(0x24, 0x28, 0x3b));
        assert_eq!(p.accent, Rgb(0x7a, 0xa2, 0xf7));

        let ghostty = theme_dir(&[(
            "ghostty.conf",
            "background = 282828\nforeground = ebdbb2\npalette = 4=#458588\n",
        )]);
        let p = load_omarchy(&ghostty).unwrap();
        assert_eq!(p.foreground, Rgb(0xeb, 0xdb, 0xb2));
        assert_eq!(p.accent, Rgb(0x45, 0x85, 0x88));
    }

    #[test]
    fn incomplete_theme_is_ignored() {
        let dir = theme_dir(&[(
            "alacritty.toml",
            "[colors.primary]\nbackground = '#000000'\n",
        )]);
        assert!(load_omarchy(&dir).is_none());
    }

    #[test]
    fn fingerprint_changes_on_edit() {
        let dir = theme_dir(&[("colors.toml", "background = \"#000000\"\n")]);
        let a = fingerprint(&dir);
        fs::write(dir.join("light.mode"), "").unwrap();
        assert_ne!(a, fingerprint(&dir));
    }

    #[test]
    fn resolve_prefers_omarchy_then_portal() {
        let omarchy = Palette {
            name: "x".into(),
            ..Palette::fallback(true)
        };
        assert_eq!(resolve(Some(&omarchy), ColorScheme::Light, false), omarchy);
        assert!(!resolve(None, ColorScheme::Light, true).dark);
        assert!(resolve(None, ColorScheme::Dark, false).dark);
        assert!(resolve(None, ColorScheme::NoPreference, true).dark);
    }
}
