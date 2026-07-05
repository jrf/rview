mod app;
mod encoder;
mod gallery;
mod image_list;
mod picker;
mod prefetch;
mod scanner;
mod search;
mod theme;
mod ui;
#[cfg(feature = "video")]
mod video;

use app::{App, ViewMode};
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use image_list::SharedImageList;
use ratatui::prelude::*;
use std::io::{self, stdout, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "rview", about = "A fast terminal image viewer")]
struct Cli {
    /// Image file(s) or directory to display [default: .]
    files: Vec<PathBuf>,

    /// Color theme (tokyonight, dark, light, catppuccin, nord)
    #[arg(short, long, default_value = "tokyonight")]
    theme: String,

    /// Number of threads for image decoding [default: all cores]
    #[arg(short = 'j', long)]
    threads: Option<usize>,
}

fn main() -> io::Result<()> {
    #[cfg(feature = "video")]
    ffmpeg::init().expect("failed to initialize ffmpeg");

    let cli = Cli::parse();
    let paths = if cli.files.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        cli.files
    };

    if let Some(n) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }

    for p in &paths {
        if !p.is_dir() && !p.exists() {
            eprintln!("{}: file not found", p.display());
            std::process::exit(1);
        }
    }

    let theme = theme::Theme::by_name(&cli.theme).unwrap_or_else(|| {
        eprintln!(
            "Unknown theme '{}'. Available: {}",
            cli.theme,
            theme::Theme::names().join(", ")
        );
        std::process::exit(1);
    });

    let initial_dir = initial_dir_from_paths(&paths);

    let shared_list = SharedImageList::new();
    scanner::spawn(paths, shared_list.clone());

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let cell_px = query_cell_pixel_size();
    let mut app = App::new(theme, cell_px, shared_list);
    app.initial_dir = Some(initial_dir);
    let result = run(&mut terminal, &mut app);

    encoder::kitty::delete_all()?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen)?;

    result?;

    Ok(())
}

fn initial_dir_from_paths(paths: &[PathBuf]) -> PathBuf {
    for p in paths {
        if p.is_dir() {
            return p.clone();
        }
    }
    if let Some(first) = paths.first() {
        if let Some(parent) = first.parent() {
            if !parent.as_os_str().is_empty() {
                return parent.to_path_buf();
            }
        }
    }
    PathBuf::from(".")
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    let mut pending_emits: Vec<usize> = Vec::new();

    loop {
        // 1. Drain all buffered input first so navigation is never blocked
        while event::poll(Duration::ZERO)? {
            if handle_event(app, event::read()?)? {
                return Ok(());
            }
        }

        // 2. Poll background tasks
        app.refresh_from_scanner();
        app.poll_filter();
        app.poll_fullscreen();
        app.prefetcher.poll();
        pending_emits.extend(app.poll_thumbnails());

        #[cfg(feature = "video")]
        if let Some(ref mut v) = app.video {
            if v.poll_frame() {
                app.needs_render = true;
            }
        }

        if app.scan_complete
            && app.images.is_empty()
            && app.mode != ViewMode::Picker
        {
            app.open_picker();
        }

        // 3. Draw UI chrome (always fast — ratatui only)
        terminal.draw(|frame| ui::draw(frame, app))?;

        // 4. Render Kitty images
        if app.mode == ViewMode::Picker {
            app.needs_render = false;
            pending_emits.clear();
        } else if app.needs_render && !app.images.is_empty() {
            #[cfg(feature = "video")]
            let is_video = matches!(app.mode, ViewMode::Video);
            #[cfg(not(feature = "video"))]
            let is_video = false;

            match app.mode {
                ViewMode::Fullscreen => {
                    app.load_if_needed();
                    render_fullscreen_image(app)?;
                    app.prefetcher.set_target_hint(app.image_rect, app.cell_px);
                    app.prefetcher.kick(app.current, &app.images);
                }
                ViewMode::Gallery => {
                    render_gallery_images(app)?;
                }
                ViewMode::Picker => {}
                #[cfg(feature = "video")]
                ViewMode::Video => {
                    app.open_video_if_needed();
                    render_video_frame(app)?;
                }
            }
            app.needs_render = false;
            pending_emits.clear();
            if !is_video {
                terminal.draw(|frame| ui::draw(frame, app))?;
            }
        } else if !pending_emits.is_empty() && app.mode == ViewMode::Gallery {
            let count = pending_emits.len().min(4);
            let batch: Vec<usize> = pending_emits.drain(..count).collect();
            emit_new_thumbnails(app, &batch)?;
        }

        // 5. Wait for next event (shorter timeout when thumbnails are pending)
        let timeout = {
            #[cfg(feature = "video")]
            if let Some(ref v) = app.video {
                v.time_until_next_frame()
            } else if pending_emits.is_empty() {
                Duration::from_millis(250)
            } else {
                Duration::from_millis(50)
            }
            #[cfg(not(feature = "video"))]
            if pending_emits.is_empty() {
                Duration::from_millis(250)
            } else {
                Duration::from_millis(50)
            }
        };
        event::poll(timeout)?;
    }
}

