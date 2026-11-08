//! The real context service: what is on disk and what was selected.
//!
//! The file index is a bounded walk of a few folders (Downloads, Documents,
//! Desktop by default), done when a request needs it. Nothing is stored and
//! nothing leaves the device. The selected file is a snapshot the app sets when
//! the bar is opened (Explorer's selection), because by the time the user has
//! typed, the bar itself has focus.

use crate::resolver::{ContextSource, FileRef};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// Stop walking after this many files, so a huge folder cannot stall a request.
const MAX_FILES: usize = 20_000;
/// How deep below a root to look.
const MAX_DEPTH: usize = 3;
/// Folders that are never worth searching.
const SKIP_DIRS: [&str; 4] = ["node_modules", ".git", "AppData", "target"];

/// Shared place where the app puts the current Explorer selection.
#[derive(Clone, Default)]
pub struct SelectionSnapshot(Arc<Mutex<Vec<PathBuf>>>);

impl SelectionSnapshot {
    /// Remember a single selected item, or none.
    pub fn set(&self, path: Option<PathBuf>) {
        self.set_all(path.into_iter().collect());
    }

    /// Remember every selected item.
    pub fn set_all(&self, paths: Vec<PathBuf>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = paths;
    }

    fn get_all(&self) -> Vec<PathBuf> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub struct FsContext {
    downloads: PathBuf,
    roots: Vec<PathBuf>,
    selection: SelectionSnapshot,
}

impl FsContext {
    pub fn new(downloads: PathBuf, roots: Vec<PathBuf>, selection: SelectionSnapshot) -> Self {
        FsContext {
            downloads,
            roots,
            selection,
        }
    }

    /// Downloads, Documents and Desktop of the current user.
    pub fn for_current_user(selection: SelectionSnapshot) -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_default();
        let downloads = home.join("Downloads");
        FsContext::new(
            downloads.clone(),
            vec![downloads, home.join("Documents"), home.join("Desktop")],
            selection,
        )
    }
}

fn file_ref(path: &Path) -> Option<FileRef> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some(FileRef {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        modified,
    })
}

/// Half-finished downloads are not files the user means.
fn is_partial(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".crdownload") || n.ends_with(".part") || n.ends_with(".tmp")
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<FileRef>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&path, depth + 1, out);
            }
        } else if kind.is_file() && !is_partial(&name) {
            if let Some(f) = file_ref(&path) {
                out.push(f);
            }
        }
    }
}

fn walk_dirs(dir: &Path, depth: usize, out: &mut Vec<FileRef>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir || name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        out.push(FileRef {
            name,
            path: path.to_string_lossy().into_owned(),
            modified: 0,
        });
        if depth < MAX_DEPTH {
            walk_dirs(&path, depth + 1, out);
        }
    }
}

impl ContextSource for FsContext {
    fn selected_file(&self) -> Option<FileRef> {
        self.selection.get_all().first().and_then(|p| file_ref(p))
    }

    fn selected_files(&self) -> Vec<FileRef> {
        self.selection
            .get_all()
            .iter()
            .filter_map(|p| file_ref(p))
            .collect()
    }

    fn recent_downloads(&self, limit: usize) -> Vec<FileRef> {
        let mut files = Vec::new();
        walk(&self.downloads, MAX_DEPTH, &mut files); // top level only
        files.sort_by_key(|f| std::cmp::Reverse(f.modified));
        files.truncate(limit);
        files
    }

    fn searchable_files(&self) -> Vec<FileRef> {
        let mut files = Vec::new();
        for root in &self.roots {
            walk(root, 0, &mut files);
        }
        files
    }

    fn searchable_folders(&self) -> Vec<FileRef> {
        let mut dirs = Vec::new();
        for root in &self.roots {
            walk_dirs(root, 0, &mut dirs);
        }
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jitmo-ctx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn indexes_nested_files_and_skips_junk() {
        let root = temp("index");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("node_modules/x")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("a/b/deep.txt"), "x").unwrap();
        fs::write(root.join("top.pdf"), "x").unwrap();
        fs::write(root.join("half.crdownload"), "x").unwrap();
        fs::write(root.join("node_modules/x/lib.js"), "x").unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        let ctx = FsContext::new(
            root.clone(),
            vec![root.clone()],
            SelectionSnapshot::default(),
        );
        let mut names: Vec<String> = ctx.searchable_files().into_iter().map(|f| f.name).collect();
        names.sort();
        assert_eq!(names, ["deep.txt", "top.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recent_downloads_are_newest_first_and_top_level_only() {
        let root = temp("recent");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("old.txt"), "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(root.join("new.txt"), "x").unwrap();
        fs::write(root.join("sub/nested.txt"), "x").unwrap();
        let ctx = FsContext::new(root.clone(), vec![], SelectionSnapshot::default());
        let names: Vec<String> = ctx
            .recent_downloads(5)
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names[0], "new.txt");
        assert!(names.contains(&"old.txt".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn several_selected_files_are_all_returned_and_missing_ones_dropped() {
        let root = temp("multi");
        for n in ["a.pdf", "b.pdf", "c.pdf"] {
            fs::write(root.join(n), "x").unwrap();
        }
        let snap = SelectionSnapshot::default();
        let ctx = FsContext::new(root.clone(), vec![], snap.clone());
        snap.set_all(vec![
            root.join("a.pdf"),
            root.join("gone.pdf"),
            root.join("b.pdf"),
            root.join("c.pdf"),
        ]);
        let names: Vec<String> = ctx.selected_files().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["a.pdf", "b.pdf", "c.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selection_comes_from_the_snapshot_and_must_still_exist() {
        let root = temp("sel");
        fs::write(root.join("picked.docx"), "x").unwrap();
        let snap = SelectionSnapshot::default();
        let ctx = FsContext::new(root.clone(), vec![], snap.clone());
        assert!(ctx.selected_file().is_none());
        snap.set(Some(root.join("picked.docx")));
        assert_eq!(ctx.selected_file().unwrap().name, "picked.docx");
        snap.set(Some(root.join("gone.docx")));
        assert!(ctx.selected_file().is_none());
        assert!(ctx.selected_files().is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
