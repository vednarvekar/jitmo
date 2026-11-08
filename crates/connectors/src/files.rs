//! `files`: show, list, open and move files (tasks F1 to F4).
//!
//! Which file is meant is decided before this runs (the resolver turns "the
//! budget sheet" into a path, or asks). This connector only acts on paths.
//!
//! - `reveal` shows the file in Explorer (F1)
//! - `recent` lists recent downloads (F2); reads `text` for "today" or "week"
//! - `open`   opens the file with its default app (F3)
//! - `move`   moves a file into a folder; Undo moves it back (F4)
//!
//! A move never overwrites: if a file with that name is already there, it stops.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{DateTime, Local, TimeZone};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Files {
    downloads: PathBuf,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

impl Files {
    pub fn new(downloads: PathBuf) -> Self {
        Files {
            downloads,
            now: Box::new(Local::now),
        }
    }

    pub fn with_now(mut self, now: impl Fn() -> DateTime<Local> + Send + Sync + 'static) -> Self {
        self.now = Box::new(now);
        self
    }
}

fn arg<'a>(args: &'a Args, name: &str) -> Result<&'a String, ConnError> {
    args.get(name)
        .ok_or_else(|| ConnError::MissingArg(name.to_string()))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The point in time a "recent" request reaches back to.
fn since(text: &str, now: DateTime<Local>) -> (DateTime<Local>, &'static str) {
    let t = text.to_lowercase();
    if t.contains("week") {
        (now - chrono::Duration::days(7), "in the last 7 days")
    } else if t.contains("yesterday") {
        let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
        let start = Local
            .from_local_datetime(&midnight)
            .earliest()
            .unwrap_or(now)
            - chrono::Duration::days(1);
        (start, "since yesterday")
    } else {
        // "today" and the default.
        let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
        (
            Local
                .from_local_datetime(&midnight)
                .earliest()
                .unwrap_or(now),
            "today",
        )
    }
}

fn recent_list(dir: &Path, text: &str, now: DateTime<Local>) -> String {
    let (from, label) = since(text, now);
    let from: SystemTime = from.into();
    let mut found: Vec<(SystemTime, String)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            let partial = name.ends_with(".crdownload") || name.ends_with(".part");
            let modified = meta.modified().ok()?;
            (meta.is_file() && !partial && modified >= from).then_some((modified, name))
        })
        .collect();
    found.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    if found.is_empty() {
        return format!("Nothing was downloaded {label}.");
    }
    let shown: Vec<&str> = found.iter().take(8).map(|(_, n)| n.as_str()).collect();
    let more = found.len().saturating_sub(shown.len());
    let mut out = format!("Downloaded {label}: {}", shown.join(", "));
    if more > 0 {
        out.push_str(&format!(" (and {more} more)"));
    }
    out.push('.');
    out
}

/// Move without ever replacing an existing file.
fn move_file(from: &Path, dir: &Path) -> Result<PathBuf, ConnError> {
    if !from.is_file() {
        return Err(ConnError::Failed(format!(
            "I can't find {} any more.",
            file_name(from)
        )));
    }
    if !dir.is_dir() {
        return Err(ConnError::Failed(format!(
            "I can't find the folder {}.",
            file_name(dir)
        )));
    }
    let to = dir.join(file_name(from));
    if to.exists() {
        return Err(ConnError::Failed(format!(
            "{} already has a file called {}.",
            file_name(dir),
            file_name(from)
        )));
    }
    if fs::rename(from, &to).is_err() {
        // Another drive: copy, then remove the original only if the copy worked.
        fs::copy(from, &to).map_err(|e| ConnError::Failed(format!("Couldn't move it: {e}")))?;
        fs::remove_file(from).map_err(|e| {
            ConnError::Failed(format!("Copied it, but couldn't remove the original: {e}"))
        })?;
    }
    Ok(to)
}

#[cfg(windows)]
fn spawn_detached(program: &str, args: &[&str]) -> Result<(), ConnError> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(program)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map(|_| ())
        .map_err(|e| ConnError::Failed(format!("Couldn't open it: {e}")))
}

#[cfg(windows)]
fn show_in_explorer(path: &Path) -> Result<(), ConnError> {
    spawn_detached("explorer", &[&format!("/select,{}", path.display())])
}

#[cfg(windows)]
fn open_default(path: &Path) -> Result<(), ConnError> {
    // `start` hands the file to whatever app is set to open it.
    spawn_detached("cmd", &["/C", "start", "", &path.display().to_string()])
}

#[cfg(not(windows))]
fn show_in_explorer(_: &Path) -> Result<(), ConnError> {
    Ok(())
}

#[cfg(not(windows))]
fn open_default(_: &Path) -> Result<(), ConnError> {
    Ok(())
}

