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

/// PowerShell that prints the first selected item of the Explorer window with
/// this handle. Nothing is printed for any other window or an empty selection.
pub fn selection_script(hwnd: isize) -> String {
    format!(
        "$sh = New-Object -ComObject Shell.Application; \
         foreach ($w in $sh.Windows()) {{ \
           if ($w.HWND -eq {hwnd}) {{ \
             $i = $w.Document.SelectedItems(); \
             if ($i.Count -ge 1) {{ $i.Item(0).Path; break }} \
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
        if hwnd == 0 {
            return None;
        }
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(selection_script(hwnd))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_selection(&String::from_utf8_lossy(&out.stdout))
    }
}

#[cfg(windows)]
pub use os::{explorer_selection, foreground_window};

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
