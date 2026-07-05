use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::path::{Path, PathBuf};

pub struct PickerEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_parent: bool,
}

pub struct PickerState {
    pub current_dir: PathBuf,
    pub entries: Vec<PickerEntry>,
    pub filter: String,
    pub filtered_indices: Vec<usize>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub filter_active: bool,
    pub error: Option<String>,
}

impl PickerState {
    pub fn new(dir: PathBuf) -> Self {
        let mut state = Self {
            current_dir: dir,
            entries: Vec::new(),
            filter: String::new(),
            filtered_indices: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            filter_active: false,
            error: None,
        };
        state.load_dir();
        state
    }

    pub fn load_dir(&mut self) {
        self.entries.clear();
        self.filter.clear();
        self.error = None;

        if let Some(parent) = self.current_dir.parent() {
            self.entries.push(PickerEntry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                is_parent: true,
            });
        }

        match std::fs::read_dir(&self.current_dir) {
            Ok(read_dir) => {
                let mut dirs: Vec<PickerEntry> = Vec::new();
                for entry in read_dir.flatten() {
                    let Ok(ft) = entry.file_type() else {
                        continue;
                    };
                    if !ft.is_dir() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    dirs.push(PickerEntry {
                        name,
                        path: entry.path(),
                        is_parent: false,
                    });
                }
                dirs.sort_by(|a, b| a.name.cmp(&b.name));
                self.entries.extend(dirs);
            }
            Err(e) => {
                self.error = Some(format!("{}: {}", self.current_dir.display(), e));
            }
        }

        self.rebuild_filter();
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn rebuild_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.entries.len()).collect();
            return;
        }

        let pattern = Pattern::parse(&self.filter, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut buf = Vec::new();

        let mut scored: Vec<(usize, u32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let haystack = Utf32Str::new(&e.name, &mut buf);
                pattern.score(haystack, &mut matcher).map(|s| (i, s))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.filtered_indices = scored.into_iter().map(|(i, _)| i).collect();
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn selected(&self) -> Option<&PickerEntry> {
        let &real = self.filtered_indices.get(self.cursor)?;
        self.entries.get(real)
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.cursor = (self.cursor + 1).min(self.filtered_indices.len() - 1);
        }
    }

    pub fn move_first(&mut self) {
        self.cursor = 0;
    }

    pub fn move_last(&mut self) {
        self.cursor = self.filtered_indices.len().saturating_sub(1);
    }

    pub fn move_page_up(&mut self, page: usize) {
        self.cursor = self.cursor.saturating_sub(page.max(1));
    }

    pub fn move_page_down(&mut self, page: usize) {
        let max = self.filtered_indices.len().saturating_sub(1);
        self.cursor = (self.cursor + page.max(1)).min(max);
    }

    /// Descend into the currently selected directory. Returns the new current path if changed.
    pub fn enter_selected(&mut self) -> Option<PathBuf> {
        let path = self.selected()?.path.clone();
        self.current_dir = path.clone();
        self.load_dir();
        Some(path)
    }

    /// Go up to the parent directory. Returns the new current path if changed.
    pub fn ascend(&mut self) -> Option<PathBuf> {
        let parent = self.current_dir.parent()?.to_path_buf();
        self.current_dir = parent.clone();
        self.load_dir();
        Some(parent)
    }

    pub fn adjust_scroll(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + visible_height {
            self.scroll_offset = self.cursor - visible_height + 1;
        }
    }

    pub fn visible_slice(&self, visible_height: usize) -> &[usize] {
        let start = self.scroll_offset;
        let end = (start + visible_height).min(self.filtered_indices.len());
        &self.filtered_indices[start..end]
    }
}

pub fn initial_picker_dir(images: &[PathBuf]) -> PathBuf {
    let candidate = images
        .first()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    canonicalize_or_self(&candidate)
}

pub fn canonicalize_or_self(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
