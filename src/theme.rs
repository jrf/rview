use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Theme {
    // Canvas
    pub background: Style,

    // Borders
    pub border: Style,
    pub border_selected: Style,
    pub border_marked: Style,
    pub title: Style,

    // Gallery
    pub label: Style,
    pub label_selected: Style,

    // Status bar
    pub status_bar: Style,
    pub status_bar_error: Style,
    pub status_bar_dim: Style,
    pub mode_normal: Style,
    pub mode_search: Style,

    // Search
    pub search_input: Style,
    pub search_cursor: Style,

    // Popup
    pub popup_border: Style,
    pub popup_title: Style,
    pub popup_text: Style,
    pub popup_key: Style,
    pub popup_desc: Style,
}

impl Theme {
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "tokyonight" => Some(Self::tokyonight()),
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            "catppuccin" => Some(Self::catppuccin()),
            "nord" => Some(Self::nord()),
            _ => None,
        }
    }

    pub fn names() -> &'static [&'static str] {
        &["tokyonight", "dark", "light", "catppuccin", "nord"]
    }

    pub fn tokyonight() -> Self {
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
        let green1 = Color::Rgb(0x4f, 0xd6, 0xbe);
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
                .bg(green1)
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

    pub fn dark() -> Self {
        Self {
            background: Style::default(),
            border: Style::default().fg(Color::DarkGray),
            border_selected: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            border_marked: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            title: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            label: Style::default().fg(Color::White),
            label_selected: Style::default().fg(Color::Yellow),
            status_bar: Style::default().fg(Color::Black).bg(Color::White),
            status_bar_error: Style::default().fg(Color::White).bg(Color::Red),
            status_bar_dim: Style::default().fg(Color::DarkGray).bg(Color::White),
            mode_normal: Style::default()
                .fg(Color::Black)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(Color::White).bg(Color::DarkGray),
            search_cursor: Style::default().fg(Color::Black).bg(Color::White),
            popup_border: Style::default().fg(Color::Yellow),
            popup_title: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(Color::White),
            popup_key: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(Color::Gray),
        }
    }

    pub fn light() -> Self {
        Self {
            background: Style::default(),
            border: Style::default().fg(Color::Gray),
            border_selected: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            border_marked: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            title: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            label: Style::default().fg(Color::Black),
            label_selected: Style::default().fg(Color::Blue),
            status_bar: Style::default().fg(Color::White).bg(Color::DarkGray),
            status_bar_error: Style::default().fg(Color::White).bg(Color::Red),
            status_bar_dim: Style::default().fg(Color::Gray).bg(Color::DarkGray),
            mode_normal: Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(Color::White)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(Color::Black).bg(Color::Gray),
            search_cursor: Style::default().fg(Color::White).bg(Color::Black),
            popup_border: Style::default().fg(Color::Blue),
            popup_title: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(Color::Black),
            popup_key: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(Color::DarkGray),
        }
    }

    pub fn catppuccin() -> Self {
        let mauve = Color::Rgb(203, 166, 247);
        let text = Color::Rgb(205, 214, 244);
        let subtext = Color::Rgb(166, 173, 200);
        let overlay = Color::Rgb(108, 112, 134);
        let surface0 = Color::Rgb(49, 50, 68);
        let base = Color::Rgb(30, 30, 46);
        let red = Color::Rgb(243, 139, 168);
        let green = Color::Rgb(166, 227, 161);
        let yellow = Color::Rgb(249, 226, 175);

        Self {
            background: Style::default(),
            border: Style::default().fg(overlay),
            border_selected: Style::default().fg(mauve).add_modifier(Modifier::BOLD),
            border_marked: Style::default().fg(green).add_modifier(Modifier::BOLD),
            title: Style::default().fg(mauve).add_modifier(Modifier::BOLD),
            label: Style::default().fg(subtext),
            label_selected: Style::default().fg(mauve),
            status_bar: Style::default().fg(text).bg(surface0),
            status_bar_error: Style::default().fg(base).bg(red),
            status_bar_dim: Style::default().fg(overlay).bg(surface0),
            mode_normal: Style::default()
                .fg(base)
                .bg(mauve)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(base)
                .bg(green)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(text).bg(surface0),
            search_cursor: Style::default().fg(base).bg(mauve),
            popup_border: Style::default().fg(mauve),
            popup_title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(text),
            popup_key: Style::default().fg(mauve).add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(subtext),
        }
    }

    pub fn nord() -> Self {
        let frost_blue = Color::Rgb(136, 192, 208);
        let snow0 = Color::Rgb(216, 222, 233);
        let snow1 = Color::Rgb(229, 233, 240);
        let polar3 = Color::Rgb(67, 76, 94);
        let polar2 = Color::Rgb(59, 66, 82);
        let polar0 = Color::Rgb(46, 52, 64);
        let aurora_red = Color::Rgb(191, 97, 106);
        let aurora_green = Color::Rgb(163, 190, 140);
        let aurora_yellow = Color::Rgb(235, 203, 139);

        Self {
            background: Style::default(),
            border: Style::default().fg(polar3),
            border_selected: Style::default().fg(frost_blue).add_modifier(Modifier::BOLD),
            border_marked: Style::default()
                .fg(aurora_green)
                .add_modifier(Modifier::BOLD),
            title: Style::default().fg(frost_blue).add_modifier(Modifier::BOLD),
            label: Style::default().fg(snow0),
            label_selected: Style::default().fg(frost_blue),
            status_bar: Style::default().fg(snow1).bg(polar2),
            status_bar_error: Style::default().fg(polar0).bg(aurora_red),
            status_bar_dim: Style::default().fg(polar3).bg(polar2),
            mode_normal: Style::default()
                .fg(polar0)
                .bg(frost_blue)
                .add_modifier(Modifier::BOLD),
            mode_search: Style::default()
                .fg(polar0)
                .bg(aurora_green)
                .add_modifier(Modifier::BOLD),
            search_input: Style::default().fg(snow0).bg(polar2),
            search_cursor: Style::default().fg(polar0).bg(frost_blue),
            popup_border: Style::default().fg(frost_blue),
            popup_title: Style::default()
                .fg(aurora_yellow)
                .add_modifier(Modifier::BOLD),
            popup_text: Style::default().fg(snow0),
            popup_key: Style::default().fg(frost_blue).add_modifier(Modifier::BOLD),
            popup_desc: Style::default().fg(snow0),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::tokyonight()
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;
    use ratatui::style::Color;

    #[test]
    fn tokyonight_uses_the_moon_palette() {
        let theme = Theme::tokyonight();

        assert_eq!(theme.background.bg, Some(Color::Rgb(0x22, 0x24, 0x36)));
        assert_eq!(theme.border.fg, Some(Color::Rgb(0x54, 0x5c, 0x7e)));
        assert_eq!(theme.border_selected.fg, Some(Color::Rgb(0xc0, 0x99, 0xff)));
        assert_eq!(theme.title.fg, Some(Color::Rgb(0x82, 0xaa, 0xff)));
        assert_eq!(theme.label.fg, Some(Color::Rgb(0xc8, 0xd3, 0xf5)));
        assert_eq!(theme.label_selected.fg, Some(Color::Rgb(0xc0, 0x99, 0xff)));
        assert_eq!(theme.status_bar.bg, Some(Color::Rgb(0x1e, 0x20, 0x30)));
        assert_eq!(
            theme.status_bar_error.bg,
            Some(Color::Rgb(0xff, 0x75, 0x7f))
        );
        assert_eq!(theme.mode_search.bg, Some(Color::Rgb(0x4f, 0xd6, 0xbe)));
        assert_eq!(theme.search_input.bg, Some(Color::Rgb(0x2f, 0x33, 0x4d)));
        assert_eq!(theme.popup_border.fg, Some(Color::Rgb(0xc0, 0x99, 0xff)));
        assert_eq!(theme.popup_title.fg, Some(Color::Rgb(0xff, 0xc7, 0x77)));
    }
}
