mod app;
mod encoder;
mod ui;

use app::App;
use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "rview", about = "A fast terminal image viewer")]
struct Cli {
    /// Image file(s) to display
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    for f in &cli.files {
        if !f.exists() {
            eprintln!("{}: file not found", f.display());
            std::process::exit(1);
        }
    }

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cli.files);
    let result = run(&mut terminal, &mut app);

    encoder::kitty::delete_all()?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen)?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        if app.needs_render {
            app.load_if_needed()?;
            render_image(app)?;
            app.needs_render = false;
            terminal.draw(|frame| ui::draw(frame, app))?;
        }

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Left | KeyCode::Char('h') => app.prev(),
                KeyCode::Right | KeyCode::Char('l') => app.next(),
                _ => {}
            },
            Event::Resize(_, _) => app.mark_dirty(),
            _ => {}
        }
    }
}

fn render_image(app: &App) -> io::Result<()> {
    let mut out = io::stdout().lock();
    encoder::kitty::delete_all_to(&mut out)?;

    if let Some(ref img) = app.loaded {
        let img_cols = img.width() / app::CELL_WIDTH;
        let img_rows = img.height() / app::CELL_HEIGHT;
        let offset_x = app.image_rect.x + (app.image_rect.width.saturating_sub(img_cols as u16)) / 2;
        let offset_y = app.image_rect.y + (app.image_rect.height.saturating_sub(img_rows as u16)) / 2;

        queue!(out, cursor::MoveTo(offset_x, offset_y))?;
        encoder::kitty::encode_to(&mut out, img)?;
    }

    out.flush()
}
