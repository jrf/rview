# rview

A fast terminal image/video viewer built with Rust and the Kitty graphics protocol.

Browse directories of images and videos in a thumbnail gallery, search by filename with fuzzy matching, and view media fullscreen — all without leaving the terminal.

## Install

```bash
cargo install --path .
```

With video playback support (requires system ffmpeg libraries):

```bash
cargo install --path . --features video
```

Or build from source:

```bash
cargo build --release
cargo build --release --features video  # with video support
cp target/release/rview ~/.local/bin/
```

## Usage

```bash
rview                              # current directory
rview ~/photos/                    # specific directory
rview image.png                    # single image (fullscreen)
rview photo.jpg screenshot.png     # multiple files
rview -t ~/.config/themes/catppuccin-mocha.toml ~/photos/
```

## Controls

### Gallery

| Key | Action |
|-----|--------|
| `h` `j` `k` `l` | Navigate grid |
| `g` `G` `Home` `End` | Jump to first / last |
| `Enter` | Open fullscreen |
| `Space` | Toggle selection at cursor |
| `a` `A` | Select all filtered / clear selection |
| `d` | Move selection (or cursor image) to Trash, with confirm |
| `D` | Permanently delete selection (or cursor image), with confirm |
| `o` | Open directory picker |
| `t` | Open session theme picker |
| `/` | Search filenames |
| `?` | Help |
| `q` `Esc` | Quit |

### Directory Picker

| Key | Action |
|-----|--------|
| `j` `k` `↑` `↓` | Move cursor |
| `Enter` | Choose highlighted directory and open its gallery |
| `l` `→` | Descend into highlighted directory |
| `h` `←` | Go to parent directory |
| `g` `G` `Home` `End` | First / last |
| `/` | Filter directory names (fuzzy) |
| `t` | Open session theme picker |
| `Esc` | Back to gallery |
| `q` | Quit |

### Search

| Key | Action |
|-----|--------|
| Type | Filter by filename (fuzzy) |
| `Enter` | Confirm filter |
| `Esc` | Cancel and clear |
| `Backspace` | Delete character |

### Theme Picker

| Key | Action |
|-----|--------|
| `j` `k` `↑` `↓` | Preview previous / next theme |
| `g` `G` `Home` `End` | Jump to first / last theme |
| `PageUp` `PageDown` | Move by one visible page |
| `Enter` | Apply for the current session |
| `Esc` `q` `t` | Cancel and restore the previous theme |

### Fullscreen

| Key | Action |
|-----|--------|
| `h` `l` `←` `→` | Previous / next image |
| `Home` `End` | Jump to first / last |
| `d` | Move current image to Trash, with confirm |
| `D` | Permanently delete current image, with confirm |
| `t` | Open session theme picker |
| `Esc` | Back to gallery |
| `?` | Help |
| `q` | Quit |

### Video Playback

| Key | Action |
|-----|--------|
| `Space` | Pause / resume |
| `h` `l` `←` `→` | Previous / next file |
| `t` | Open session theme picker |
| `Esc` | Back to gallery |
| `?` | Help |
| `q` | Quit |

## Themes

Rview reads two explicit paths from `~/.config/rview/config.toml`:

```toml
theme = "~/.config/themes/tokyo-night-moon.toml"
theme_catalog = "~/.config/themes/catalog.toml"
```

`theme` is loaded directly at startup. `theme_catalog` contains a `themes = [...]`
array of explicit file paths used by the picker; Rview never scans a theme
directory. Press `t` to preview catalog themes, `Enter` to keep one for the
current session, or `Esc` to restore the prior theme. Picker changes never
rewrite `config.toml`; edit `theme` directly to change the startup theme.

`--theme` accepts an explicit theme path or catalog name as a session-only
startup override.

## Supported Formats

**Images:** PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, and AVIF.

**Video** (with `video` feature): MP4, MOV, MKV, AVI, WebM, M4V. Videos play at up to 10fps using PNG-encoded frames over the Kitty protocol. Videos loop automatically and show first-frame thumbnails in the gallery.

## Requirements

A terminal with [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) support:

- [Kitty](https://sw.kovidgoyal.net/kitty/)
- [WezTerm](https://wezfurlong.org/wezterm/)
- [Ghostty](https://ghostty.org/)

For video support: system ffmpeg libraries (ffmpeg 7+).

## Development

Run the complete local validation suite with:

```bash
just check-all
```

The suite checks formatting, runs Clippy with warnings denied, and tests both the default and video-enabled feature sets. Use `just check` when FFmpeg is unavailable to validate the default feature set only.

## License

MIT. See [LICENSE](LICENSE).
