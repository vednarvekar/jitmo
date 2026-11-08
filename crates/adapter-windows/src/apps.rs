//! Finding, launching and detecting apps (task A1).
//!
//! Apps come from the same list as the Start menu (`Get-StartApps`), which
//! includes Microsoft Store apps such as the Store build of Spotify. An app is
//! launched by its Start-menu id, never by running text the user typed.

/// One launchable app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Display name, like "Spotify".
    pub name: String,
    /// Start-menu id, like `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`.
    pub app_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Found(AppEntry),
    NotFound,
    /// Several different apps fit; carries their names.
    Ambiguous(Vec<String>),
}

/// Lowercase letters and digits only, so "Visual Studio Code" and
/// "visual-studio-code" compare equal.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Pick the app the user meant: an exact name first, then a single app whose
/// name contains the request. Several matches are reported, never guessed.
pub fn find_app(entries: &[AppEntry], wanted: &str) -> Lookup {
    let w = normalize(wanted);
    if w.is_empty() {
        return Lookup::NotFound;
    }
    let usable = || {
        entries
            .iter()
            .filter(|e| !normalize(&e.name).contains("uninstall"))
    };
    if let Some(e) = usable().find(|e| normalize(&e.name) == w) {
        return Lookup::Found(e.clone());
    }
    let mut hits: Vec<&AppEntry> = Vec::new();
    for e in usable().filter(|e| normalize(&e.name).contains(&w)) {
        if !hits
            .iter()
            .any(|h| normalize(&h.name) == normalize(&e.name))
        {
            hits.push(e);
        }
    }
    match hits.as_slice() {
        [] => Lookup::NotFound,
        [one] => Lookup::Found((*one).clone()),
        many => Lookup::Ambiguous(many.iter().map(|e| e.name.clone()).collect()),
    }
}

/// Parse `Name<TAB>AppID` lines (the output of the app-listing command).
pub fn parse_app_list(text: &str) -> Vec<AppEntry> {
    text.lines()
        .filter_map(|line| {
            let (name, id) = line.trim_end_matches('\r').split_once('\t')?;
            let (name, id) = (name.trim(), id.trim());
            (!name.is_empty() && !id.is_empty()).then(|| AppEntry {
                name: name.into(),
                app_id: id.into(),
            })
        })
        .collect()
}

/// Does a running process (by image name, like `Spotify.exe`) belong to the app the user named?
pub fn process_matches(image_name: &str, wanted: &str) -> bool {
    let lower = image_name.to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    let (p, w) = (normalize(stem), normalize(wanted));
    if p.is_empty() || w.is_empty() {
        return false;
    }
    p == w || p.contains(&w) || (p.len() >= 4 && w.contains(&p))
}

/// Does a window title belong to the app the user named?
pub fn title_matches(title: &str, wanted: &str) -> bool {
    let (t, w) = (normalize(title), normalize(wanted));
    !t.is_empty() && !w.is_empty() && t.contains(&w)
}

/// Parse `tasklist /FO CSV /NH [/V]` output into (image name, window title) pairs.
/// The title is empty when the verbose column is absent or not set.
pub fn parse_tasklist(csv: &str) -> Vec<(String, String)> {
    csv.lines()
        .filter_map(|line| {
            let line = line.trim();
            let inner = line.strip_prefix('"')?.strip_suffix('"')?;
            let fields: Vec<&str> = inner.split("\",\"").collect();
            let image = fields.first()?.to_string();
            let title = fields
                .get(8)
                .map(|t| t.to_string())
                .filter(|t| t != "N/A")
                .unwrap_or_default();
            Some((image, title))
        })
        .collect()
}

#[cfg(windows)]
mod os {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Mutex;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Start-menu apps. Listing takes about a second, so it is cached and only
    /// refreshed when an app is not found (it may have just been installed).
    static CACHE: Mutex<Option<Vec<AppEntry>>> = Mutex::new(None);

