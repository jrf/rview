# rview

A fast TUI image viewer written in Rust, using the Kitty graphics protocol.

## Build & Run

```bash
cargo build              # dev build
cargo build --release    # optimized build
cargo run -- <image>     # run directly
```

## Architecture

- `src/main.rs` — CLI entry point (clap), terminal setup, event loop
- `src/app.rs` — application state (image list, current index, loaded image)
- `src/ui.rs` — ratatui layout and status bar rendering
- `src/encoder/kitty.rs` — Kitty graphics protocol encoder

## Key Dependencies

- `image` — image decoding and resizing
- `clap` — CLI argument parsing
- `ratatui` + `crossterm` — TUI framework and terminal backend
- `base64` — Kitty protocol payload encoding