fn handle_event(app: &mut App, event: Event) -> io::Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if app.help_visible {
                app.help_visible = false;
                app.needs_render = true;
            } else if app.pending_delete.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        encoder::kitty::delete_all()?;
                        app.confirm_delete();
                        if app.scan_complete && app.images.is_empty() {
                            return Ok(true);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        app.cancel_delete();
                    }
                    _ => {}
                }
            } else {
                match app.mode {
                    ViewMode::Fullscreen => match key.code {
                        KeyCode::Char('q') => return Ok(true),
                        KeyCode::Char('?') => {
                            encoder::kitty::delete_all()?;
                            app.help_visible = true;
                        }
                        KeyCode::Esc => {
                            encoder::kitty::delete_all()?;
                            app.enter_gallery();
                        }
                        KeyCode::Left | KeyCode::Char('h') => app.prev(),
                        KeyCode::Right | KeyCode::Char('l') => app.next(),
                        KeyCode::Home => {
                            encoder::kitty::delete_all()?;
                            app.first();
                        }
                        KeyCode::End => {
                            encoder::kitty::delete_all()?;
                            app.last();
                        }
                        KeyCode::Char('d') => {
                            app.begin_delete();
                        }
                        _ => {}
                    },
                    #[cfg(feature = "video")]
                    ViewMode::Video => match key.code {
                        KeyCode::Char('q') => return Ok(true),
                        KeyCode::Char(' ') => {
                            if let Some(ref mut v) = app.video {
                                v.toggle_pause();
                                app.needs_render = true;
                            }
                        }
                        KeyCode::Esc => {
                            encoder::kitty::delete_all()?;
                            app.exit_video();
                        }
                        KeyCode::Char('?') => {
                            encoder::kitty::delete_all()?;
                            app.help_visible = true;
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            encoder::kitty::delete_all()?;
                            app.prev();
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            encoder::kitty::delete_all()?;
                            app.next();
                        }
                        KeyCode::Home => {
                            encoder::kitty::delete_all()?;
                            app.first();
                        }
                        KeyCode::End => {
                            encoder::kitty::delete_all()?;
                            app.last();
                        }
                        KeyCode::Char('d') => {
                            app.begin_delete();
                        }
                        _ => {}
                    },
                    ViewMode::Gallery if app.gallery.search_active => match key.code {
                        KeyCode::Esc => {
                            app.gallery.search_active = false;
                            app.gallery.search_query.clear();
                            app.update_filter();
                        }
                        KeyCode::Enter => {
                            app.gallery.search_active = false;
                            app.update_filter();
                        }
                        KeyCode::Backspace => {
                            app.gallery.search_query.pop();
                            app.update_filter();
                        }
                        KeyCode::Char(c) => {
                            app.gallery.search_query.push(c);
                            app.update_filter();
                        }
                        _ => {}
                    },
                    ViewMode::Gallery => {
                        let prev_offset = app.gallery.scroll_offset;
                        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
                            KeyCode::Char('?') => {
                                encoder::kitty::delete_all()?;
                                app.help_visible = true;
                            }
                            KeyCode::Char('/') => {
                                app.gallery.search_active = true;
                            }
                            KeyCode::Enter => {
                                encoder::kitty::delete_all()?;
                                app.enter_fullscreen_selected();
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
                                app.gallery.move_left();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                app.gallery.move_right();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.gallery.move_up();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.gallery.move_down();
                                app.pre_decode_hovered();
                            }
                            KeyCode::PageUp | KeyCode::Char('b') if ctrl || matches!(key.code, KeyCode::PageUp) => {
                                app.gallery.move_page_up();
                                app.pre_decode_hovered();
                            }
                            KeyCode::PageDown | KeyCode::Char('f') if ctrl || matches!(key.code, KeyCode::PageDown) => {
                                app.gallery.move_page_down();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Char('g') | KeyCode::Home => {
                                app.gallery.move_to_first();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Char('G') | KeyCode::End => {
                                app.gallery.move_to_last();
                                app.pre_decode_hovered();
                            }
                            KeyCode::Char(' ') => {
                                app.toggle_selection_at_cursor();
                            }
                            KeyCode::Char('a') => {
                                app.select_all_filtered();
                            }
                            KeyCode::Char('A') => {
                                app.clear_selection();
                            }
                            KeyCode::Char('d') => {
                                encoder::kitty::delete_all()?;
                                app.begin_delete();
                            }
                            KeyCode::Char('o') => {
                                encoder::kitty::delete_all()?;
                                app.open_picker();
                            }
                            _ => {}
                        }
                        if app.gallery.scroll_offset != prev_offset {
                            app.needs_render = true;
                        }
                    }
                    ViewMode::Picker => {
                        if handle_picker_key(app, key.code)? {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Event::Resize(_, _) => app.mark_dirty(),
        _ => {}
    }
    Ok(false)
}

fn handle_picker_key(app: &mut App, code: KeyCode) -> io::Result<bool> {
    let filter_active = app
        .picker
        .as_ref()
        .is_some_and(|p| p.filter_active);

    if filter_active {
        let Some(picker) = app.picker.as_mut() else {
            return Ok(false);
        };
        match code {
            KeyCode::Esc => {
                picker.filter_active = false;
                picker.filter.clear();
                picker.rebuild_filter();
                app.needs_render = true;
            }
            KeyCode::Enter => {
                picker.filter_active = false;
                app.needs_render = true;
            }
            KeyCode::Backspace => {
                picker.filter.pop();
                picker.rebuild_filter();
                app.needs_render = true;
            }
            KeyCode::Char(c) => {
                picker.filter.push(c);
                picker.rebuild_filter();
                app.needs_render = true;
            }
            _ => {}
        }
        return Ok(false);
    }

    match code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Esc => {
            if !app.images.is_empty() {
                app.close_picker();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(p) = app.picker.as_mut() {
                p.move_up();
                app.needs_render = true;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(p) = app.picker.as_mut() {
                p.move_down();
                app.needs_render = true;
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if let Some(p) = app.picker.as_mut() {
                p.move_first();
                app.needs_render = true;
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if let Some(p) = app.picker.as_mut() {
                p.move_last();
                app.needs_render = true;
            }
        }
        KeyCode::PageUp => {
            if let Some(p) = app.picker.as_mut() {
                p.move_page_up(10);
                app.needs_render = true;
            }
        }
        KeyCode::PageDown => {
            if let Some(p) = app.picker.as_mut() {
                p.move_page_down(10);
                app.needs_render = true;
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            let target = app.picker.as_mut().and_then(|p| p.ascend());
            if let Some(t) = target {
                app.switch_to_dir(t);
            }
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
            let target = app.picker.as_mut().and_then(|p| {
                let sel = p.selected()?;
                if sel.is_parent {
                    return p.ascend();
                }
                p.enter_selected()
            });
            if let Some(t) = target {
                app.switch_to_dir(t);
            }
        }
        KeyCode::Char('/') => {
            if let Some(p) = app.picker.as_mut() {
                p.filter_active = true;
                p.filter.clear();
                p.rebuild_filter();
                app.needs_render = true;
            }
        }
        KeyCode::Char('?') => {
            app.help_visible = true;
            app.needs_render = true;
        }
        _ => {}
    }
    Ok(false)
}

fn query_cell_pixel_size() -> (u32, u32) {
    crossterm::terminal::window_size()
        .map(|ws| {
            let w = if ws.columns > 0 {
                ws.width as u32 / ws.columns as u32
            } else {
                8
            };
            let h = if ws.rows > 0 {
                ws.height as u32 / ws.rows as u32
            } else {
                16
            };
            (w.max(1), h.max(1))
        })
        .unwrap_or((8, 16))
}

fn render_fullscreen_image(app: &App) -> io::Result<()> {
    let mut out = io::stdout().lock();
    encoder::kitty::delete_all_to(&mut out)?;

    if let Some(ref img) = app.loaded {
        let (cpw, cph) = app.cell_px;
        let img_cols = (img.width() + cpw - 1) / cpw;
        let img_rows = (img.height() + cph - 1) / cph;
        let offset_x =
            app.image_rect.x + (app.image_rect.width.saturating_sub(img_cols as u16)) / 2;
        let offset_y =
            app.image_rect.y + (app.image_rect.height.saturating_sub(img_rows as u16)) / 2;

        queue!(out, cursor::MoveTo(offset_x, offset_y))?;
        encoder::kitty::encode_png_to(
            &mut out,
            img,
            &encoder::kitty::DisplayOpts { id: None, cols: None, rows: None },
        )?;
    }

    out.flush()
}

#[cfg(feature = "video")]
const VIDEO_IMAGE_ID: u32 = 900;

#[cfg(feature = "video")]
fn render_video_frame(app: &App) -> io::Result<()> {
    use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};

    let mut out = io::stdout().lock();

    if let Some(ref v) = app.video {
        if let Some(ref img) = v.current_frame {
            let (cpw, cph) = app.cell_px;
            let img_cols = (img.width() + cpw - 1) / cpw;
            let img_rows = (img.height() + cph - 1) / cph;
            let offset_x =
                app.image_rect.x + (app.image_rect.width.saturating_sub(img_cols as u16)) / 2;
            let offset_y =
                app.image_rect.y + (app.image_rect.height.saturating_sub(img_rows as u16)) / 2;

            queue!(out, BeginSynchronizedUpdate)?;
            queue!(out, cursor::MoveTo(offset_x, offset_y))?;
            encoder::kitty::encode_png_to(
                &mut out,
                img,
                &encoder::kitty::DisplayOpts { id: Some(VIDEO_IMAGE_ID), cols: None, rows: None },
            )?;
            queue!(out, EndSynchronizedUpdate)?;
        }
    }

    out.flush()
}

fn emit_new_thumbnails(app: &App, new_indices: &[usize]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    for (vis_idx, img_idx) in app.gallery.visible_items() {
        if !new_indices.contains(&img_idx) {
            continue;
        }
        let cell_rect = app.gallery.cell_rect(vis_idx);
        if let Some((img, id)) = app.thumb_cache.peek(img_idx) {
            queue!(out, cursor::MoveTo(cell_rect.x + 1, cell_rect.y + 1))?;
            encoder::kitty::encode_png_to(
                &mut out,
                img,
                &encoder::kitty::DisplayOpts {
                    id: Some(id),
                    cols: None,
                    rows: None,
                },
            )?;
        }
    }
    out.flush()
}

fn render_gallery_images(app: &mut App) -> io::Result<()> {
    let visible: Vec<(usize, usize)> = app.gallery.visible_items().collect();

    for &(vis_idx, img_idx) in &visible {
        let cell_rect = app.gallery.cell_rect(vis_idx);
        let inner = Rect {
            x: cell_rect.x + 1,
            y: cell_rect.y + 1,
            width: cell_rect.width.saturating_sub(2),
            height: cell_rect.height.saturating_sub(2),
        };
        let path = app.images[img_idx].clone();
        app.spawn_thumb_decode(img_idx, path, inner);
    }

    let mut out = io::stdout().lock();
    encoder::kitty::delete_all_to(&mut out)?;

    for &(vis_idx, img_idx) in &visible {
        let cell_rect = app.gallery.cell_rect(vis_idx);
        if let Some((img, id)) = app.thumb_cache.peek(img_idx) {
            let inner_x = cell_rect.x + 1;
            let inner_y = cell_rect.y + 1;
            queue!(out, cursor::MoveTo(inner_x, inner_y))?;
            encoder::kitty::encode_png_to(
                &mut out,
                img,
                &encoder::kitty::DisplayOpts {
                    id: Some(id),
                    cols: None,
                    rows: None,
                },
            )?;
        }
    }

    out.flush()
}
