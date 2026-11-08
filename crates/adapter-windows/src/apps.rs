//! Finding, launching and detecting apps (task A1).
//!
//! Apps come from the same list as the Start menu (`Get-StartApps`), which
//! includes Microsoft Store apps such as the Store build of Spotify. An app is
//! launched by its Start-menu id, never by running text the user typed.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

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

/// Combine the two sources: Start-menu apps first, then shortcut-only apps whose
/// name is not already listed.
pub fn merge_apps(listed: &[AppEntry], shortcuts: &[AppEntry]) -> Vec<AppEntry> {
    let mut all = listed.to_vec();
    for s in shortcuts {
        let n = normalize(&s.name);
        if !all.iter().any(|e| normalize(&e.name) == n) {
            all.push(s.clone());
        }
    }
    all
}

type FastLoader = Box<dyn Fn() -> Vec<AppEntry> + Send + Sync>;
type FullLoader = Box<dyn Fn() -> Result<Vec<AppEntry>, String> + Send + Sync>;

#[derive(Default)]
struct IndexState {
    shortcuts: Vec<AppEntry>,
    listed: Vec<AppEntry>,
    merged: Vec<AppEntry>,
    refreshing: bool,
    /// A refresh has been started at least once.
    started: bool,
    last_refresh: Option<Instant>,
    last_error: Option<String>,
}

/// In-memory list of installed apps. It is built on a background thread so a
/// request never pays for the slow app listing (PowerShell `Get-StartApps`
/// takes 2-8 s). A request waits only when the index has nothing to answer
/// from yet, or when the app is missing and the last refresh is old; in both
/// cases it waits for the refresh already running instead of starting another.
pub struct AppIndex {
    state: Mutex<IndexState>,
    done: Condvar,
    fast: FastLoader,
    full: FullLoader,
    /// A cached answer older than this is served, then refreshed in the background.
    max_age: Duration,
    /// A miss triggers a refresh only if the last one is older than this.
    miss_cooldown: Duration,
}

impl AppIndex {
    /// `fast` reads Start Menu shortcuts (milliseconds); `full` asks Windows for
    /// the full app list (seconds) and is what makes the index complete.
    pub fn new(
        fast: impl Fn() -> Vec<AppEntry> + Send + Sync + 'static,
        full: impl Fn() -> Result<Vec<AppEntry>, String> + Send + Sync + 'static,
    ) -> Self {
        AppIndex {
            state: Mutex::default(),
            done: Condvar::new(),
            fast: Box::new(fast),
            full: Box::new(full),
            max_age: Duration::from_secs(10 * 60),
            miss_cooldown: Duration::from_secs(60),
        }
    }

    pub fn with_timing(mut self, max_age: Duration, miss_cooldown: Duration) -> Self {
        self.max_age = max_age;
        self.miss_cooldown = miss_cooldown;
        self
    }

    fn lock(&self) -> MutexGuard<'_, IndexState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wait<'a>(&self, st: MutexGuard<'a, IndexState>) -> MutexGuard<'a, IndexState> {
        self.done.wait(st).unwrap_or_else(|p| p.into_inner())
    }

    /// Start building the index on a background thread. Does nothing if a
    /// build is already running. Returns at once.
    pub fn warm_up(self: &Arc<Self>) {
        let mut st = self.lock();
        self.start_refresh(&mut st);
    }

    fn start_refresh(self: &Arc<Self>, st: &mut IndexState) {
        if st.refreshing {
            return;
        }
        st.refreshing = true;
        st.started = true;
        let me = Arc::clone(self);
        std::thread::spawn(move || me.refresh_now());
    }

    fn refresh_now(&self) {
        // Stage 1: shortcuts. Quick, so a lookup can often be answered before
        // the slow listing finishes.
        let shortcuts = (self.fast)();
        {
            let mut st = self.lock();
            st.merged = merge_apps(&st.listed, &shortcuts);
            st.shortcuts = shortcuts;
        }
        self.done.notify_all();
        // Stage 2: the full list.
        let result = (self.full)();
        let mut st = self.lock();
        match result {
            Ok(listed) => {
                st.merged = merge_apps(&listed, &st.shortcuts);
                st.listed = listed;
                st.last_error = None;
            }
            Err(e) => st.last_error = Some(e),
        }
        st.refreshing = false;
        st.last_refresh = Some(Instant::now());
        drop(st);
        self.done.notify_all();
    }

    /// Look an app up by name.
    pub fn lookup(self: &Arc<Self>, wanted: &str) -> Result<Lookup, String> {
        let mut st = self.lock();
        if !st.started {
            self.start_refresh(&mut st);
        }
        // Nothing to answer from yet: wait for the build that is running.
        while st.merged.is_empty() && st.refreshing {
            st = self.wait(st);
        }
        if st.merged.is_empty() {
            if let Some(e) = &st.last_error {
                return Err(e.clone());
            }
        }
        let first = find_app(&st.merged, wanted);
        let age = st.last_refresh.map(|t| t.elapsed());
        if first != Lookup::NotFound {
            if !st.refreshing && age.is_none_or(|a| a >= self.max_age) {
                self.start_refresh(&mut st);
            }
            return Ok(first);
        }
        // A miss: the app may be new, or the full list may not be in yet.
        let worth_refreshing = st.refreshing || age.is_none_or(|a| a >= self.miss_cooldown);
        if !worth_refreshing {
            return Ok(Lookup::NotFound);
        }
        self.start_refresh(&mut st);
        while st.refreshing {
            st = self.wait(st);
        }
        if st.merged.is_empty() {
            if let Some(e) = &st.last_error {
                return Err(e.clone());
            }
        }
        Ok(find_app(&st.merged, wanted))
    }
}

#[cfg(windows)]
mod os {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::OnceLock;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn index() -> &'static Arc<AppIndex> {
        static INDEX: OnceLock<Arc<AppIndex>> = OnceLock::new();
        INDEX.get_or_init(|| Arc::new(AppIndex::new(scan_shortcuts, list_start_apps)))
    }

