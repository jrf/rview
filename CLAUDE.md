# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build                        # dev build (deps optimized at opt-level=2)
cargo build --features video       # with video playback (requires system ffmpeg 7+)
cargo build --release              # release: LTO, strip, opt-level=3
cargo run -- <path>                # run on file or directory
cargo run -- -j4 ~/photos         # limit to 4 decode threads
cargo build --no-default-features  # build without turbojpeg (avoids system dep)
```

There is also a `justfile` with `build`, `release`, `install`, `run`, `clean` targets.

No test suite exists yet.

## Architecture

### Two-Phase Rendering

The event loop (`main.rs:run`) separates fast UI updates from expensive image I/O:

1. Drain all buffered keyboard events (never block navigation)
2. Poll background workers (scanner, filter, prefetch, thumbnails, fullscreen decode, video frames)
3. Draw UI chrome via ratatui (always fast, CPU-only)
4. Emit Kitty graphics escape sequences to stdout (only when `needs_render` is set)
5. Wait with adaptive timeout (50ms if thumbnails pending, 250ms idle, frame-interval for video)

The `needs_render` flag is the gate between phases 3 and 4. Only set it when images actually need retransmission (scroll change, resize, mode switch, new image loaded).

**Video flicker prevention**: For video mode, the second `terminal.draw()` call is skipped (ratatui rewriting empty cells over the image area causes visible flicker). Video frames are wrapped in `BeginSynchronizedUpdate`/`EndSynchronizedUpdate` for atomic display. Frames use a fixed Kitty image ID (900) so the terminal replaces rather than delete-then-draw.

### Async Task Orchestration

`App` in `app.rs` coordinates five independent async pipelines, all polled each loop iteration:

| Pipeline | Channel | Producer | Consumer |
|----------|---------|----------|----------|
| Directory scan | `SharedImageList` (atomics) | `scanner::spawn` thread | `refresh_from_scanner()` |
| Fuzzy search | `mpsc` | `update_filter()` thread | `poll_filter()` |
| Fullscreen decode | `mpsc` | `start_fullscreen_decode()` via rayon | `poll_fullscreen()` |
| Thumbnail decode | `mpsc` + generation counter | `spawn_thumb_decode()` via rayon | `poll_thumbnails()` |
| Image prefetch | `mpsc` + LRU(8) | `prefetcher.kick()` via rayon | `prefetcher.poll()` |

**Generation counter pattern**: `thumb_generation` increments on resize/invalidation. Stale decode results (wrong generation) are silently discarded. This prevents old thumbnails from overwriting fresh ones after a resize.

**Prefetcher stores raw `DynamicImage`** (decoded, not resized). The SIMD resize via `fast_image_resize` is ~5ms, so resize happens lazily in `take_resized()` when the image is actually needed. This makes the cache rect-independent. Prefetch radius is ±3 neighbors.

### Video Playback (feature-gated: `video`)

All video code is behind `#[cfg(feature = "video")]` — compiles to zero without the feature flag. Requires system ffmpeg libraries.

`video.rs` spawns a dedicated `std::thread` per video. The decode thread opens the file, creates decoder + swscale context (handles both YUV→RGBA conversion AND resize to target pixels in one pass), and pushes frames into a bounded `sync_channel(3)`. The main thread consumes frames at 10fps max via `poll_frame()`.

**Thread communication**: `VideoCmd` enum (`Play`/`Pause`/`Stop`) sent via `mpsc::channel`. On pause, the decode thread blocks on `cmd_rx.recv()`. On stop or drop, the thread exits.

**Aspect ratio**: `fit_aspect()` computes the largest size fitting within the target rect while preserving the source aspect ratio. Applied in both `decode_loop` (playback) and `decode_first_frame` (gallery thumbnails).

**No ffmpeg objects cross thread boundaries** — the `open()` method extracts metadata (fps, duration) then drops the input context. The decode thread re-opens the file internally. This avoids `Send` issues with `SwsContext`.

### Kitty Graphics Protocol

Encoder in `encoder/kitty.rs`. Uses PNG format (`f=100`) for all rendering (~5x less PTY data than raw RGBA, important for SSH). All commands use `q=2` to suppress terminal responses. Images are chunked into 4096-byte base64 segments with `m=` continuation flag.

### Key Data Flow

**Gallery → Fullscreen transition**:
1. Gallery hover pre-decodes selected image into prefetch cache (raw decode, no resize)
2. Enter clears loaded image, switches mode, sets `needs_render`
3. Next frame: `ui::draw` sets correct `image_rect`, then `load_if_needed` checks prefetch cache
4. If cache hit: instant SIMD resize (~5ms). If miss: async decode starts, renders when ready

**Gallery → Video transition**:
1. Enter on a video file sets `ViewMode::Video`, `open_video_if_needed()` spawns decode thread
2. Each loop iteration: `poll_frame()` checks if a new frame is ready and interval has elapsed
3. Frame rendered with fixed image ID for atomic replacement

**Scanner → Gallery**:
1. Scanner thread batches 1024 paths into `SharedImageList`
2. `refresh_from_scanner()` drains new paths incrementally (no full rebuild)
3. If no active search filter, new indices appended directly to `filtered_indices`

### Module Responsibilities

- **`app.rs`** — State machine, async task coordination, `decode_image_with_hint()` and `resize_decoded()` (public for prefetcher)
- **`main.rs`** — Event loop, Kitty image rendering functions, terminal setup/teardown
- **`gallery.rs`** — Grid layout math (fixed 20×9 cells), cursor/scroll, `ThumbnailCache` (LRU-200)
- **`prefetch.rs`** — Background image pre-decode with LRU-8 cache of raw `DynamicImage`
- **`video.rs`** — Decode thread, frame pacing, first-frame extraction for thumbnails (cfg-gated)
- **`image_list.rs`** — Lock-free length reads via `AtomicUsize`, `AtomicBool` for completion flag
- **`scanner.rs`** — Uses `entry.file_type()` not `path.is_file()` to avoid stat syscalls on large dirs
- **`ui.rs`** — Ratatui widgets, help popup, filename truncation

## Conventions

- Cell pixel size detected once at startup via `crossterm::terminal::window_size()`, used everywhere for image↔cell coordinate math
- Thumbnail emission capped at 4 per frame to prevent PTY saturation
- Fullscreen async decode validates both index AND rect match before applying (prevents stale frames from wrong-size decodes)
- `turbo` feature (default): uses turbojpeg for JPEG decode, falls back to `image` crate on failure
- `video` feature (optional): uses ffmpeg-next for video decode, adds ~400 lines
- Rust 2024 edition: `gen` is a reserved keyword
