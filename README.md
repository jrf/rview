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
| Mouse wheel | Zoom in / out over the fullscreen image viewport |
| Left-button drag | Pan the zoomed fullscreen image |
| `Esc` | Back, or quit from the gallery |
| `q` | Quit |
| `?` | Full help |

Mouse navigation is inactive in dialogs and outside fullscreen image viewing.

## Formats

**Images:** PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, AVIF, SVG.

SVGs render as static images with transparency, system fonts, and local image references resolved relative to the SVG file. Thumbnails rasterize at the target size; fullscreen fit does not upscale beyond the SVG's intrinsic dimensions. Zoom renders vectors at the displayed pixel resolution, preserving detail instead of enlarging an intrinsic-resolution raster. Zoom and pan rendering runs in the background, prioritizing the latest requested view: obsolete results are skipped, and reusable SVG canvases crop directly to the newest pan position. Ready frames publish immediately, without a fixed animation cadence.

Fullscreen keeps the previous image visible while rendering and transferring its replacement, then swaps placements after the transfer finishes. Lossless PNG uploads and streamed encoding reduce terminal traffic.

SVG thumbnails are cached only in memory so changes to referenced files appear when reopening the gallery. SVG raster buffers use the same 512 MiB allocation limit as the standard image decoder. Zoom retains a straight-alpha rendered canvas for panning when it fits that budget; whole-pixel pans crop it without re-rasterizing or repeating alpha conversion, while scale or subpixel changes re-render. Unfiltered SVGs fall back to viewport-only rendering when the full canvas would exceed the limit. SVG filters require the full zoomed canvas to preserve inputs outside the viewport; that canvas and the visible crop share the allocation limit.

**Video** (`--features video`): MP4, MOV, MKV, AVI, WebM, M4V. Playback loops at up to 10 fps. Gallery thumbnails use the first frame.

## License

MIT
