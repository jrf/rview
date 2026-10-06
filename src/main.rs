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
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute, queue,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use encoder::GraphicsBackend;
use image_list::SharedImageList;
use ratatui::prelude::*;
use std::io::{self, BufWriter, Write, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "rview", about = "A fast terminal image viewer")]
struct Cli {
    /// Image file(s) or directory to display [default: .]
    files: Vec<PathBuf>,

    /// Explicit theme file path or configured catalog name
    #[arg(short, long)]
    theme: Option<String>,

    /// Number of threads for image decoding [default: all cores]
    #[arg(short = 'j', long)]
    threads: Option<usize>,
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    restored: bool,
}

impl TerminalSession {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut output = stdout();
        if let Err(error) = execute!(
            output,
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        ) {
            let _ = disable_raw_mode();
            let _ = execute!(
                output,
                DisableMouseCapture,
                cursor::Show,
                LeaveAlternateScreen
            );
            return Err(error);
        }

        match Terminal::new(CrosstermBackend::new(output)) {
            Ok(terminal) => Ok(Self {
                terminal,
                restored: false,
            }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(
                    stdout(),
                    DisableMouseCapture,
                    cursor::Show,
                    LeaveAlternateScreen
                );
                Err(error)
            }
        }
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;

        let mut first_error = encoder::KittyBackend.delete_all().err();
        {
            let mut out = io::stdout().lock();
            for id in encoder::FULLSCREEN_IMAGE_IDS {
                if let Err(error) = encoder::KittyBackend.delete_image_to(&mut out, id)
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
            if let Err(error) = out.flush()
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Err(error) = disable_raw_mode()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            cursor::Show,
            LeaveAlternateScreen
        ) && first_error.is_none()
        {
            first_error = Some(error);
        }

        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn main() -> io::Result<()> {
    #[cfg(feature = "video")]
    ffmpeg::init()
        .map_err(|error| io::Error::other(format!("failed to initialize ffmpeg: {error}")))?;

    let cli = Cli::parse();
    let paths = if cli.files.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        cli.files
    };

    if let Some(n) = cli.threads {
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--threads must be greater than zero",
            ));
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .map_err(|error| {
                io::Error::other(format!("failed to configure thread pool: {error}"))
            })?;
    }

    for p in &paths {
        if !p.is_dir() && !p.exists() {
            eprintln!("{}: file not found", p.display());
            std::process::exit(1);
        }
        if p.is_file() && !scanner::is_supported(p) {
            eprintln!("{}: unsupported media format", p.display());
            std::process::exit(1);
        }
    }

    let theme_set = theme::load_themes(cli.theme.as_deref())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let theme = theme_set.themes[theme_set.selected].theme.clone();

    let initial_dir = initial_dir_from_paths(&paths);

    let shared_list = SharedImageList::new();
    scanner::spawn(paths, shared_list.clone());

    let mut session = TerminalSession::enter()?;

    let cell_px = query_cell_pixel_size();
    let mut app = App::new(theme, cell_px, shared_list);
    app.install_themes(theme_set.themes, theme_set.selected);
    app.initial_dir = Some(initial_dir);
    let run_result = run(session.terminal_mut(), &mut app);
    let restore_result = session.restore();
    run_result.and(restore_result)
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

#[derive(Clone, Copy)]
struct FullscreenImage {
    id: u32,
    revision: u64,
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    let mut pending_emits: Vec<usize> = Vec::new();
    let mut mouse_drag = None;
    let mut fullscreen_view = None;
    let mut displayed_fullscreen = None;
    let mut fullscreen_image = None;

