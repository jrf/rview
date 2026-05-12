# rview

A fast terminal image viewer built with Rust and the Kitty graphics protocol.

Browse directories of images in a thumbnail gallery, search by filename with fuzzy matching, and view images fullscreen — all without leaving the terminal.

## Install

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
cp target/release/rview ~/.local/bin/
```

## Usage

```bash
rview                              # current directory
rview ~/photos/                    # specific directory
rview image.png                    # single image (fullscreen)
rview photo.jpg screenshot.png     # multiple files
rview -t catppuccin ~/photos/      # choose theme
```

## Controls

### Gallery

| Key | Action |
|-----|--------|
| `h` `j` `k` `l` | Navigate grid |
| `Enter` | Open fullscreen |
| `/` | Search filenames |
| `?` | Help |
| `q` `Esc` | Quit |

### Search

| Key | Action |
|-----|--------|
| Type | Filter by filename (fuzzy) |
| `Enter` | Confirm filter |
| `Esc` | Cancel and clear |
| `Backspace` | Delete character |

### Fullscreen

| Key | Action |
|-----|--------|
| `h` `l` `←` `→` | Previous / next image |
| `Esc` | Back to gallery |
| `?` | Help |
| `q` | Quit |

## Themes

Five built-in color themes: **tokyonight** (default), **dark**, **light**, **catppuccin**, **nord**.

```bash
rview -t nord
```

## Supported Formats

PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, AVIF, and more via the [image](https://crates.io/crates/image) crate.

## Requirements

A terminal with [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) support:

- [Kitty](https://sw.kovidgoyal.net/kitty/)
- [WezTerm](https://wezfurlong.org/wezterm/)
- [Ghostty](https://ghostty.org/)

## License

MIT
