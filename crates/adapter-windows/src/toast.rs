//! Windows notifications (toasts) for reminders and timers.
//!
//! Uses the Windows PowerShell notification identity, so no installer
//! registration is needed during development. Title and text travel in
//! environment variables and are XML-escaped inside the script, so a reminder
//! like `call "mom" & dad` can never break the XML or inject script.

/// The PowerShell script that shows the toast. Reads `JITMO_TOAST_TITLE` and
/// `JITMO_TOAST_BODY`.
pub const TOAST_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$title = [System.Security.SecurityElement]::Escape($env:JITMO_TOAST_TITLE)
$body = [System.Security.SecurityElement]::Escape($env:JITMO_TOAST_BODY)
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml("<toast scenario='reminder'><visual><binding template='ToastGeneric'><text>$title</text><text>$body</text></binding></visual><audio src='ms-winsoundevent:Notification.Reminder'/></toast>")
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
"#;

#[cfg(windows)]
mod os {
    use super::TOAST_SCRIPT;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    /// Show a notification. Returns an error message if Windows refused it
    /// (for example Focus Assist or notifications turned off for the app).
    pub fn show_toast(title: &str, body: &str) -> Result<(), String> {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", TOAST_SCRIPT])
            .env("JITMO_TOAST_TITLE", title)
            .env("JITMO_TOAST_BODY", body)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

#[cfg(windows)]
pub use os::show_toast;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_escapes_text_instead_of_splicing_it_in() {
        // Text must come from the environment and be escaped, never pasted into the script.
        assert!(TOAST_SCRIPT.contains("$env:JITMO_TOAST_TITLE"));
        assert!(TOAST_SCRIPT.contains("SecurityElement]::Escape"));
        assert!(!TOAST_SCRIPT.contains("{0}"));
    }

    /// Shows one real notification. Run with `--ignored`; it needs a desktop session.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_toast_with_awkward_characters() {
        show_toast("jitmo test", "Call \"mom\" & dad <now>").unwrap();
    }
}
