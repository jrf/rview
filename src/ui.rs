use crate::app::{App, ViewMode};
use crate::gallery::{CELL_HEIGHT_TOTAL, LABEL_ROWS, THUMB_COLS, THUMB_ROWS};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match app.mode {
        ViewMode::Gallery => draw_gallery(frame, app),
        ViewMode::Fullscreen => draw_fullscreen(frame, app),
        #[cfg(feature = "video")]
        ViewMode::Video => draw_video(frame, app),
    }
    if app.help_visible {
        draw_help_popup(frame, app);
    }
}

fn draw_fullscreen(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let area = frame.area();

    let filename = app
        .current_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let bottom_line = if let Some(ref err) = app.error {
        Line::from(Span::styled(format!(" Error: {err} "), theme.status_bar_error))
    } else {
        let dims = app
            .loaded
            .as_ref()
            .map(|img| format!(" {}x{}", img.width(), img.height()))
            .unwrap_or_default();
        Line::from(vec![
            Span::styled(
                format!(
                    " {} [{}/{}]{dims} ",
                    filename,
                    app.current + 1,
                    app.images.len(),
                ),
                theme.popup_text,
            ),
            Span::styled(
                " Esc: gallery  \u{2190}/\u{2192}: navigate  ?: help  q: quit ",
                theme.popup_desc,
            ),
        ])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Line::from(Span::styled(
            format!(" {} ", filename),
            theme.title,
        )))
        .title_bottom(bottom_line);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    app.image_rect = inner;
}

#[cfg(feature = "video")]
fn draw_video(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let area = frame.area();

    let filename = app
        .current_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (status, pts, duration, fps) = match &app.video {
        Some(v) => {
            let status = if v.playing { "\u{25b6}" } else { "\u{23f8}" };
            (status, v.current_pts, v.duration, v.fps)
        }
        None => ("\u{25b6}", 0.0, 0.0, 0.0),
    };

    let fmt_time = |secs: f64| -> String {
        let s = secs as u64;
        format!("{:02}:{:02}", s / 60, s % 60)
    };

    let bottom_line = if let Some(ref err) = app.error {
        Line::from(Span::styled(format!(" Error: {err} "), theme.status_bar_error))
    } else {
        Line::from(vec![
            Span::styled(
                format!(
                    " {} {} {}/{} [{:.0}fps] [{}/{}] ",
                    status,
                    filename,
                    fmt_time(pts),
                    fmt_time(duration),
                    fps,
                    app.current + 1,
                    app.images.len(),
                ),
                theme.popup_text,
            ),
            Span::styled(
                " Space: pause  Esc: gallery  \u{2190}/\u{2192}: navigate  q: quit ",
                theme.popup_desc,
            ),
        ])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Line::from(Span::styled(
            format!(" {} ", filename),
            theme.title,
        )))
        .title_bottom(bottom_line);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    app.image_rect = inner;
}

