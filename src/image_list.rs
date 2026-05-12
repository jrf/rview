use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Inner {
    paths: Vec<PathBuf>,
    filenames: Vec<String>,
}

#[derive(Clone)]
pub struct SharedImageList {
    inner: Arc<Mutex<Inner>>,
    len: Arc<AtomicUsize>,
    complete: Arc<AtomicBool>,
}

impl SharedImageList {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                paths: Vec::new(),
                filenames: Vec::new(),
            })),
            len: Arc::new(AtomicUsize::new(0)),
            complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn push_batch(&self, paths: Vec<PathBuf>) {
        let mut inner = self.inner.lock().unwrap();
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            inner.paths.push(path);
            inner.filenames.push(name);
        }
        self.len.store(inner.paths.len(), Ordering::Release);
    }

    pub fn mark_complete(&self) {
        self.complete.store(true, Ordering::Release);
    }

    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn drain_since(&self, known_len: usize) -> (Vec<PathBuf>, Vec<String>) {
        let inner = self.inner.lock().unwrap();
        let paths = inner.paths[known_len..].to_vec();
        let filenames = inner.filenames[known_len..].to_vec();
        (paths, filenames)
    }
}