impl Connector for Files {
    fn id(&self) -> &str {
        "files"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |name: &str, args: &[&str]| ActionSpec {
            name: name.to_string(),
            required_args: args.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("reveal", &["path"]),
            a("recent", &["text"]),
            a("open", &["path"]),
            a("move", &["path", "dir"]),
        ]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let msg = |m: String| Output([("message".to_string(), m)].into());
        match action {
            "reveal" => {
                let path = PathBuf::from(arg(args, "path")?);
                if !path.exists() {
                    return Err(ConnError::Failed(format!(
                        "I can't find {} any more.",
                        file_name(&path)
                    )));
                }
                show_in_explorer(&path)?;
                let folder = path.parent().map(file_name).unwrap_or_default();
                Ok(msg(format!("Found {} in {folder}.", file_name(&path))))
            }
            "recent" => Ok(msg(recent_list(
                &self.downloads,
                arg(args, "text")?,
                (self.now)(),
            ))),
            "open" => {
                let path = PathBuf::from(arg(args, "path")?);
                if !path.exists() {
                    return Err(ConnError::Failed(format!(
                        "I can't find {} any more.",
                        file_name(&path)
                    )));
                }
                open_default(&path)?;
                Ok(msg(format!("Opened {}.", file_name(&path))))
            }
            "move" => {
                let from = PathBuf::from(arg(args, "path")?);
                let dir = PathBuf::from(arg(args, "dir")?);
                let to = move_file(&from, &dir)?;
                Ok(Output(
                    [
                        (
                            "message".to_string(),
                            format!("Moved {} to {}.", file_name(&from), file_name(&dir)),
                        ),
                        ("from".to_string(), from.display().to_string()),
                        ("to".to_string(), to.display().to_string()),
                    ]
                    .into(),
                ))
            }
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        match action {
            "reveal" | "open" => Ok(Path::new(arg(args, "path")?).exists()),
            "recent" => Ok(self.downloads.is_dir()),
            "move" => {
                let from = Path::new(arg(args, "path")?);
                let to = Path::new(arg(args, "dir")?).join(file_name(from));
                Ok(to.is_file() && !from.exists())
            }
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "move" {
            return None;
        }
        let (Some(from), Some(to)) = (output.0.get("from"), output.0.get("to")) else {
            return Some(Err(ConnError::MissingArg("from/to".into())));
        };
        let (from, to) = (Path::new(from), Path::new(to));
        if from.exists() {
            return Some(Err(ConnError::Failed(format!(
                "There is already a file at {}, so I left {} where it is.",
                from.display(),
                file_name(to)
            ))));
        }
        let back = from.parent().unwrap_or(Path::new("."));
        Some(move_file(to, back).map(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jitmo-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn args(pairs: &[(&str, &str)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn move_then_undo_puts_the_file_back() {
        let root = temp("move");
        let (src, dst) = (root.join("inbox"), root.join("Invoices"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.pdf"), "x").unwrap();
        let c = Files::new(root.clone());
        let a = args(&[
            ("path", &src.join("a.pdf").display().to_string()),
            ("dir", &dst.display().to_string()),
        ]);
        let out = c.execute("move", &a).unwrap();
        assert_eq!(out.0["message"], "Moved a.pdf to Invoices.");
        assert!(dst.join("a.pdf").is_file() && !src.join("a.pdf").exists());
        assert!(c.check("move", &a).unwrap());
        c.undo("move", &out).unwrap().unwrap();
        assert!(src.join("a.pdf").is_file() && !dst.join("a.pdf").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_move_never_overwrites() {
        let root = temp("overwrite");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/x.txt"), "one").unwrap();
        fs::write(root.join("b/x.txt"), "two").unwrap();
        let c = Files::new(root.clone());
        let r = c.execute(
            "move",
            &args(&[
                ("path", &root.join("a/x.txt").display().to_string()),
                ("dir", &root.join("b").display().to_string()),
            ]),
        );
        assert!(matches!(r, Err(ConnError::Failed(m)) if m.contains("already has a file")));
        assert_eq!(fs::read_to_string(root.join("b/x.txt")).unwrap(), "two");
        assert!(root.join("a/x.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn undo_refuses_when_the_original_spot_is_taken() {
        let root = temp("undo-taken");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/x.txt"), "one").unwrap();
        let c = Files::new(root.clone());
        let out = c
            .execute(
                "move",
                &args(&[
                    ("path", &root.join("a/x.txt").display().to_string()),
                    ("dir", &root.join("b").display().to_string()),
                ]),
            )
            .unwrap();
        fs::write(root.join("a/x.txt"), "someone else's").unwrap();
        assert!(c.undo("move", &out).unwrap().is_err());
        assert!(root.join("b/x.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_file_and_folder_are_clear_errors() {
        let root = temp("missing");
        let c = Files::new(root.clone());
        let gone = c.execute(
            "open",
            &args(&[("path", &root.join("nope.docx").display().to_string())]),
        );
        assert!(matches!(gone, Err(ConnError::Failed(m)) if m.contains("can't find nope.docx")));
        fs::write(root.join("f.txt"), "x").unwrap();
        let nodir = c.execute(
            "move",
            &args(&[
                ("path", &root.join("f.txt").display().to_string()),
                ("dir", &root.join("Nowhere").display().to_string()),
            ]),
        );
        assert!(matches!(nodir, Err(ConnError::Failed(m)) if m.contains("can't find the folder")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recent_lists_todays_downloads_newest_first() {
        let root = temp("recent");
        fs::write(root.join("old.txt"), "x").unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(5 * 24 * 3600);
        fs::File::options()
            .write(true)
            .open(root.join("old.txt"))
            .unwrap()
            .set_modified(old_time)
            .unwrap();
        fs::write(root.join("new.pdf"), "x").unwrap();
        fs::write(root.join("half.crdownload"), "x").unwrap();
        let c = Files::new(root.clone());
        let out = c
            .execute("recent", &args(&[("text", "What did I download today?")]))
            .unwrap();
        assert_eq!(out.0["message"], "Downloaded today: new.pdf.");
        let week = c
            .execute("recent", &args(&[("text", "downloads this week")]))
            .unwrap();
        assert!(week.0["message"].contains("new.pdf"));
        assert!(
            week.0["message"].contains("old.txt"),
            "5 days old is inside a week"
        );
        assert!(
            !out.0["message"].contains("old.txt"),
            "but not inside today"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_downloaded_says_so() {
        let root = temp("empty");
        let c = Files::new(root.clone());
        let out = c
            .execute("recent", &args(&[("text", "what did i download today")]))
            .unwrap();
        assert_eq!(out.0["message"], "Nothing was downloaded today.");
        let _ = fs::remove_dir_all(root);
    }
}
