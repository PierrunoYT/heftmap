//! Parallel filesystem scanner.
//!
//! Directories are walked concurrently with rayon into an owned intermediate tree,
//! which is then flattened into a [`Tree`] with aggregated sizes, sorted by size.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use rayon::prelude::*;

use crate::listing::{Kind, list_dir};
use crate::tree::{Entry, NodeId, Tree, TreeBuilder, unix_secs};

/// Live counters shared between the scanner and the UI. Also carries the cancel flag.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    cancel: AtomicBool,
    current: Mutex<String>,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    /// Directory most recently entered by any worker.
    pub fn current(&self) -> String {
        self.current.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_current(&self, path: &Path) {
        // Best effort: never make workers wait on the UI's read.
        if let Ok(mut current) = self.current.try_lock() {
            current.clear();
            current.push_str(&path.to_string_lossy());
        }
    }

    fn error(&self) {
        self.errors.fetch_add(1, Relaxed);
    }
}

/// A file inside [`RawDir`]; its name is a slice of the directory's `names` buffer.
struct RawFile {
    size: u64,
    allocated: u64,
    mtime: i64,
    name_end: u32,
}

/// Scan result for one directory, kept small because this intermediate tree coexists
/// with the final [`Tree`] while flattening: at most three allocations per directory,
/// none per file.
struct RawDir {
    /// The directory's own name, followed by the names of its files.
    names: Box<str>,
    files: Box<[RawFile]>,
    dirs: Box<[RawDir]>,
    mtime: i64,
    name_len: u32,
    error: bool,
}

impl RawDir {
    fn name(&self) -> &str {
        &self.names[..self.name_len as usize]
    }

    fn file_names(&self) -> impl Iterator<Item = &str> {
        let mut start = self.name_len as usize;
        self.files.iter().map(move |f| {
            let name = &self.names[start..f.name_end as usize];
            start = f.name_end as usize;
            name
        })
    }
}

struct Walker<'a> {
    progress: &'a Progress,
    /// Device of the scan root; subdirectories on other filesystems are skipped (Unix only).
    root_dev: Option<u64>,
}

/// Scans `root` recursively. Returns `Ok(None)` if cancelled through `progress`.
///
/// Symlinks and junctions are not followed and contribute nothing, so no data is
/// counted twice through them. Unreadable directories are kept with `error` set.
pub fn scan(root: &Path, progress: &Progress) -> io::Result<Option<Tree>> {
    let started = Instant::now();
    // No effect unless elevated; then protected folders become listable.
    crate::elevation::enable_backup_privilege();
    let meta = fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a directory"));
    }

    #[cfg(unix)]
    let root_dev = Some(std::os::unix::fs::MetadataExt::dev(&meta));
    #[cfg(not(unix))]
    let root_dev = None;
    let walker = Walker { progress, root_dev };

    // Deep hierarchies plus work-stealing nest frames, so give workers generous stacks.
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(16 << 20)
        .thread_name(|i| format!("scan-{i}"))
        .build()
        .map_err(io::Error::other)?;

    pool.install(|| {
        let raw = walker.dir(root, root.display().to_string(), unix_secs(meta.modified().ok()));
        if progress.is_cancelled() {
            return Ok(None);
        }
        let capacity = progress.files.load(Relaxed) + progress.dirs.load(Relaxed);
        if capacity >= u64::from(NodeId::MAX) {
            return Err(io::Error::other("too many entries (over 4 billion)"));
        }
        let mut builder = TreeBuilder::new(
            capacity as usize,
            Entry { name: raw.name(), size: 0, allocated: 0, mtime: raw.mtime, is_dir: true, error: raw.error },
        );
        flatten(raw, crate::tree::ROOT, &mut builder);
        Ok(Some(builder.finish(root.to_path_buf(), progress.errors.load(Relaxed), started.elapsed())))
    })
}

