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

**Images:** PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, AVIF.

**Video** (`--features video`): MP4, MOV, MKV, AVI, WebM, M4V. Playback loops at up to 10 fps. Gallery thumbnails use the first frame.

## License

MIT
