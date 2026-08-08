use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub background: Style,
    pub border: Style,
    pub border_selected: Style,
    pub border_marked: Style,
    pub title: Style,
    pub label: Style,
    pub label_selected: Style,
    pub status_bar: Style,
    pub status_bar_error: Style,
    pub status_bar_dim: Style,
    pub mode_normal: Style,
    pub mode_search: Style,
    pub search_input: Style,
    pub search_cursor: Style,
    pub popup_border: Style,
    pub popup_title: Style,
    pub popup_text: Style,
    pub popup_key: Style,
    pub popup_desc: Style,
}

#[derive(Debug, Clone)]
pub struct NamedTheme {
    pub name: String,
    pub path: Option<PathBuf>,
    pub theme: Theme,
}

pub struct ThemeSet {
    pub themes: Vec<NamedTheme>,
    pub selected: usize,
}

#[derive(Default, Deserialize)]
struct AppConfig {
    theme: Option<String>,
    theme_catalog: Option<String>,
}

#[derive(Default, Deserialize)]
struct ThemeCatalog {
    #[serde(default)]
    themes: Vec<String>,
}

#[derive(Default, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    colors: BTreeMap<String, String>,
    #[serde(default)]
    ui: BTreeMap<String, String>,
}

pub fn load_themes(cli_theme: Option<&str>) -> Result<ThemeSet, String> {
    let home = home_dir();
    load_themes_at(&home, cli_theme)
}

fn load_themes_at(home: &Path, cli_theme: Option<&str>) -> Result<ThemeSet, String> {
    let config_path = home.join(".config/rview/config.toml");
    let config = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|contents| toml::from_str::<AppConfig>(&contents).ok())
        .unwrap_or_default();

    let mut themes = Vec::new();
    let mut loaded_paths = HashSet::new();
    if let Some(catalog_path) = config.theme_catalog.as_deref() {
        let catalog_path = expand_home(home, catalog_path);
        for path in load_catalog_paths(&catalog_path, home) {
            if let Some(theme) = load_theme_path(&path)
                && loaded_paths.insert(path.clone())
            {
                themes.push(theme);
            }
        }
    }

    let configured = cli_theme.or(config.theme.as_deref());
    let strict = cli_theme.is_some();
    let selected = configured
        .and_then(|value| select_theme(value, home, &mut themes, &mut loaded_paths))
        .or_else(|| (!themes.is_empty()).then_some(0));

    if strict && selected.is_none() {
        let available = themes
            .iter()
            .map(|theme| theme.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "theme '{value}' is not a readable file or configured catalog entry{}",
            if available.is_empty() {
                String::new()
            } else {
                format!("; available: {available}")
            },
            value = configured.unwrap_or_default(),
        ));
    }

    let selected = selected.unwrap_or_else(|| {
        themes.push(NamedTheme {
            name: "tokyo night moon fallback".to_string(),
            path: None,
            theme: Theme::fallback(),
        });
        0
    });

    Ok(ThemeSet { themes, selected })
}

fn select_theme(
    configured: &str,
    home: &Path,
    themes: &mut Vec<NamedTheme>,
    loaded_paths: &mut HashSet<PathBuf>,
) -> Option<usize> {
    let path = expand_home(home, configured);
    if path.is_file() {
        if let Some(index) = themes
            .iter()
            .position(|theme| theme.path.as_ref() == Some(&path))
        {
            return Some(index);
        }
        let theme = load_theme_path(&path)?;
        loaded_paths.insert(path);
        themes.push(theme);
        return Some(themes.len() - 1);
    }

    let requested = normalized_name(configured);
    themes.iter().position(|theme| {
        normalized_name(&theme.name) == requested
            || (requested == "catppuccin" && normalized_name(&theme.name) == "catppuccinmocha")
    })
}

fn load_catalog_paths(catalog_path: &Path, home: &Path) -> Vec<PathBuf> {
    let Ok(contents) = std::fs::read_to_string(catalog_path) else {
        return Vec::new();
    };
    let Ok(catalog) = toml::from_str::<ThemeCatalog>(&contents) else {
        return Vec::new();
    };
    catalog
        .themes
        .iter()
        .map(|path| expand_home(home, path))
        .collect()
}

fn load_theme_path(path: &Path) -> Option<NamedTheme> {
    let contents = std::fs::read_to_string(path).ok()?;
    let file = toml::from_str::<ThemeFile>(&contents).ok()?;
    let theme = Theme::from_file(&file)?;
    Some(NamedTheme {
        name: theme_name(path),
        path: Some(path.to_path_buf()),
        theme,
    })
}

