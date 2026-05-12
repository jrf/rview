# rview

A fast TUI image viewer written in Rust, using the Kitty graphics protocol.

## Build & Run

```bash
cargo build              # dev build
cargo build --release    # optimized build
cargo run -- <image>     # run directly
```

## Architecture

- `src/main.rs` — CLI entry point (clap), terminal setup, event loop, Kitty image rendering
- `src/app.rs` — application state (ViewMode, image list, thumbnail cache, help_visible)
- `src/image_list.rs` — SharedImageList: append-only thread-safe image list (Arc<Mutex> + AtomicUsize)
- `src/scanner.rs` — background directory walker, pushes batches of 1024 paths into SharedImageList
- `src/ui.rs` — ratatui layout: bordered titled blocks, search bar, help popup
- `src/gallery.rs` — grid layout math, cursor navigation, scroll tracking, thumbnail cache
- `src/search.rs` — fuzzy filename matching via nucleo-matcher
- `src/theme.rs` — semantic theme system (tokyonight, dark, light, catppuccin, nord)
- `src/encoder/kitty.rs` — Kitty graphics protocol encoder (chunked base64, display opts)

## Key Dependencies

- `image` — image decoding and resizing
- `clap` — CLI argument parsing
- `ratatui` + `crossterm` — TUI framework and terminal backend
- `base64` — Kitty protocol payload encoding
- `nucleo-matcher` — fuzzy search

## Rendering Model

Two-phase rendering: ratatui draws UI chrome (borders, labels, status), then Kitty escape
sequences write images directly to stdout. The `needs_render` flag prevents unnecessary
image retransmission (only re-sends when scroll offset changes or window resizes).

## Conventions

- `q=2` on all Kitty graphics commands (suppresses terminal responses)
- Cell pixel size detected via `crossterm::terminal::window_size()` for accurate image sizing
