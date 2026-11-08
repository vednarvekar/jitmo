//! What is selected in File Explorer.
//!
//! The command bar is opened by a hotkey, and by the time the user types, the
//! bar has focus. So the app notes the foreground window at the moment of the
//! hotkey ([`foreground_window`]) and asks Explorer what that window had
//! selected ([`explorer_selection`]). This only reads; it sends no keys or clicks.

use std::path::PathBuf;

/// First line of the script output is the selected item's full path.
pub fn parse_selection(output: &str) -> Option<PathBuf> {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(PathBuf::from(line))
}

/// Most selected items that are read from one window.
pub const MAX_SELECTED: usize = 50;

/// Every non-blank line of the script output is one selected item's full path
/// (at most [`MAX_SELECTED`]).
pub fn parse_selections(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_SELECTED)
        .map(PathBuf::from)
        .collect()
}

/// PowerShell that prints the selected items (one path per line, at most
/// [`MAX_SELECTED`]) of the Explorer window with this handle. Nothing is printed
/// for any other window or an empty selection.
pub fn selection_script(hwnd: isize) -> String {
    format!(
        "$sh = New-Object -ComObject Shell.Application; \
         foreach ($w in $sh.Windows()) {{ \
           if ($w.HWND -eq {hwnd}) {{ \
             $i = $w.Document.SelectedItems(); \
             $n = [Math]::Min($i.Count, {MAX_SELECTED}); \
             for ($k = 0; $k -lt $n; $k++) {{ $i.Item($k).Path }}; \
             break \
           }} \
         }}"
    )
}

#[cfg(windows)]
mod os {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> isize;
    }

    /// Handle of the window the user is in right now (0 if none).
    pub fn foreground_window() -> isize {
        // SAFETY: a plain query with no arguments and no pointers.
        unsafe { GetForegroundWindow() }
    }

    /// The first selected item in the Explorer window `hwnd`, if it is an
    /// Explorer window with something selected. Takes a few hundred
    /// milliseconds, so call it off the UI thread.
    pub fn explorer_selection(hwnd: isize) -> Option<PathBuf> {
        explorer_selection_all(hwnd).into_iter().next()
    }

    /// Every selected item (at most 50) in the Explorer window `hwnd`; empty for
    /// any other window or an empty selection. Same cost as [`explorer_selection`].
    pub fn explorer_selection_all(hwnd: isize) -> Vec<PathBuf> {
        if hwnd == 0 {
            return Vec::new();
        }
        let Ok(out) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(selection_script(hwnd))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        parse_selections(&String::from_utf8_lossy(&out.stdout))
    }
}

#[cfg(windows)]
pub use os::{explorer_selection, explorer_selection_all, foreground_window};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_nonblank_line_is_the_path() {
        assert_eq!(
            parse_selection("\r\nC:\\Users\\me\\a b.pdf\r\n"),
            Some(PathBuf::from("C:\\Users\\me\\a b.pdf"))
        );
    }

    #[test]
    fn empty_output_means_nothing_selected() {
        assert_eq!(parse_selection(""), None);
        assert_eq!(parse_selection("  \r\n"), None);
    }

    #[test]
    fn every_line_is_one_selected_path_capped_at_fifty() {
        assert_eq!(
            parse_selections("C:\\a.pdf\r\n\r\n  C:\\b c.pdf \r\nC:\\d.pdf"),
            vec![
                PathBuf::from("C:\\a.pdf"),
                PathBuf::from("C:\\b c.pdf"),
                PathBuf::from("C:\\d.pdf")
            ]
        );
        assert!(parse_selections(" \r\n").is_empty());
        let many: String = (0..80).map(|i| format!("C:\\f{i}.txt\n")).collect();
        assert_eq!(parse_selections(&many).len(), MAX_SELECTED);
    }

    #[test]
    fn script_prints_all_selected_items() {
        let s = selection_script(7);
        assert!(s.contains("$i.Item($k).Path"));
        assert!(s.contains("Math]::Min($i.Count, 50)"));
    }

    #[test]
    fn script_targets_the_given_window() {
        let s = selection_script(12345);
        assert!(s.contains("$w.HWND -eq 12345"));
        assert!(s.contains("SelectedItems"));
    }

    /// Opens Explorer with a file selected, then reads it back. Needs a desktop
    /// session; run with `--ignored`. It sends no keys or clicks.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_selection_of_a_new_explorer_window() {
        let dir = std::env::temp_dir().join("jitmo-live-explorer");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("live-selected.txt");
        std::fs::write(&file, "x").unwrap();
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", file.display()))
            .status()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        // The new window should be in front now.
        let got = explorer_selection(foreground_window());
        assert_eq!(got, Some(file));
    }
}