    fn list_start_apps() -> Result<Vec<AppEntry>, String> {
        let out = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-StartApps | ForEach-Object { $_.Name + \"`t\" + $_.AppID }",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("could not list apps: {e}"))?;
        Ok(parse_app_list(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Look an app up by name, refreshing the app list once if it is not found.
    pub fn lookup_app(wanted: &str) -> Result<Lookup, String> {
        let mut cache = CACHE
            .lock()
            .map_err(|_| "app list is unavailable".to_string())?;
        if let Some(entries) = cache.as_ref() {
            let found = find_app(entries, wanted);
            if found != Lookup::NotFound {
                return Ok(found);
            }
        }
        let fresh = list_start_apps()?;
        let result = find_app(&fresh, wanted);
        *cache = Some(fresh);
        Ok(result)
    }

    /// Start an app. Returns as soon as Windows has been asked to start it.
    pub fn launch(app: &AppEntry) -> Result<(), String> {
        Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{}", app.app_id))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start {}: {e}", app.name))
    }

    fn tasklist(verbose: bool) -> Vec<(String, String)> {
        let mut cmd = Command::new("tasklist.exe");
        cmd.args(["/FO", "CSV", "/NH"]);
        if verbose {
            cmd.arg("/V");
        }
        match cmd.creation_flags(CREATE_NO_WINDOW).output() {
            Ok(out) => parse_tasklist(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => Vec::new(),
        }
    }

    /// Is a process or window for this app running right now?
    /// `deep` also compares window titles (slower).
    pub fn is_app_running(wanted: &str, deep: bool) -> bool {
        tasklist(deep).iter().any(|(image, title)| {
            process_matches(image, wanted) || (deep && title_matches(title, wanted))
        })
    }
}

#[cfg(windows)]
pub use os::{is_app_running, launch, lookup_app};

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<AppEntry> {
        parse_app_list(
            "Spotify\tSpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify\r\n\
             Notepad\tMicrosoft.WindowsNotepad_8wekyb3d8bbwe!App\r\n\
             Visual Studio Code\tCode.exe\r\n\
             Microsoft Word\tWord\r\n\
             Microsoft Excel\tExcel\r\n\
             Uninstall Spotify\tuninstall.exe\r\n\
             Notepad\tduplicate!App\r\n\
             \r\n\
             broken line without a tab\r\n",
        )
    }

    #[test]
    fn parses_the_app_list_and_skips_bad_lines() {
        let e = entries();
        assert_eq!(e.len(), 7);
        assert_eq!(
            e[0],
            AppEntry {
                name: "Spotify".into(),
                app_id: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into()
            }
        );
    }

    #[test]
    fn exact_name_wins_regardless_of_case_and_punctuation() {
        let Lookup::Found(e) = find_app(&entries(), "spotify") else {
            panic!()
        };
        assert_eq!(e.name, "Spotify");
        assert!(
            matches!(find_app(&entries(), "visual-studio-code"), Lookup::Found(e) if e.name == "Visual Studio Code")
        );
    }

    #[test]
    fn uninstallers_are_never_picked() {
        // "uninstall spotify" would otherwise be the only match for "uninstall".
        assert_eq!(find_app(&entries(), "uninstall"), Lookup::NotFound);
    }

    #[test]
    fn one_partial_match_is_used() {
        assert!(
            matches!(find_app(&entries(), "code"), Lookup::Found(e) if e.name == "Visual Studio Code")
        );
    }

    #[test]
    fn several_partial_matches_are_reported_not_guessed() {
        assert_eq!(
            find_app(&entries(), "microsoft"),
            Lookup::Ambiguous(vec!["Microsoft Word".into(), "Microsoft Excel".into()])
        );
    }

    #[test]
    fn unknown_or_empty_is_not_found() {
        assert_eq!(find_app(&entries(), "telegram"), Lookup::NotFound);
        assert_eq!(find_app(&entries(), "  !! "), Lookup::NotFound);
    }

    #[test]
    fn user_text_is_never_part_of_what_gets_launched() {
        // The launched id always comes from the app list.
        let Lookup::Found(e) = find_app(&entries(), "spotify & calc.exe") else {
            return; // not found is fine: nothing is launched
        };
        assert!(entries().contains(&e));
    }

    #[test]
    fn process_names_match_the_app_name() {
        assert!(process_matches("Spotify.exe", "spotify"));
        assert!(process_matches("chrome.exe", "Google Chrome"));
        assert!(process_matches("Code.exe", "Visual Studio Code"));
        assert!(!process_matches("svchost.exe", "spotify"));
        assert!(!process_matches("exe", "spotify"));
        assert!(
            !process_matches("cmd.exe", "Command Prompt Tools"),
            "short names must not match loosely"
        );
    }

    #[test]
    fn window_titles_match_the_app_name() {
        assert!(title_matches("Spotify Free", "spotify"));
        assert!(!title_matches("N/A", "spotify"));
        assert!(!title_matches("", "spotify"));
    }

    #[test]
    fn parses_plain_and_verbose_tasklist_output() {
        let plain = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"Spotify.exe\",\"4242\",\"Console\",\"1\",\"90,000 K\"\r\n";
        let rows = parse_tasklist(plain);
        assert_eq!(rows[1], ("Spotify.exe".to_string(), String::new()));

        let verbose = "\"Spotify.exe\",\"4242\",\"Console\",\"1\",\"90,000 K\",\"Running\",\"PC\\me\",\"0:00:03\",\"Spotify Free\"\r\n\
                       \"svchost.exe\",\"8\",\"Services\",\"0\",\"1 K\",\"Running\",\"SYSTEM\",\"0:00:01\",\"N/A\"\r\n";
        let rows = parse_tasklist(verbose);
        assert_eq!(rows[0].1, "Spotify Free");
        assert_eq!(rows[1].1, "");
    }
}