    loop {
        // Bound each drain so a continuous mouse stream cannot starve workers.
        let input_deadline = Instant::now() + Duration::from_millis(4);
        for _ in 0..64 {
            if Instant::now() >= input_deadline || !event::poll(Duration::ZERO)? {
                break;
            }
            let input = event::read()?;
            let previous_view = (
                app.mode,
                app.current,
                app.help_visible,
                app.theme_picker.is_some(),
            );
            let explicit_clear = matches!(
                &input,
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && ((app.mode == ViewMode::Fullscreen
                            && matches!(key.code, KeyCode::Home | KeyCode::End))
                            || (app.pending_delete.is_some()
                                && matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))))
            );
            if handle_event(app, input, &mut mouse_drag)? {
                return Ok(());
            }
            if previous_view
                != (
                    app.mode,
                    app.current,
                    app.help_visible,
                    app.theme_picker.is_some(),
                )
                || explicit_clear
            {
                displayed_fullscreen = None;
                if explicit_clear && app.mode == ViewMode::Fullscreen {
                    // Home/End can delete graphics without changing the index.
                    app.needs_render = true;
                }
            }
        }

        // 2. Poll background tasks
        app.refresh_from_scanner();
        app.poll_filter();
        app.poll_fullscreen();
        app.poll_source();
        app.poll_zoom();
        app.prefetcher.poll();
        pending_emits.extend(app.poll_thumbnails());

        // Existing clear paths invalidate uploaded IDs, even if the frame identity
        // is unchanged (Home/End at the boundary, help, or theme overlays).
        if fullscreen_image
            .is_some_and(|image: FullscreenImage| !app.transmitted_image_ids.contains(&image.id))
        {
            fullscreen_image = None;
            displayed_fullscreen = None;
            if app.mode == ViewMode::Fullscreen {
                app.needs_render = true;
            }
        }
        if (app.mode != ViewMode::Fullscreen || app.theme_picker.is_some() || app.help_visible)
            && fullscreen_image.is_some()
        {
            let mut out = io::stdout().lock();
            for id in encoder::FULLSCREEN_IMAGE_IDS {
                app.graphics.delete_image_to(&mut out, id)?;
                app.transmitted_image_ids.remove(&id);
            }
            out.flush()?;
            fullscreen_image = None;
        }

        #[cfg(feature = "video")]
        if let Some(ref mut v) = app.video {
            if v.poll_frame() {
                app.needs_render = true;
            }
        }

        if app.scan_complete
            && app.images.is_empty()
            && app.error.is_none()
            && app.mode != ViewMode::Picker
        {
            app.open_picker();
        }

        // 3. Draw UI chrome (always fast — ratatui only)
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.mode == ViewMode::Fullscreen {
            let view = (app.current, app.image_rect, app.cell_px);
            if fullscreen_view != Some(view) {
                fullscreen_view = Some(view);
                displayed_fullscreen = None;
            }
        } else {
            fullscreen_view = None;
            displayed_fullscreen = None;
        }

        if app.mode == ViewMode::Fullscreen
            && app.needs_render
            && !app.images.is_empty()
            && app.theme_picker.is_none()
            && !app.help_visible
        {
            // Compute and publish the newest available view without a frame delay.
            app.load_if_needed();
        }

        // 4. Render Kitty images
        if app.theme_picker.is_some() || app.help_visible || app.mode == ViewMode::Picker {
            app.needs_render = false;
            displayed_fullscreen = None;
            pending_emits.clear();
        } else if app.needs_render && !app.images.is_empty() {
            #[cfg(feature = "video")]
            let is_video = matches!(app.mode, ViewMode::Video);
            #[cfg(not(feature = "video"))]
            let is_video = false;

            match app.mode {
                ViewMode::Fullscreen => {
                    // A pending request keeps the previous terminal image intact.
                    // Dirty input alone is not a newly published image.
                    let frame = (
                        app.loaded_revision,
                        app.current,
                        app.image_rect,
                        app.cell_px,
                    );
                    if displayed_fullscreen != Some(frame)
                        && (app.loaded.is_some() || !app.zoom_render_pending())
                    {
                        render_fullscreen_image(app, &mut fullscreen_image)?;
                        displayed_fullscreen = Some(frame);
                    }
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

        // Wake promptly for worker results and for the final coalesced motion,
        // even when the input stream has stopped.
        let mut timeout = {
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
        if app.zoom_render_pending() {
            timeout = timeout.min(Duration::from_millis(4));
        }
        event::poll(timeout)?;
    }
}

fn handle_event(
    app: &mut App,
    event: Event,
    mouse_drag: &mut Option<(u16, u16)>,
) -> io::Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            *mouse_drag = None;
            if app.help_visible {
                app.help_visible = false;
                app.needs_render = true;
            } else if app.pending_delete.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        app.graphics_delete_all()?;
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
            } else if app.theme_picker.is_some() {
                handle_theme_picker_key(app, key.code);
            } else if key.code == KeyCode::Char('t')
                && !(app.mode == ViewMode::Gallery && app.gallery.search_active)
                && !(app.mode == ViewMode::Picker
                    && app
                        .picker
                        .as_ref()
                        .is_some_and(|picker| picker.filter_active))
            {
                app.graphics_delete_all()?;
                app.open_theme_picker();
            } else {
                match app.mode {
                    ViewMode::Fullscreen => match key.code {
                        KeyCode::Char('q') => return Ok(true),
                        KeyCode::Char('?') => {
                            app.graphics_delete_all()?;
                            app.help_visible = true;
                        }
                        KeyCode::Esc => {
                            app.graphics_delete_all()?;
                            app.enter_gallery();
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => app.zoom_in(),
                        KeyCode::Char('-') | KeyCode::Char('_') => app.zoom_out(),
                        KeyCode::Char('0') => app.reset_zoom(),
                        KeyCode::Left | KeyCode::Char('h') if app.zoom > 1.0 => app.pan(-1.0, 0.0),
                        KeyCode::Right | KeyCode::Char('l') if app.zoom > 1.0 => app.pan(1.0, 0.0),
                        KeyCode::Up | KeyCode::Char('k') if app.zoom > 1.0 => app.pan(0.0, -1.0),
                        KeyCode::Down | KeyCode::Char('j') if app.zoom > 1.0 => app.pan(0.0, 1.0),
                        KeyCode::Left | KeyCode::Char('h') => app.prev(),
                        KeyCode::Right | KeyCode::Char('l') => app.next(),
                        KeyCode::Home => {
                            app.graphics_delete_all()?;
                            app.first();
                        }
                        KeyCode::End => {
                            app.graphics_delete_all()?;
                            app.last();
                        }
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            app.begin_delete(matches!(key.code, KeyCode::Char('D')));
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
                            app.graphics_delete_all()?;
                            app.exit_video();
                        }
                        KeyCode::Char('?') => {
                            app.graphics_delete_all()?;
                            app.help_visible = true;
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            app.graphics_delete_all()?;
                            app.prev();
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            app.graphics_delete_all()?;
                            app.next();
                        }
                        KeyCode::Home => {
                            app.graphics_delete_all()?;
                            app.first();
                        }
                        KeyCode::End => {
                            app.graphics_delete_all()?;
                            app.last();
                        }
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            app.begin_delete(matches!(key.code, KeyCode::Char('D')));
                        }
                        _ => {}
                    },
                    ViewMode::Gallery if app.gallery.search_active => match key.code {
                        KeyCode::Esc => {
                            app.gallery.search_active = false;
                            if !app.gallery.search_query.is_empty() {
                                app.gallery.search_query.clear();
                                app.update_filter();
                            }
                        }
                        KeyCode::Enter => {
                            app.gallery.search_active = false;
                        }
                        KeyCode::Backspace if app.gallery.search_query.pop().is_some() => {
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
                                app.graphics_delete_all()?;
                                app.help_visible = true;
                            }
                            KeyCode::Char('/') => {
                                app.gallery.search_active = true;
                            }
                            KeyCode::Enter => {
                                app.graphics_delete_all()?;
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
                            KeyCode::PageUp | KeyCode::Char('b')
                                if ctrl || matches!(key.code, KeyCode::PageUp) =>
                            {
                                app.gallery.move_page_up();
                                app.pre_decode_hovered();
                            }
                            KeyCode::PageDown | KeyCode::Char('f')
                                if ctrl || matches!(key.code, KeyCode::PageDown) =>
                            {
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
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                app.begin_delete(matches!(key.code, KeyCode::Char('D')));
                            }
                            KeyCode::Char('o') => {
                                app.graphics_delete_all()?;
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
        Event::Mouse(mouse) => handle_mouse(app, mouse, mouse_drag),
        Event::Resize(_, _) => {
            *mouse_drag = None;
            app.mark_dirty();
        }
        Event::FocusLost => *mouse_drag = None,
        _ => {}
    }
    Ok(false)
}

fn handle_mouse(app: &mut App, mouse: MouseEvent, drag: &mut Option<(u16, u16)>) {
    if app.mode != ViewMode::Fullscreen
        || app.help_visible
        || app.pending_delete.is_some()
        || app.theme_picker.is_some()
    {
        *drag = None;
        return;
    }

    let inside = app
        .image_rect
        .contains(Position::new(mouse.column, mouse.row));
    match mouse.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            *drag = None;
            if inside {
                if mouse.kind == MouseEventKind::ScrollUp {
                    app.zoom_in();
                } else {
                    app.zoom_out();
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) if inside && app.zoom > 1.0 => {
            *drag = Some((mouse.column, mouse.row));
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((column, row)) = *drag {
                let dx = f64::from(i32::from(mouse.column) - i32::from(column));
                let dy = f64::from(i32::from(mouse.row) - i32::from(row));
                app.pan_pixels(dx * f64::from(app.cell_px.0), dy * f64::from(app.cell_px.1));
                *drag = Some((mouse.column, mouse.row));
            }
        }
        _ => *drag = None,
    }
}

fn handle_theme_picker_key(app: &mut App, code: KeyCode) {
    let Some(picker) = app.theme_picker.as_ref() else {
        return;
    };
    let selected = picker.selected;
    let visible_height = picker.visible_height.max(1);
    let last = app.themes.len().saturating_sub(1);
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            app.theme_picker_select(if selected == last { 0 } else { selected + 1 });
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.theme_picker_select(if selected == 0 { last } else { selected - 1 });
        }
        KeyCode::Home | KeyCode::Char('g') => app.theme_picker_select(0),
        KeyCode::End | KeyCode::Char('G') => app.theme_picker_select(last),
        KeyCode::PageDown => {
            app.theme_picker_select((selected + visible_height).min(last));
        }
        KeyCode::PageUp => app.theme_picker_select(selected.saturating_sub(visible_height)),
        KeyCode::Enter => app.theme_picker_confirm(),
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => app.theme_picker_cancel(),
        _ => {}
    }
}

fn handle_picker_key(app: &mut App, code: KeyCode) -> io::Result<bool> {
    let filter_active = app.picker.as_ref().is_some_and(|p| p.filter_active);

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
        KeyCode::Esc if !app.images.is_empty() => {
            app.close_picker();
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
        KeyCode::Right | KeyCode::Char('l') => {
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
        KeyCode::Enter => {
            let target = app
                .picker
                .as_ref()
                .and_then(|picker| picker.selected())
                .map(|entry| entry.path.clone());
            if let Some(target) = target {
                app.switch_to_dir(target);
                app.close_picker();
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
            // A terminal that does not report pixel dimensions (common under
            // terminal multiplexers such as rift/tmux, and some SSH setups)
            // returns ws.width/ws.height == 0 even though columns/rows are
            // valid. Treat a zero pixel report as "unknown" and fall back to
            // conventional cell metrics, otherwise the division below collapses
            // to 1x1 and thumbnails render only a few pixels tall.
            let w = if ws.columns > 0 && ws.width > 0 {
                ws.width as u32 / ws.columns as u32
            } else {
                8
            };
            let h = if ws.rows > 0 && ws.height > 0 {
                ws.height as u32 / ws.rows as u32
            } else {
                16
            };
            (w.max(1), h.max(1))
        })
        .unwrap_or((8, 16))
}

fn render_fullscreen_image(
    app: &mut App,
    fullscreen_image: &mut Option<FullscreenImage>,
) -> io::Result<()> {
    let mut out = BufWriter::with_capacity(64 * 1024, io::stdout().lock());
    render_fullscreen_image_to(app, fullscreen_image, &mut out)
}

fn render_fullscreen_image_to<W: Write>(
    app: &mut App,
    fullscreen_image: &mut Option<FullscreenImage>,
    out: &mut W,
) -> io::Result<()> {
    use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};

    let Some(img) = &app.loaded else {
        queue!(out, BeginSynchronizedUpdate)?;
        app.graphics_delete_all_to(out)?;
        for id in encoder::FULLSCREEN_IMAGE_IDS {
            app.graphics.delete_image_to(out, id)?;
        }
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        *fullscreen_image = None;
        return Ok(());
    };

    let previous =
        (*fullscreen_image).filter(|image| app.transmitted_image_ids.contains(&image.id));
    let reuse = previous.is_some_and(|image| image.revision == app.loaded_revision);
    let id = if reuse {
        previous.unwrap().id
    } else if previous.is_some_and(|image| image.id == encoder::FULLSCREEN_IMAGE_IDS[0]) {
        encoder::FULLSCREEN_IMAGE_IDS[1]
    } else {
        encoder::FULLSCREEN_IMAGE_IDS[0]
    };

    if !reuse {
        // a=t creates no placement. Never retransmit the visible ID: Kitty
        // deletes its old placements when that ID receives replacement data.
        // Finish all chunks before sending any other graphics command.
        app.graphics.upload_to(out, img, id)?;
        out.flush()?;
    }

    let (cpw, cph) = app.cell_px;
    let img_cols = img.width().div_ceil(cpw);
    let img_rows = img.height().div_ceil(cph);
    let offset_x = app.image_rect.x + (app.image_rect.width.saturating_sub(img_cols as u16)) / 2;
    let offset_y = app.image_rect.y + (app.image_rect.height.saturating_sub(img_rows as u16)) / 2;

    // Only short placement/retirement commands are synchronized; a slow bulk
    // upload cannot outlive the terminal's synchronization timeout and blank it.
    queue!(
        out,
        BeginSynchronizedUpdate,
        cursor::MoveTo(offset_x, offset_y)
    )?;
    if previous.is_none() {
        // Scanner-driven gallery -> fullscreen transitions need not pass through
        // an input clear. Remove those placements only after the upload finishes.
        app.graphics.clear_placements_to(out)?;
    }
    app.graphics.place_with_id_to(out, id, 1)?;
    if let Some(previous) = previous.filter(|image| image.id != id) {
        app.graphics.delete_image_to(out, previous.id)?;
    }
    queue!(out, EndSynchronizedUpdate)?;
    out.flush()?;

    if let Some(previous) = previous.filter(|image| image.id != id) {
        app.transmitted_image_ids.remove(&previous.id);
    }
    app.transmitted_image_ids.insert(id);
    *fullscreen_image = Some(FullscreenImage {
        id,
        revision: app.loaded_revision,
    });
    Ok(())
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
            let img_cols = img.width().div_ceil(cpw);
            let img_rows = img.height().div_ceil(cph);
            let offset_x =
                app.image_rect.x + (app.image_rect.width.saturating_sub(img_cols as u16)) / 2;
            let offset_y =
                app.image_rect.y + (app.image_rect.height.saturating_sub(img_rows as u16)) / 2;

            queue!(out, BeginSynchronizedUpdate)?;
            queue!(out, cursor::MoveTo(offset_x, offset_y))?;
            app.graphics.transmit(
                &mut out,
                img,
                &encoder::DisplayOptions {
                    id: Some(VIDEO_IMAGE_ID),
                    cols: None,
                    rows: None,
                },
            )?;
            queue!(out, EndSynchronizedUpdate)?;
        }
    }

    out.flush()
}

