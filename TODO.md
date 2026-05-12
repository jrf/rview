# TODO

## Now

## Next

- [ ] Sixel protocol support #feature
- [ ] Stdin piping support (`curl ... | rview -`) #feature
- [ ] Animated GIF playback #feature

## Later

- [ ] Remote URL support (`rview https://...`) #feature
- [ ] Mouse-based pan/zoom in interactive mode #feature

## Done

- [x] Large directory scaling (streaming background scanner, incremental display) #improvement
- [x] Kitty graphics protocol encoding #feature
- [x] CLI arg parsing with clap #feature
- [x] Auto-resize to terminal dimensions #feature
- [x] Multi-file support #feature
- [x] TUI with ratatui (status bar, keyboard navigation, centered images) #feature
- [x] Thumbnail gallery grid with cursor navigation #feature
- [x] Fuzzy filename search with nucleo-matcher #feature
- [x] Semantic theming system (tokyonight, dark, light, catppuccin, nord) #feature
- [x] Grimoire-inspired UI: bordered titled blocks, search bar, help popup #improvement
- [x] LRU thumbnail cache (bounded 200 entries) #improvement
- [x] Fullscreen neighbor prefetch (background decode of N-1/N+1) #improvement
- [x] Parallel thumbnail loading with rayon #improvement
- [x] turbojpeg fast path for JPEG decoding #improvement
- [x] Optimized dev profile for dependencies #chore
