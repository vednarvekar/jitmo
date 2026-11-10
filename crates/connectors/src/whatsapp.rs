//! WhatsApp on this PC, the safe way: Jitmo opens the chat with your message already typed
//! (WhatsApp's own `whatsapp://send` link) and **you press Send**. Nothing is sent by Jitmo,
//! nothing is linked to your account, and nothing in it can get a number banned.
//!
//! - A text: the chat opens with the message in the box.
//! - A file: the chat opens and the file is copied, so Ctrl+V then Send attaches it.
//! - A person without a saved number (with country code, like +91…): WhatsApp opens on its
//!   own chat list with the text ready, and Jitmo says to pick the chat.
//!
//! Without the desktop app the same link opens WhatsApp Web in the browser.

use crate::ms365::{arg, out};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};

type PhoneOf = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type Run = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct WhatsAppDesktop {
    phone_of: PhoneOf,
    open: Run,
    copy_file: Run,
    desktop_installed: Box<dyn Fn() -> bool + Send + Sync>,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Digits of an international number ("+91 98765 43210" -> "919876543210"); `None` without a
/// country code, because WhatsApp cannot find a local number.
pub fn international_digits(phone: &str) -> Option<String> {
    let p = phone.trim();
    if !p.starts_with('+') {
        return None;
    }
    let digits: String = p.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 8).then_some(digits)
}

/// The link that opens a chat (and types the text). `web` selects WhatsApp Web.
pub fn chat_link(number: Option<&str>, text: Option<&str>, web: bool) -> String {
    let mut params = Vec::new();
    if let Some(n) = number {
        params.push(format!("phone={n}"));
    }
    if let Some(t) = text {
        params.push(format!("text={}", encode(t)));
    }
    let q = params.join("&");
    if web {
        format!("https://web.whatsapp.com/send?{q}")
    } else {
        format!("whatsapp://send?{q}")
    }
}

impl WhatsAppDesktop {
    pub fn new(
        phone_of: PhoneOf,
        open: Run,
        copy_file: Run,
        desktop_installed: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> WhatsAppDesktop {
        WhatsAppDesktop {
            phone_of,
            open,
            copy_file,
            desktop_installed,
        }
    }

    /// Opens the chat; `true` when a number was known (so the right chat is open).
    fn open_chat(&self, contact: &str, text: Option<&str>) -> Result<bool, ConnError> {
        let number = (self.phone_of)(contact).and_then(|p| international_digits(&p));
        let known = number.is_some();
        let link = chat_link(number.as_deref(), text, !(self.desktop_installed)());
        (self.open)(&link)
            .map_err(|_| ConnError::UiChanged("WhatsApp".into()))
            .map(|_| known)
    }
}

impl Connector for WhatsAppDesktop {
    fn id(&self) -> &str {
        "whatsapp_desktop"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("open_chat", &["contact"]),
            a("prefill", &["contact", "text"]),
            a("attach_and_send", &["file"]),
            a("await_send", &[]),
            a("last_message_is_file", &[]),
        ]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        match action {
            "open_chat" => {
                let contact = arg(args, "contact")?;
                let known = self.open_chat(contact, None)?;
                let message = if known {
                    format!("Opened {contact}'s chat.")
                } else {
                    format!(
                        "Opened WhatsApp. I don't have a number for {contact}: pick their chat."
                    )
                };
                Ok(out(&[("message", message)]))
            }
            "prefill" => {
                let contact = arg(args, "contact")?;
                let text = arg(args, "text")?;
                let known = self.open_chat(contact, Some(text))?;
                let message = if known {
                    format!("Opened {contact}'s chat with your message.")
                } else {
                    format!(
                        "Opened WhatsApp with your message. I don't have a number for {contact}: pick their chat."
                    )
                };
                Ok(out(&[("message", message)]))
            }
            // Attaching needs a click inside WhatsApp, which Jitmo never does for you: the
            // file is copied, and the hand-over message says to paste it.
            "attach_and_send" => {
                let file = arg(args, "file")?;
                (self.copy_file)(file).map_err(|_| ConnError::UiChanged("WhatsApp".into()))?;
                Err(ConnError::UiChanged("WhatsApp".into()))
            }
            // The user presses Send: Jitmo hands over.
            "await_send" => Err(ConnError::UiChanged("WhatsApp".into())),
            "last_message_is_file" => Ok(out(&[("message", String::new())])),
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, _args: &Args) -> Result<bool, ConnError> {
        match action {
            // Opening the chat is the whole job; there is nothing more Jitmo can confirm.
            "open_chat" | "prefill" => Ok(true),
            "attach_and_send" | "await_send" | "last_message_is_file" => {
                Err(ConnError::UiChanged("WhatsApp".into()))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn args(p: &[(&str, &str)]) -> Args {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn connector(installed: bool) -> (WhatsAppDesktop, Arc<Mutex<Vec<String>>>) {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let o = opened.clone();
        let c = WhatsAppDesktop::new(
            Box::new(|n| (n == "Ved").then(|| "+91 98765 43210".to_string())),
            Box::new(move |u| {
                o.lock().unwrap().push(u.to_string());
                Ok(())
            }),
            Box::new(|_| Ok(())),
            Box::new(move || installed),
        );
        (c, opened)
    }

    #[test]
    fn numbers_need_a_country_code() {
        assert_eq!(
            international_digits("+91 98765 43210").as_deref(),
            Some("919876543210")
        );
        assert_eq!(international_digits("98765 43210"), None);
    }

    #[test]
    fn a_known_contact_gets_their_chat_with_the_text_typed() {
        let (c, opened) = connector(true);
        let o = c
            .execute(
                "prefill",
                &args(&[("contact", "Ved"), ("text", "I'll be late & sorry.")]),
            )
            .unwrap();
        assert!(o.0.get("message").unwrap().contains("Ved's chat"));
        assert_eq!(
            opened.lock().unwrap()[0],
            "whatsapp://send?phone=919876543210&text=I%27ll%20be%20late%20%26%20sorry."
        );
    }

    #[test]
    fn an_unknown_number_opens_whatsapp_and_says_to_pick_the_chat() {
        let (c, opened) = connector(true);
        let o = c
            .execute("prefill", &args(&[("contact", "Mom"), ("text", "Hi.")]))
            .unwrap();
        assert!(o.0.get("message").unwrap().contains("pick their chat"));
        assert_eq!(opened.lock().unwrap()[0], "whatsapp://send?text=Hi.");
    }

    #[test]
    fn without_the_desktop_app_whatsapp_web_opens() {
        let (c, opened) = connector(false);
        c.execute("open_chat", &args(&[("contact", "Ved")]))
            .unwrap();
        assert!(opened.lock().unwrap()[0].starts_with("https://web.whatsapp.com/send?phone="));
    }

    #[test]
    fn a_file_is_copied_and_the_user_takes_over() {
        let (c, _) = connector(true);
        assert!(matches!(
            c.execute("attach_and_send", &args(&[("file", "C:\\a.pdf")])),
            Err(ConnError::UiChanged(_))
        ));
    }
}