impl Walker<'_> {
    fn dir(&self, path: &Path, name: String, mtime: i64) -> RawDir {
        let p = self.progress;
        let name_len = name.len() as u32;
        let mut names = name;
        let mut files = Vec::new();
        let done = |names: String, files: Vec<RawFile>, dirs: Vec<RawDir>, error| RawDir {
            names: names.into_boxed_str(),
            files: files.into_boxed_slice(),
            dirs: dirs.into_boxed_slice(),
            mtime,
            name_len,
            error,
        };
        if p.is_cancelled() {
            return done(names, files, Vec::new(), false);
        }
        p.dirs.fetch_add(1, Relaxed);
        p.set_current(path);

        let mut subdirs: Vec<(PathBuf, String, i64)> = Vec::new();
        let mut bytes = 0;
        let listed = list_dir(path, self.root_dev, |item| match item.kind {
            Kind::Dir(child) => subdirs.push((child, item.name.to_owned(), item.mtime)),
            Kind::File => {
                bytes += item.size;
                names.push_str(item.name);
                files.push(RawFile {
                    size: item.size,
                    allocated: item.allocated,
                    mtime: item.mtime,
                    name_end: names.len() as u32,
                });
            }
        });
        let error = listed.unwrap_or(true);
        if error {
            p.error();
        }
        p.files.fetch_add(files.len() as u64, Relaxed);
        p.bytes.fetch_add(bytes, Relaxed);

        let dirs = subdirs
            .into_par_iter()
            .map(|(path, name, mtime)| self.dir(&path, name, mtime))
            .collect();
        done(names, files, dirs, error)
    }
}

/// Moves `raw`'s contents below node `id`, consuming (and freeing) it as it goes.
fn flatten(raw: RawDir, id: NodeId, builder: &mut TreeBuilder) {
    let dirs = raw
        .dirs
        .iter()
        .map(|d| Entry { name: d.name(), size: 0, allocated: 0, mtime: d.mtime, is_dir: true, error: d.error });
    let files = raw
        .file_names()
        .zip(&raw.files)
        .map(|(name, f)| Entry {
            name,
            size: f.size,
            allocated: f.allocated,
            mtime: f.mtime,
            is_dir: false,
            error: false,
        });
    let first = builder.add_children(id, dirs.chain(files));
    let RawDir { dirs, .. } = raw; // file names are no longer needed
    for (k, sub) in dirs.into_vec().into_iter().enumerate() {
        flatten(sub, first + k as NodeId, builder);
    }
    builder.finish_dir(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::ROOT;

    #[test]
    fn aggregates_sizes_and_counts() {
        let root = std::env::temp_dir().join(format!("heftmap-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        // Above the NTFS resident-file limit, so allocated sizes order like logical sizes.
        fs::write(root.join("top.bin"), vec![0u8; 10_000]).unwrap();
        fs::write(root.join("a/one.TXT"), vec![0u8; 100_000]).unwrap();
        fs::write(root.join("a/b/two.txt"), vec![0u8; 1_000_000]).unwrap();
        fs::write(root.join("a/b/ünïcödé.txt"), vec![0u8; 1]).unwrap();

        let tree = scan(&root, &Progress::default()).unwrap().unwrap();
        fs::remove_dir_all(&root).unwrap();

        let r = tree.node(ROOT);
        assert_eq!((r.size, r.files, r.dirs), (1_110_001, 4, 3));
        // Most disk space first: a, top.bin, empty.
        let names: Vec<&str> = tree.children(ROOT).iter().map(|&c| tree.name(c)).collect();
        assert_eq!(names, ["a", "top.bin", "empty"]);

        let a = tree.children(ROOT)[0];
        assert_eq!((tree.node(a).size, tree.node(a).files, tree.node(a).dirs), (1_100_001, 3, 1));
        assert_eq!(tree.path(a), root.join("a"));
        let b = tree.children(a)[0];
        let b_names: Vec<&str> = tree.children(b).iter().map(|&c| tree.name(c)).collect();
        assert_eq!(b_names, ["two.txt", "ünïcödé.txt"]);
        assert_eq!(tree.path(tree.children(b)[1]), root.join("a/b/ünïcödé.txt"));

        let stats = tree.subtree_stats(ROOT, 2);
        let txt = stats.extensions.iter().find(|e| e.ext == "txt").unwrap();
        assert_eq!((txt.bytes, txt.files), (tree.node(a).allocated, 3), "extensions are case-folded");
        let largest: Vec<&str> = stats.largest.iter().map(|&n| tree.name(n)).collect();
        assert_eq!(largest, ["two.txt", "one.TXT"]);
    }

    #[test]
    fn reports_allocated_size() {
        let root = std::env::temp_dir().join(format!("heftmap-alloc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // Large enough to live outside the NTFS MFT, so it occupies whole clusters/blocks.
        fs::write(root.join("big.bin"), vec![1u8; 100_000]).unwrap();

        let tree = scan(&root, &Progress::default()).unwrap().unwrap();
        fs::remove_dir_all(&root).unwrap();

        let file = tree.node(tree.children(ROOT)[0]);
        assert_eq!(file.size, 100_000);
        assert!(file.allocated >= 100_000 && file.allocated.is_multiple_of(512), "allocated {}", file.allocated);
        assert_eq!(tree.node(ROOT).allocated, file.allocated);
    }
}