impl Theme {
    fn from_file(file: &ThemeFile) -> Option<Self> {
        let background = resolve(file, &["background"], &["bg", "base"])?;
        let background_dark = resolve(
            file,
            &["background_dark", "popup_bg"],
            &["bg_dark", "mantle", "bg"],
        )
        .unwrap_or(background);
        let background_deep = resolve(
            file,
            &["background_deep"],
            &["bg_dark1", "crust", "bg_dark", "base"],
        )
        .unwrap_or(background_dark);
        let background_highlight = resolve(
            file,
            &["cursor_bg"],
            &["bg_highlight", "surface0", "surface1"],
        )
        .unwrap_or(background_dark);
        let text = resolve(file, &["text"], &["fg", "text"])?;
        let text_dim = resolve(
            file,
            &["text_dim"],
            &["comment", "fg_dim", "subtext0", "overlay0"],
        )
        .unwrap_or(text);
        let border =
            resolve(file, &["border"], &["fg_gutter", "surface1", "overlay0"]).unwrap_or(text_dim);
        let accent =
            resolve(file, &["accent"], &["magenta", "mauve", "purple", "pink"]).unwrap_or(text);
        let heading = resolve(file, &["heading"], &["blue", "sapphire"]).unwrap_or(accent);
        let key = resolve(file, &["key"], &["cyan", "sky"]).unwrap_or(heading);
        let green = resolve(file, &[], &["green", "teal"]).unwrap_or(accent);
        let search = resolve(file, &[], &["teal", "green1", "green"]).unwrap_or(green);
        let error = resolve(file, &["error"], &["red", "maroon"]).unwrap_or(accent);
        let cursor =
            resolve(file, &["picker_loading"], &["blue5", "cyan", "sky", "blue"]).unwrap_or(key);
        let popup_border =
            resolve(file, &["picker_border"], &["blue7", "lavender", "surface1"]).unwrap_or(accent);
        let popup_title = resolve(file, &[], &["yellow", "peach"]).unwrap_or(accent);

        Some(Self {
            background: Style::default().bg(background),
            border: Style::default().fg(border),
            border_selected: Style::default().fg(accent).add_modifier(Modifier::BOLD),
            border_marked: Style::default().fg(green).add_modifier(Modifier::BOLD),
            title: Style::default().fg(heading).add_modifier(Modifier::BOLD),
            label: Style::default().fg(text),
            label_selected: Style::default().fg(accent),
            status_bar: Style::default().fg(text).bg(background_dark),
            status_bar_error: Style::default().fg(background_deep).bg(error),
            status_bar_dim: Style::default().fg(text_dim).bg(background_dark),
            mode_normal: Style::default()
                .fg(background_deep)
                .bg(heading)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(background_deep)
                .bg(search)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(text).bg(background_highlight),
            search_cursor: Style::default().fg(background_deep).bg(cursor),
            popup_border: Style::default().fg(popup_border),
            popup_title: Style::default()
                .fg(popup_title)
                .add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(text),
            popup_key: Style::default().fg(key).add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(text_dim),
        })
    }

