use crate::app::App;
use ratatui::{
    prelude::*,
    widgets::Paragraph,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    app.image_rect = chunks[0];

    let filename = app
        .current_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let status_text = if let Some(ref err) = app.error {
        format!(" Error: {err}")
    } else {
        let dims = app
            .loaded
            .as_ref()
            .map(|img| format!("{}x{}", img.width(), img.height()))
            .unwrap_or_default();
        format!(
            " {filename} [{}/{}] {dims}  q: quit  \u{2190}/\u{2192}: navigate",
            app.current + 1,
            app.images.len(),
        )
    };

    let style = if app.error.is_some() {
        Style::default().fg(Color::White).bg(Color::Red)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    };

    frame.render_widget(Paragraph::new(status_text).style(style), chunks[1]);
}
