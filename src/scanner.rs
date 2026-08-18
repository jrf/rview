use crate::image_list::SharedImageList;
use std::path::{Path, PathBuf};
use std::thread;

const BATCH_SIZE: usize = 64;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "avif",
];

#[cfg(feature = "video")]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm", "m4v"];

pub(crate) fn is_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    let Some(ext) = ext else { return false };
    let ext = ext.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    #[cfg(feature = "video")]
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    false
}

pub fn spawn(paths: Vec<PathBuf>, list: SharedImageList) {
    thread::spawn(move || {
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for path in paths {
            if path.is_dir() {
                scan_dir(&path, &list, &mut batch);
            } else if is_supported(&path) {
                batch.push(path);
                if batch.len() >= BATCH_SIZE {
                    flush_batch(&list, &mut batch);
                }
            } else {
                list.push_error(format!("unsupported media format: {}", path.display()));
            }
        }

        if !batch.is_empty() {
            flush_batch(&list, &mut batch);
        }

        list.mark_complete();
    });
}

fn scan_dir(dir: &Path, list: &SharedImageList, batch: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            list.push_error(format!("{}: {error}", dir.display()));
            return;
        }
    };

    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                match entry.file_type() {
                    Ok(file_type) => {
                        // `file_type` comes from the directory entry and does not
                        // follow symlinks, so a link pointing at a file reports
                        // neither `is_file()` nor `is_dir()`. Resolve the target
                        // with `metadata()` (which follows symlinks) so linked
                        // media is picked up too.
                        let is_regular_file = if file_type.is_symlink() {
                            match std::fs::metadata(&path) {
                                Ok(target) => target.is_file(),
                                Err(error) => {
                                    list.push_error(format!("{}: {error}", path.display()));
                                    false
                                }
                            }
                        } else {
                            file_type.is_file()
                        };

                        if is_regular_file && is_supported(&path) {
                            batch.push(path);
                            if batch.len() >= BATCH_SIZE {
                                flush_batch(list, batch);
                            }
                        }
                    }
                    Err(error) => list.push_error(format!("{}: {error}", path.display())),
                }
            }
            Err(error) => list.push_error(format!("{}: {error}", dir.display())),
        }
    }
}

fn flush_batch(list: &SharedImageList, batch: &mut Vec<PathBuf>) {
    batch.sort_by(|left, right| {
        left.file_name()
            .cmp(&right.file_name())
            .then_with(|| left.cmp(right))
    });
    list.push_batch(std::mem::take(batch));
    *batch = Vec::with_capacity(BATCH_SIZE);
}

#[cfg(test)]
mod tests {
    use super::{is_supported, scan_dir};
    use crate::image_list::SharedImageList;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(is_supported(Path::new("synthetic.PNG")));
        assert!(is_supported(Path::new("synthetic.jpeg")));
    }

    #[test]
    fn unsupported_extensions_are_rejected() {
        assert!(!is_supported(Path::new("synthetic.txt")));
        assert!(!is_supported(Path::new("synthetic")));
    }

    #[cfg(unix)]
    #[test]
    fn scan_dir_follows_symlinked_files() {
        let root = std::env::temp_dir().join(format!(
            "rview-synthetic-scanner-symlink-{}",
            std::process::id()
        ));
        let source = root.join("source");
        let gallery = root.join("gallery");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&gallery).unwrap();

        let target = source.join("real.png");
        fs::write(&target, b"synthetic").unwrap();
        let link = gallery.join("linked.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let list = SharedImageList::new();
        let mut batch: Vec<PathBuf> = Vec::new();
        scan_dir(&gallery, &list, &mut batch);

        assert_eq!(batch, vec![link]);
        let _ = fs::remove_dir_all(&root);
    }
}