    /// Start building the app index in the background. Call once at startup.
    pub fn warm_up() {
        index().warm_up();
    }

    fn collect_lnk(dir: &Path, depth: u32, out: &mut Vec<AppEntry>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 4 {
                    collect_lnk(&p, depth + 1, out);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(AppEntry {
                        name: stem.into(),
                        app_id: p.to_string_lossy().into_owned(),
                    });
                }
            }
        }
    }

    /// Start Menu shortcuts (all users and this user). The "id" is the .lnk path.
    fn scan_shortcuts() -> Vec<AppEntry> {
        let tail = Path::new("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        let mut out = Vec::new();
        for var in ["ProgramData", "APPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                collect_lnk(&PathBuf::from(base).join(&tail), 0, &mut out);
            }
        }
        out
    }

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

    /// Look an app up by name in the in-memory index (see [`AppIndex`]).
    pub fn lookup_app(wanted: &str) -> Result<Lookup, String> {
        index().lookup(wanted)
    }

    /// Start an app. Returns as soon as Windows has been asked to start it.
    pub fn launch(app: &AppEntry) -> Result<(), String> {
        let target = if app.app_id.to_lowercase().ends_with(".lnk") {
            app.app_id.clone() // a Start Menu shortcut from the scan
        } else {
            format!("shell:AppsFolder\\{}", app.app_id)
        };
        Command::new("explorer.exe")
            .arg(target)
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
pub use os::{is_app_running, launch, lookup_app, warm_up};

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

    fn app(name: &str) -> AppEntry {
        AppEntry {
            name: name.into(),
            app_id: format!("{name}!id"),
        }
    }

    /// An index whose slow listing sleeps and counts how often it ran.
    fn slow_index(full_ms: u64) -> (Arc<AppIndex>, Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let idx = AppIndex::new(
            || vec![app("Quick")],
            move || {
                r.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(full_ms));
                Ok(vec![app("Notepad"), app("Spotify")])
            },
        );
        (Arc::new(idx), runs)
    }

    #[test]
    fn merge_keeps_listed_apps_and_adds_new_shortcuts_only() {
        let m = merge_apps(&[app("Notepad")], &[app("notepad"), app("Quick")]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].name, "Quick");
    }

    #[test]
    fn a_hit_in_the_fast_stage_does_not_wait_for_the_slow_listing() {
        let (idx, _) = slow_index(1500);
        idx.warm_up();
        let t = Instant::now();
        assert!(matches!(idx.lookup("quick"), Ok(Lookup::Found(_))));
        assert!(
            t.elapsed() < Duration::from_millis(1000),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn a_lookup_during_warm_up_waits_for_it_and_does_not_start_a_second() {
        use std::sync::atomic::Ordering;
        let (idx, runs) = slow_index(300);
        idx.warm_up();
        assert!(matches!(idx.lookup("spotify"), Ok(Lookup::Found(_))));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_warm_index_answers_without_loading_again() {
        use std::sync::atomic::Ordering;
        let (idx, runs) = slow_index(50);
        idx.warm_up();
        assert!(matches!(idx.lookup("notepad"), Ok(Lookup::Found(_))));
        let t = Instant::now();
        for _ in 0..50 {
            assert!(matches!(idx.lookup("notepad"), Ok(Lookup::Found(_))));
        }
        assert!(t.elapsed() < Duration::from_millis(100));
        // A miss just after a refresh is answered at once, not by listing again.
        assert_eq!(idx.lookup("telegram"), Ok(Lookup::NotFound));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_old_index_answers_first_and_refreshes_in_the_background() {
        use std::sync::atomic::Ordering;
        let (idx, runs) = slow_index(300);
        let idx = Arc::new(
            Arc::try_unwrap(idx)
                .ok()
                .unwrap()
                .with_timing(Duration::from_millis(0), Duration::from_secs(60)),
        );
        assert!(matches!(idx.lookup("notepad"), Ok(Lookup::Found(_))));
        let t = Instant::now();
        assert!(matches!(idx.lookup("notepad"), Ok(Lookup::Found(_))));
        assert!(t.elapsed() < Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(600));
        assert!(runs.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn a_miss_after_the_cooldown_refreshes_and_finds_new_apps() {
        let installed = Arc::new(Mutex::new(vec![app("Notepad")]));
        let i = installed.clone();
        let idx = Arc::new(
            AppIndex::new(Vec::new, move || Ok(i.lock().unwrap().clone()))
                .with_timing(Duration::from_secs(600), Duration::from_millis(0)),
        );
        assert_eq!(idx.lookup("telegram"), Ok(Lookup::NotFound));
        installed.lock().unwrap().push(app("Telegram"));
        assert!(matches!(idx.lookup("telegram"), Ok(Lookup::Found(_))));
    }

    #[test]
    fn a_failed_listing_with_nothing_cached_is_an_error() {
        let idx = Arc::new(AppIndex::new(Vec::new, || Err("no powershell".into())));
        assert_eq!(idx.lookup("x"), Err("no powershell".to_string()));
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
