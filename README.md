# rview

A fast TUI image viewer using the Kitty graphics protocol.

## Install

```bash
cargo install --path .
```

## Usage

```bash
rview image.png
rview photo.jpg screenshot.png    # multiple files
```

### Controls

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `←` / `h` | Previous image |
| `→` / `l` | Next image |

## Supported Formats

PNG, JPEG, GIF, WebP, BMP, TIFF, and more (via the `image` crate).

## Requirements

A terminal that supports the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) (Kitty, WezTerm, Ghostty, etc.).
