mod app;
mod encoder;
mod gallery;
mod image_list;
mod prefetch;
mod scanner;
mod search;
mod theme;
mod ui;

use app::{App, ViewMode};
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use image_list::SharedImageList;
use ratatui::prelude::*;
use rayon::prelude::*;
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

    let shared_list = SharedImageList::new();
    scanner::spawn(paths, shared_list.clone());

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let cell_px = query_cell_pixel_size();
    let mut app = App::new(theme, cell_px, shared_list);
    let result = run(&mut terminal, &mut app);

    encoder::kitty::delete_all()?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen)?;

    result?;

    if app.scan_complete && app.images.is_empty() {
        eprintln!("No image files found.");
        std::process::exit(1);
    }

    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        app.refresh_from_scanner();

        if app.scan_complete && app.images.is_empty() {
            return Ok(());
        }

        terminal.draw(|frame| ui::draw(frame, app))?;

        if app.needs_render && !app.images.is_empty() {
            match app.mode {
                ViewMode::Fullscreen => {
                    app.load_if_needed()?;
                    render_fullscreen_image(app)?;
                    app.prefetcher.kick(app.current, &app.images, app.image_rect, app.cell_px);
                }
                ViewMode::Gallery => {
                    render_gallery_images(app)?;
                }
            }
            app.needs_render = false;
            terminal.draw(|frame| ui::draw(frame, app))?;
        }

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if app.help_visible {
                    app.help_visible = false;
                    app.needs_render = true;
                } else {
                    match app.mode {
                        ViewMode::Fullscreen => match key.code {
                            KeyCode::Char('q') => return Ok(()),
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
                        ViewMode::Gallery => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
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
                                let prev_offset = app.gallery.scroll_offset;
                                app.gallery.move_left();
                                if app.gallery.scroll_offset != prev_offset {
                                    app.needs_render = true;
                                }
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                let prev_offset = app.gallery.scroll_offset;
                                app.gallery.move_right();
                                if app.gallery.scroll_offset != prev_offset {
                                    app.needs_render = true;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let prev_offset = app.gallery.scroll_offset;
                                app.gallery.move_up();
                                if app.gallery.scroll_offset != prev_offset {
                                    app.needs_render = true;
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let prev_offset = app.gallery.scroll_offset;
                                app.gallery.move_down();
                                if app.gallery.scroll_offset != prev_offset {
                                    app.needs_render = true;
                                }
                            }
                            _ => {}
                        },
                    }
                }
            },
            Event::Resize(_, _) => app.mark_dirty(),
            _ => {}
        }
    }
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
        encoder::kitty::encode_to(&mut out, img)?;
    }

    out.flush()
}

fn render_gallery_images(app: &mut App) -> io::Result<()> {
    let visible: Vec<(usize, usize)> = app.gallery.visible_items().collect();
    let cell_px = app.cell_px;

    let to_load: Vec<(usize, PathBuf, Rect)> = visible
        .iter()
        .filter_map(|&(vis_idx, img_idx)| {
            if app.thumb_cache.contains(img_idx) {
                return None;
            }
            let cell_rect = app.gallery.cell_rect(vis_idx);
            let inner = Rect {
                x: cell_rect.x + 1,
                y: cell_rect.y + 1,
                width: cell_rect.width.saturating_sub(2),
                height: cell_rect.height.saturating_sub(2),
            };
            Some((img_idx, app.images[img_idx].clone(), inner))
        })
        .collect();

    let decoded: Vec<(usize, image::RgbaImage)> = to_load
        .par_iter()
        .filter_map(|(img_idx, path, inner)| {
            app::load_and_resize(path, *inner, cell_px)
                .ok()
                .map(|img| (*img_idx, img))
        })
        .collect();

    for (img_idx, img) in decoded {
        app.thumb_cache.insert(img_idx, img);
    }

    let mut out = io::stdout().lock();
    encoder::kitty::delete_all_to(&mut out)?;

    for &(vis_idx, img_idx) in &visible {
        let cell_rect = app.gallery.cell_rect(vis_idx);
        if let Some((img, id)) = app.thumb_cache.peek(img_idx) {
            let inner_x = cell_rect.x + 1;
            let inner_y = cell_rect.y + 1;
            queue!(out, cursor::MoveTo(inner_x, inner_y))?;
            encoder::kitty::encode_with_opts(
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
