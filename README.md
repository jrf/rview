# rview

A terminal media viewer for the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/). It shows a directory as a thumbnail grid, fuzzy-searches filenames, and opens images fullscreen. Video playback is optional.

Works in [Kitty](https://sw.kovidgoyal.net/kitty/), [WezTerm](https://wezfurlong.org/wezterm/), and [Ghostty](https://ghostty.org/).

## Install

```bash
cargo install --path .
cargo install --path . --features video   # requires ffmpeg 7+
```

## Usage

```bash
rview                                      # current directory
rview ~/photos
rview image.png                            # single file opens fullscreen
rview drawing.svg                          # SVG files also work in the gallery
rview photo.jpg screenshot.png
rview -t ~/.config/themes/nord.toml        # theme file, or a catalog name
rview -j 4 ~/photos                        # decode threads (default: all cores)
```

Themes load from `~/.config/rview/config.toml` (`theme`, `theme_catalog`). `-t` overrides the session. `t` previews the catalog; that choice is not written back.

## Controls

| Key | Action |
|-----|--------|
| `h` `j` `k` `l`, arrows | Move |
| `g` `G`, `Home` `End` | First / last |
| `Ctrl-b` `Ctrl-f`, `PgUp` `PgDn` | Page up / down |
| `Enter` | Open |
| `/` | Search. `Enter` applies, `Esc` clears, `Backspace` deletes |
| `Space` | Toggle selection; pause or resume video |
| `a` `A` | Select all / clear selection |
| `d` `D` | Trash / permanently delete, with confirm |
| `o` | Directory picker |
| `t` | Theme picker |
| `+` `-` `0` | Zoom in, out, fit. `hjkl` pans when zoomed |
| `Esc` | Back, or quit from the gallery |
| `q` | Quit |
| `?` | Full help |

## Formats

**Images:** PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, AVIF, SVG.

SVGs render as static images with transparency, system fonts, and local image references resolved relative to the SVG file. Thumbnails rasterize at the target size; fullscreen fit does not upscale beyond the SVG's intrinsic dimensions. Zoom and pan re-render the vector image at the visible viewport's pixel resolution, preserving detail instead of enlarging an intrinsic-resolution raster.

SVG thumbnails are cached only in memory so changes to referenced files appear when reopening the gallery. SVG raster buffers use the same 512 MiB allocation limit as the standard image decoder. Zoom without filters allocates only the visible viewport, so oversized intrinsic dimensions do not require oversized raster buffers. SVG filters render on a full zoomed canvas before cropping, preserving effects whose inputs lie outside the viewport; that canvas and the visible crop share the allocation limit.

**Video** (`--features video`): MP4, MOV, MKV, AVI, WebM, M4V. Playback loops at up to 10 fps. Gallery thumbnails use the first frame.

## License

MIT