    pub fn fallback() -> Self {
        let bg = Color::Rgb(0x22, 0x24, 0x36);
        let bg_dark1 = Color::Rgb(0x19, 0x1b, 0x29);
        let fg = Color::Rgb(0xc8, 0xd3, 0xf5);
        let fg_dark = Color::Rgb(0x82, 0x8b, 0xb8);
        let bg_dark = Color::Rgb(0x1e, 0x20, 0x30);
        let bg_highlight = Color::Rgb(0x2f, 0x33, 0x4d);
        let blue = Color::Rgb(0x82, 0xaa, 0xff);
        let blue5 = Color::Rgb(0x89, 0xdd, 0xff);
        let yellow = Color::Rgb(0xff, 0xc7, 0x77);
        let red = Color::Rgb(0xff, 0x75, 0x7f);
        let green = Color::Rgb(0xc3, 0xe8, 0x8d);
        let teal = Color::Rgb(0x4f, 0xd6, 0xbe);
        let magenta = Color::Rgb(0xc0, 0x99, 0xff);
        let comment = Color::Rgb(0x63, 0x6d, 0xa6);
        let dark3 = Color::Rgb(0x54, 0x5c, 0x7e);

        Self {
            background: Style::default().bg(bg),
            border: Style::default().fg(dark3),
            border_selected: Style::default().fg(magenta).add_modifier(Modifier::BOLD),
            border_marked: Style::default().fg(green).add_modifier(Modifier::BOLD),
            title: Style::default().fg(blue).add_modifier(Modifier::BOLD),
            label: Style::default().fg(fg),
            label_selected: Style::default().fg(magenta),
            status_bar: Style::default().fg(fg).bg(bg_dark),
            status_bar_error: Style::default().fg(bg_dark1).bg(red),
            status_bar_dim: Style::default().fg(comment).bg(bg_dark),
            mode_normal: Style::default()
                .fg(bg_dark1)
                .bg(blue)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(bg_dark1)
                .bg(teal)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(fg).bg(bg_highlight),
            search_cursor: Style::default().fg(bg_dark1).bg(blue5),
            popup_border: Style::default().fg(magenta),
            popup_title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(fg),
            popup_key: Style::default().fg(blue).add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(fg_dark),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::fallback()
    }
}

fn resolve(file: &ThemeFile, ui_names: &[&str], color_names: &[&str]) -> Option<Color> {
    ui_names
        .iter()
        .filter_map(|name| file.ui.get(*name))
        .find_map(|value| resolve_value(value, &file.colors))
        .or_else(|| {
            color_names
                .iter()
                .filter_map(|name| file.colors.get(*name))
                .find_map(|value| parse_hex(value))
        })
}

fn resolve_value(value: &str, colors: &BTreeMap<String, String>) -> Option<Color> {
    parse_hex(value).or_else(|| colors.get(value).and_then(|value| parse_hex(value)))
}

fn parse_hex(value: &str) -> Option<Color> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    Some(Color::Rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn expand_home(home: &Path, configured_path: &str) -> PathBuf {
    configured_path
        .strip_prefix("~/")
        .map(|rest| home.join(rest))
        .unwrap_or_else(|| PathBuf::from(configured_path))
}

fn theme_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("theme")
        .replace('-', " ")
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const SHARED_THEME: &str = r##"
[colors]
bg = "#222436"
bg_dark = "#1e2030"
bg_dark1 = "#191b29"
bg_highlight = "#2f334d"
fg = "#c8d3f5"
comment = "#636da6"
fg_gutter = "#3b4261"
red = "#ff757f"
yellow = "#ffc777"
green = "#c3e88d"
blue = "#82aaff"
blue5 = "#89ddff"
magenta = "#c099ff"
teal = "#4fd6be"

[ui]
background = "bg"
background_dark = "bg_dark"
background_deep = "bg_dark1"
border = "fg_gutter"
accent = "magenta"
key = "blue5"
heading = "blue"
text = "fg"
text_dim = "comment"
cursor_bg = "bg_highlight"
error = "red"
"##;

    #[test]
    fn shared_theme_maps_rview_roles() {
        let file = toml::from_str::<ThemeFile>(SHARED_THEME).unwrap();
        let theme = Theme::from_file(&file).unwrap();

        assert_eq!(theme.background.bg, Some(Color::Rgb(0x22, 0x24, 0x36)));
        assert_eq!(theme.border_selected.fg, Some(Color::Rgb(0xc0, 0x99, 0xff)));
        assert_eq!(theme.title.fg, Some(Color::Rgb(0x82, 0xaa, 0xff)));
        assert_eq!(theme.mode_search.bg, Some(Color::Rgb(0x4f, 0xd6, 0xbe)));
        assert_eq!(theme.search_input.bg, Some(Color::Rgb(0x2f, 0x33, 0x4d)));
    }

    #[test]
    fn catalog_is_explicit_and_loading_does_not_rewrite_config() {
        let root = test_root();
        let config_dir = root.join(".config/rview");
        let themes_dir = root.join("themes");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&themes_dir).unwrap();
        let selected = themes_dir.join("synthetic-theme.toml");
        std::fs::write(&selected, SHARED_THEME).unwrap();
        std::fs::write(themes_dir.join("unlisted.toml"), SHARED_THEME).unwrap();
        let catalog = root.join("catalog.toml");
        std::fs::write(&catalog, format!("themes = [\"{}\"]\n", selected.display())).unwrap();
        let config_path = config_dir.join("config.toml");
        let config = format!(
            "theme = \"{}\"\ntheme_catalog = \"{}\"\n",
            selected.display(),
            catalog.display()
        );
        std::fs::write(&config_path, &config).unwrap();

        let set = load_themes_at(&root, None).unwrap();

        assert_eq!(set.themes.len(), 1);
        assert_eq!(set.themes[0].name, "synthetic theme");
        assert_eq!(set.selected, 0);
        assert_eq!(std::fs::read_to_string(config_path).unwrap(), config);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn test_root() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "rview-theme-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