fn emit_new_thumbnails(app: &mut App, new_indices: &[usize]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let visible: Vec<(usize, usize)> = app.gallery.visible_items().collect();
    let mut newly_transmitted: Vec<u32> = Vec::new();
    for (vis_idx, img_idx) in visible {
        if !new_indices.contains(&img_idx) {
            continue;
        }
        let cell_rect = app.gallery.cell_rect(vis_idx);
        if let Some((img, id)) = app.thumb_cache.peek(img_idx) {
            queue!(out, cursor::MoveTo(cell_rect.x + 1, cell_rect.y + 1))?;
            app.graphics.transmit(
                &mut out,
                img,
                &encoder::DisplayOptions {
                    id: Some(id),
                    cols: None,
                    rows: None,
                },
            )?;
            newly_transmitted.push(id);
        }
    }
    out.flush()?;
    for id in newly_transmitted {
        app.transmitted_image_ids.insert(id);
    }
    Ok(())
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
    if app.graphics_storage_dirty {
        app.graphics_delete_all_to(&mut out)?;
        app.graphics_storage_dirty = false;
    } else {
        // Drop visible placements only; keep stored image data so already-transmitted thumbs
        // can be re-placed with a cheap `a=p` instead of a full PNG retransmit.
        app.graphics.clear_placements_to(&mut out)?;
    }

    let mut newly_transmitted: Vec<u32> = Vec::new();
    for &(vis_idx, img_idx) in &visible {
        let cell_rect = app.gallery.cell_rect(vis_idx);
        if let Some((img, id)) = app.thumb_cache.peek(img_idx) {
            let inner_x = cell_rect.x + 1;
            let inner_y = cell_rect.y + 1;
            queue!(out, cursor::MoveTo(inner_x, inner_y))?;
            if app.transmitted_image_ids.contains(&id) {
                app.graphics.place_by_id_to(&mut out, id)?;
            } else {
                app.graphics.transmit(
                    &mut out,
                    img,
                    &encoder::DisplayOptions {
                        id: Some(id),
                        cols: None,
                        rows: None,
                    },
                )?;
                newly_transmitted.push(id);
            }
        }
    }

    out.flush()?;
    for id in newly_transmitted {
        app.transmitted_image_ids.insert(id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        App, FullscreenImage, KeyCode, SharedImageList, ViewMode, handle_picker_key,
        handle_theme_picker_key, render_fullscreen_image_to,
    };
    use crate::encoder::FULLSCREEN_IMAGE_IDS;
    use crate::theme::{NamedTheme, Theme};
    use image::RgbaImage;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingWriter {
        bytes: Vec<u8>,
        flushes: Vec<usize>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes.push(self.bytes.len());
            Ok(())
        }
    }

    fn fullscreen_app() -> App {
        let mut app = App::new(Theme::default(), (8, 16), SharedImageList::new());
        app.mode = ViewMode::Fullscreen;
        app.image_rect = Rect::new(0, 0, 20, 10);
        app.loaded = Some(RgbaImage::new(16, 16));
        app.loaded_revision = 1;
        app
    }

    #[test]
    fn fullscreen_upload_finishes_before_synchronized_place_and_retire() {
        let mut app = fullscreen_app();
        let old_id = FULLSCREEN_IMAGE_IDS[0];
        let new_id = FULLSCREEN_IMAGE_IDS[1];
        app.transmitted_image_ids.insert(old_id);
        let mut image = Some(FullscreenImage {
            id: old_id,
            revision: 0,
        });
        let mut out = RecordingWriter::default();
        render_fullscreen_image_to(&mut app, &mut image, &mut out).unwrap();

        assert_eq!(out.flushes.len(), 2);
        let upload = std::str::from_utf8(&out.bytes[..out.flushes[0]]).unwrap();
        assert!(upload.starts_with("\x1b_Ga=t,"));
        assert!(upload.contains(&format!(",i={new_id},")));
        assert!(upload.contains("m=0;"));
        assert!(upload.ends_with("\x1b\\"));
        assert!(!upload.contains("\x1b_Ga=d"));
        assert!(!upload.contains("\x1b[?2026h"));
        let swap = std::str::from_utf8(&out.bytes[out.flushes[0]..]).unwrap();
        assert!(swap.starts_with("\x1b[?2026h"));
        let placement = swap.find(&format!("a=p,q=2,C=1,i={new_id},p=1")).unwrap();
        let retirement = swap.find(&format!("a=d,d=I,q=2,i={old_id}")).unwrap();
        assert!(placement < retirement);
        assert!(swap.ends_with("\x1b[?2026l"));
        assert!(!swap.contains("d=A"));
        assert_eq!(image.unwrap().id, new_id);
        assert!(!app.transmitted_image_ids.contains(&old_id));
        assert!(app.transmitted_image_ids.contains(&new_id));

        app.loaded_revision += 1;
        let mut next = Vec::new();
        render_fullscreen_image_to(&mut app, &mut image, &mut next).unwrap();
        assert_eq!(image.unwrap().id, old_id);
        assert_eq!(app.transmitted_image_ids.len(), 1);
    }

    #[test]
    fn fullscreen_reuses_unchanged_upload_but_reuploads_after_external_clear() {
        let mut app = fullscreen_app();
        let mut image = None;
        render_fullscreen_image_to(&mut app, &mut image, &mut Vec::new()).unwrap();
        let id = image.unwrap().id;

        // Geometry/chrome redraws reposition the same stable placement, not pixels.
        app.image_rect.x = 1;
        let mut redraw = Vec::new();
        render_fullscreen_image_to(&mut app, &mut image, &mut redraw).unwrap();
        let redraw = String::from_utf8(redraw).unwrap();
        assert!(!redraw.contains("a=t"));
        assert!(!redraw.contains("a=d"));
        assert!(redraw.contains(&format!("a=p,q=2,C=1,i={id},p=1")));

        app.transmitted_image_ids.clear();
        let mut cleared = Vec::new();
        render_fullscreen_image_to(&mut app, &mut image, &mut cleared).unwrap();
        assert!(
            String::from_utf8(cleared)
                .unwrap()
                .starts_with("\x1b_Ga=t,")
        );
        assert_eq!(app.transmitted_image_ids.len(), 1);
    }

    #[test]
    fn fullscreen_missing_image_clears_placements_and_both_reserved_slots() {
        let mut app = fullscreen_app();
        let mut image = None;
        render_fullscreen_image_to(&mut app, &mut image, &mut Vec::new()).unwrap();
        app.loaded = None;
        let mut cleared = Vec::new();
        render_fullscreen_image_to(&mut app, &mut image, &mut cleared).unwrap();
        let cleared = String::from_utf8(cleared).unwrap();
        assert!(cleared.contains("a=d,d=A"));
        for id in FULLSCREEN_IMAGE_IDS {
            assert!(cleared.contains(&format!("a=d,d=I,q=2,i={id}")));
        }
        assert!(image.is_none());
        assert!(app.transmitted_image_ids.is_empty());
    }

    fn picker_app(root: &Path) -> App {
        let list = SharedImageList::new();
        let mut app = App::new(Theme::default(), (8, 16), list);
        app.initial_dir = Some(root.to_path_buf());
        app.open_picker();
        app
    }

    fn synthetic_tree(test_name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "rview-synthetic-{test_name}-{}",
            std::process::id()
        ));
        let child = root.join("synthetic-images");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&child).unwrap();
        (
            fs::canonicalize(root).unwrap(),
            fs::canonicalize(child).unwrap(),
        )
    }

    fn select_path(app: &mut App, target: &Path) {
        let picker = app.picker.as_mut().unwrap();
        let entry_index = picker
            .entries
            .iter()
            .position(|entry| entry.path == target)
            .unwrap();
        picker.cursor = picker
            .filtered_indices
            .iter()
            .position(|index| *index == entry_index)
            .unwrap();
    }

    #[test]
    fn enter_chooses_directory_and_closes_picker() {
        let (root, child) = synthetic_tree("choose-directory");
        let mut app = picker_app(&root);
        select_path(&mut app, &child);

        assert!(!handle_picker_key(&mut app, KeyCode::Enter).unwrap());
        assert_eq!(app.mode, ViewMode::Gallery);
        assert!(app.picker.is_none());
        assert_eq!(app.current_dir, Some(fs::canonicalize(&child).unwrap()));

        std::thread::sleep(Duration::from_millis(20));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn right_descends_without_closing_picker() {
        let (root, child) = synthetic_tree("descend-directory");
        let mut app = picker_app(&root);
        select_path(&mut app, &child);

        assert!(!handle_picker_key(&mut app, KeyCode::Right).unwrap());
        assert_eq!(app.mode, ViewMode::Picker);
        assert_eq!(
            app.picker.as_ref().map(|picker| &picker.current_dir),
            Some(&fs::canonicalize(&child).unwrap())
        );

        std::thread::sleep(Duration::from_millis(20));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn theme_picker_applies_for_the_session_and_cancel_restores() {
        let list = SharedImageList::new();
        let first = Theme::fallback();
        let mut second = Theme::fallback();
        second.title = Style::default().fg(Color::Red);
        let mut app = App::new(first.clone(), (8, 16), list);
        app.install_themes(
            vec![
                NamedTheme {
                    name: "first".to_string(),
                    path: None,
                    theme: first.clone(),
                },
                NamedTheme {
                    name: "second".to_string(),
                    path: None,
                    theme: second.clone(),
                },
            ],
            0,
        );

        app.open_theme_picker();
        handle_theme_picker_key(&mut app, KeyCode::Down);
        assert_eq!(app.theme, second);
        app.theme_picker_confirm();
        assert_eq!(app.theme_index, 1);
        assert!(app.theme_picker.is_none());

        app.open_theme_picker();
        handle_theme_picker_key(&mut app, KeyCode::Up);
        assert_eq!(app.theme, first);
        app.theme_picker_cancel();
        assert_eq!(app.theme, second);
        assert_eq!(app.theme_index, 1);
    }

    #[test]
    fn theme_picker_pages_by_the_visible_rows() {
        let list = SharedImageList::new();
        let theme = Theme::fallback();
        let mut app = App::new(theme.clone(), (8, 16), list);
        let themes = (0..12)
            .map(|index| NamedTheme {
                name: format!("theme-{index}"),
                path: None,
                theme: theme.clone(),
            })
            .collect();
        app.install_themes(themes, 0);
        app.open_theme_picker();
        app.theme_picker.as_mut().unwrap().visible_height = 5;

        handle_theme_picker_key(&mut app, KeyCode::PageDown);
        assert_eq!(app.theme_picker.as_ref().unwrap().selected, 5);
        handle_theme_picker_key(&mut app, KeyCode::PageDown);
        assert_eq!(app.theme_picker.as_ref().unwrap().selected, 10);
        handle_theme_picker_key(&mut app, KeyCode::PageUp);
        assert_eq!(app.theme_picker.as_ref().unwrap().selected, 5);
    }
}