fn draw_gallery(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let area = frame.area();

    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
    let search_rect = chunks[0];
    let gallery_area = chunks[1];

    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Line::from(Span::styled(" Search ", theme.title)));

    let search_inner = search_block.inner(search_rect);
    frame.render_widget(search_block, search_rect);

    if app.gallery.search_active {
        let match_count = app.gallery.filtered_indices.len();
        let total = app.images.len();
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("/{}\u{2588}", app.gallery.search_query),
                    theme.search_input,
                ),
                Span::styled(
                    format!("  {match_count}/{total} matches"),
                    theme.popup_desc,
                ),
            ])),
            search_inner,
        );
    } else {
        frame.render_widget(Paragraph::new(""), search_inner);
    }

    let filtered_count = app.gallery.filtered_indices.len();
    let total = app.images.len();
    let scanning = if app.is_scanning() { ", scanning\u{2026}" } else { "" };
    let gallery_title = if app.gallery.search_query.is_empty() {
        format!(" Gallery [{total} images{scanning}] ")
    } else {
        format!(" Gallery [{filtered_count}/{total} matches{scanning}] ")
    };

    let selected_name = app
        .gallery
        .selected_index()
        .and_then(|i| app.images[i].file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let hint = if app.gallery.search_active {
        " Esc: cancel  Enter: confirm "
    } else {
        " Enter: open  hjkl: navigate  /: search  ?: help  q: quit "
    };

    let bottom_line = if filtered_count == 0 {
        Line::from(Span::styled(hint, theme.popup_desc))
    } else {
        Line::from(vec![
            Span::styled(
                format!(
                    " {} [{}/{}] ",
                    selected_name,
                    app.gallery.cursor + 1,
                    filtered_count,
                ),
                theme.popup_text,
            ),
            Span::styled(hint, theme.popup_desc),
        ])
    };

    let gallery_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Line::from(Span::styled(gallery_title, theme.title)))
        .title_bottom(bottom_line);

    let grid_rect = gallery_block.inner(gallery_area);
    frame.render_widget(gallery_block, gallery_area);

    app.gallery.update_grid(grid_rect);

    for (vis_idx, img_idx) in app.gallery.visible_items().collect::<Vec<_>>() {
        let col = vis_idx % app.gallery.grid_cols;
        let row = vis_idx / app.gallery.grid_cols;

        let cell_x = grid_rect.x + (col as u16) * THUMB_COLS;
        let cell_y = grid_rect.y + (row as u16) * CELL_HEIGHT_TOTAL;

        let is_selected =
            vis_idx + app.gallery.scroll_offset * app.gallery.grid_cols == app.gallery.cursor;

        let border_style = if is_selected {
            theme.border_selected
        } else {
            theme.border
        };

        let thumb_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

        let block_rect = Rect {
            x: cell_x,
            y: cell_y,
            width: THUMB_COLS.min(grid_rect.right().saturating_sub(cell_x)),
            height: THUMB_ROWS.min(grid_rect.bottom().saturating_sub(cell_y)),
        };
        frame.render_widget(thumb_block, block_rect);

        let filename = app.images[img_idx]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let max_label = THUMB_COLS as usize - 2;
        let filename = if filename.len() > max_label {
            format!("{}\u{2026}", &filename[..max_label - 1])
        } else {
            filename
        };

        let label_y = cell_y + THUMB_ROWS;
        if label_y < grid_rect.bottom() {
            let label_rect = Rect {
                x: cell_x,
                y: label_y,
                width: THUMB_COLS.min(grid_rect.right().saturating_sub(cell_x)),
                height: LABEL_ROWS,
            };
            let label_style = if is_selected {
                theme.label_selected
            } else {
                theme.label
            };
            frame.render_widget(
                Paragraph::new(filename)
                    .style(label_style)
                    .alignment(Alignment::Center),
                label_rect,
            );
        }
    }
}

fn draw_help_popup(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();

    let help_lines = [
        ("", "Gallery"),
        ("h/j/k/l", "Navigate grid"),
        ("g / G", "Jump to first / last"),
        ("Home / End", "Jump to first / last"),
        ("Enter", "Open fullscreen"),
        ("/", "Search"),
        ("?", "Toggle help"),
        ("q / Esc", "Quit"),
        ("", ""),
        ("", "Search"),
        ("<type>", "Filter by filename"),
        ("Esc", "Cancel search"),
        ("Enter", "Confirm filter"),
        ("Backspace", "Delete character"),
        ("", ""),
        ("", "Fullscreen"),
        ("\u{2190} / \u{2192}", "Previous / next image"),
        ("h / l", "Previous / next image"),
        ("Home / End", "Jump to first / last"),
        ("Esc", "Back to gallery"),
        ("?", "Toggle help"),
        ("q", "Quit"),
    ];

    let height = (help_lines.len() as u16 + 4).min(area.height.saturating_sub(2));
    let width = 48.min(area.width.saturating_sub(4));
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.popup_border)
        .title(Line::from(Span::styled(" Help ", theme.popup_title)));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    let key_width = 12;
    let lines: Vec<Line> = help_lines
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() {
                Line::from(Span::styled(format!(" {desc}"), theme.popup_title))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(" {:>width$}  ", key, width = key_width),
                        theme.popup_key,
                    ),
                    Span::styled(*desc, theme.popup_desc),
                ])
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " press any key to close",
            theme.popup_desc,
        ))),
        chunks[1],
    );
}
