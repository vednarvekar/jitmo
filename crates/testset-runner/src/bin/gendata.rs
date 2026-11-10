//! Training-data generator for the tool-picker model (docs/03-delivery/15, step 5).
//!
//! Writes chat-format JSONL (`{"messages":[system, user, assistant]}`) where the system message
//! lists the three candidate tools (as the engine would) and the answer is the number of the
//! right one, or 0 for "none". Requests come from hand-written templates and filler lists,
//! then are roughed up the way real typing is (typos, Hinglish words, casing, "please").
//!
//! It never reads `testset/heldout.txt`, and its templates were written without looking at it:
//! the held-out set must stay a fair test.
//!
//! ```text
//! jitmo-gendata --out training/router_train.jsonl [--per-skill 60] [--seed 7] [--skills skills]
//!               [--worker target/release/jitmo-model-worker.exe]
//! ```

use jitmo_core::router::Router;
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// A small deterministic random generator (no extra dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

fn fillers(kind: &str) -> &'static [&'static str] {
    match kind {
        "app" => &[
            "spotify",
            "chrome",
            "notepad",
            "word",
            "excel",
            "vlc",
            "paint",
            "discord",
            "zoom",
            "telegram",
            "calculator",
            "task manager",
            "edge",
            "settings",
            "powerpoint",
        ],
        "person" => &[
            "rahul", "priya", "neha", "vikram", "ved", "anita", "sam", "kabir", "meera", "john",
        ],
        "topic" => &[
            "the budget review",
            "the design sync",
            "the client call",
            "the sprint planning",
            "the project kickoff",
            "the team standup",
            "the quarterly review",
            "the demo",
        ],
        "place" => &[
            "pune",
            "mumbai",
            "delhi",
            "bangalore",
            "chennai",
            "kolkata",
            "hyderabad",
            "goa",
            "london",
            "dubai",
        ],
        "time" => &[
            "5pm",
            "9am",
            "noon",
            "7:30pm",
            "tomorrow morning",
            "friday at 4",
            "monday 10am",
            "tonight at 8",
            "tomorrow at 3pm",
        ],
        "dur" => &[
            "5 minutes",
            "10 minutes",
            "15 minutes",
            "half an hour",
            "an hour",
            "2 minutes",
            "40 minutes",
            "one hour",
            "90 seconds",
        ],
        "file" => &[
            "the invoice",
            "my resume",
            "the budget sheet",
            "notes.txt",
            "the contract",
            "last week's report",
            "my cv",
            "the slides",
            "the tax return",
            "assignment.docx",
        ],
        "folder" => &[
            "documents",
            "downloads",
            "desktop",
            "the finance folder",
            "work",
            "study",
            "projects",
        ],
        "artist" => &[
            "arijit singh",
            "coldplay",
            "taylor swift",
            "a r rahman",
            "the weeknd",
            "kishore kumar",
            "some lofi",
            "jazz",
            "relaxing piano",
            "old bollywood songs",
        ],
        "channel" => &[
            "mkbhd",
            "veritasium",
            "ali abdaal",
            "carryminati",
            "3blue1brown",
            "kurzgesagt",
        ],
        "site" => &[
            "github",
            "youtube",
            "gmail",
            "linkedin",
            "wikipedia",
            "netflix",
            "amazon",
            "reddit",
            "twitter",
            "stackoverflow",
        ],
        "query" => &[
            "best laptops 2026",
            "how to tie a tie",
            "pasta recipes",
            "cheap hotels in goa",
            "python list comprehension",
            "train timings pune delhi",
            "latest iphone price",
        ],
        "sum" => &[
            "17 times 23",
            "450 plus 275",
            "15% of 2400",
            "sqrt of 225",
            "1000 divided by 8",
            "12 percent of 560",
            "7 squared",
            "250 minus 87",
        ],
        "fact" => &[
            "the capital of france",
            "the tallest building in the world",
            "who wrote hamlet",
            "the speed of light",
            "the president of the usa",
            "the boiling point of water",
            "who discovered penicillin",
            "the largest planet",
            "the author of the ramayana",
        ],
        "city2" => &[
            "tokyo",
            "new york",
            "sydney",
            "paris",
            "singapore",
            "berlin",
            "san francisco",
        ],
        "msg" => &[
            "i'm running late",
            "the file is ready",
            "call me back",
            "see you at 5",
            "thanks for the update",
            "meeting moved to friday",
            "i'll send it tonight",
        ],
        "level" => &["20", "40", "60", "75", "100", "30"],
        "radio" => &["bluetooth", "wifi", "airplane mode", "the wi-fi", "bt"],
        _ => &["x"],
    }
}

/// Templates per skill. `{kind}` is replaced by a random filler of that kind.
fn templates(skill: &str) -> Vec<&'static str> {
    match skill {
        "apps.open" => vec![
            "launch {app}",
            "i want to use {app}",
            "{app} kholo",
            "run {app}",
            "start up {app}",
            "could you open {app}",
            "need {app} now",
            "open up {app} for me",
            "{app} please",
            "boot {app}",
            "get {app} running",
            "jump into {app}",
        ],
        "browser.search" => vec![
            "search the web for {query}",
            "google {query}",
            "look up {query} online",
            "find me results for {query}",
            "{query} search karo",
            "do a web search on {query}",
            "i want to search {query}",
            "bing {query}",
        ],
        "browser.site" => vec![
            "open {site} in my browser",
            "go to {site}.com",
            "{site} website kholo",
            "visit {site}",
            "load {site}",
            "browse to {site}",
            "show me {site} on the web",
        ],
        "browser.youtube_latest" => vec![
            "latest video by {channel}",
            "{channel}'s newest upload",
            "what's new on {channel}'s channel",
            "show me the freshest {channel} video",
            "{channel} ka naya video",
            "most recent upload from {channel}",
        ],
        "calc.math" => vec![
            "calculate {sum}",
            "what is {sum}",
            "{sum}",
            "work out {sum}",
            "quick math: {sum}",
            "how much is {sum}",
            "compute {sum}",
            "can you do {sum}",
        ],
        "calc.moon" => vec![
            "moon phase today",
            "how much of the moon is lit tonight",
            "is there a new moon",
            "when is the next full moon",
            "chand ki phase kya hai",
            "tell me about tonight's moon",
        ],
        "calc.time_in" => vec![
            "time in {city2}",
            "what's the clock showing in {city2}",
            "how late is it in {city2}",
            "{city2} time now",
            "tell me the time in {city2}",
            "what time is it over in {city2}",
        ],
        "calendar.accept" => vec![
            "accept {topic}",
            "i'll be there for {topic}",
            "reply yes to {topic}",
            "confirm my attendance at {topic}",
            "rsvp yes to {topic}",
            "mark me as going to {topic}",
        ],
        "calendar.add" => vec![
            "add {topic} to my calendar at {time}",
            "schedule {topic} {time}",
            "book a slot for {topic} on {time}",
            "calendar: {topic} {time}",
            "create an event {topic} {time}",
            "{time} pe {topic} calendar mein daalo",
            "put {topic} in my diary for {time}",
        ],
        "calendar.agenda" => vec![
            "what's on my calendar {time}",
            "show my agenda for {time}",
            "any meetings {time}",
            "what do i have {time}",
            "calendar for {time}",
            "am i free or busy {time}",
            "list my events today",
            "how packed is my day",
        ],
        "calendar.block" => vec![
            "block {dur} on my calendar {time}",
            "reserve {dur} for focus {time}",
            "keep {dur} free {time}",
            "mark me busy for {dur} {time}",
            "hold {dur} {time} for studying",
        ],
        "calendar.context" => vec![
            "what is {topic} about",
            "who's attending {topic}",
            "brief me on {topic}",
            "details of {topic}",
            "prep me for {topic}",
            "what do i need for {topic}",
        ],
        "calendar.decline" => vec![
            "decline {topic}",
            "i can't attend {topic}",
            "tell them no for {topic}",
            "pass on {topic}",
            "cancel my attendance for {topic}",
            "refuse the invite to {topic}",
        ],
        "calendar.free" => vec![
            "when is {person} free",
            "find a time that works for me and {person}",
            "check {person}'s availability",
            "when can {person} and i meet",
            "a free slot with {person} {time}",
            "is {person} available {time}",
        ],
        "calendar.invite" => vec![
            "invite {person} to {topic}",
            "add {person} as an attendee on {topic}",
            "include {person} in {topic}",
            "send {person} an invite for {topic}",
            "{person} ko {topic} mein bulao",
        ],
        "calendar.move" => vec![
            "move {topic} to {time}",
            "reschedule {topic} for {time}",
            "push {topic} to {time}",
            "change the time of {topic} to {time}",
            "shift {topic} to {time}",
        ],
        "chat.post" => vec![
            "post in the general channel that {msg}",
            "tell the team on teams that {msg}",
            "message the project channel: {msg}",
            "write in the dev channel {msg}",
            "send a teams message to the group saying {msg}",
        ],
        "files.email" => vec![
            "email {file} to {person}",
            "send {file} to {person} over mail",
            "attach {file} and mail it to {person}",
            "mail {person} {file}",
            "forward {file} to {person} by email",
        ],
        "files.find" => vec![
            "find {file}",
            "where is {file}",
            "search my pc for {file}",
            "locate {file}",
            "i lost {file}, look for it",
            "{file} kahan hai",
            "look for {file} on my computer",
        ],
        "files.move" => vec![
            "move {file} to {folder}",
            "put {file} in {folder}",
            "relocate {file} into {folder}",
            "file {file} under {folder}",
            "{file} ko {folder} mein daalo",
        ],
        "files.open" => vec![
            "open {file}",
            "show me {file}",
            "launch {file}",
            "i need to see {file}",
            "{file} kholo",
            "bring up {file}",
        ],
        "files.recent" => vec![
            "what did i save recently",
            "newest files in downloads",
            "latest downloads",
            "recent files please",
            "show what i downloaded today",
            "last few files i got",
        ],
        "files.share" => vec![
            "share {folder} with {person}",
            "give {person} access to {folder}",
            "let {person} open {folder}",
            "send {person} a link to {folder}",
            "{folder} {person} ke saath share karo",
        ],
        "mail.archive_newsletters" => vec![
            "archive the newsletters",
            "clean out promotional mail",
            "move newsletters out of my inbox",
            "get rid of newsletter emails",
            "sweep my newsletters away",
            "newsletters archive kar do",
        ],
        "mail.draft" => vec![
            "draft an email to {person} saying {msg}",
            "write a draft for {person}: {msg}",
            "prepare a mail to {person} but don't send: {msg}",
            "save a draft to {person} about {topic}",
        ],
        "mail.find" => vec![
            "find the email from {person}",
            "search my inbox for {topic}",
            "look for mail about {topic}",
            "emails from {person} with attachments",
            "did {person} write to me",
            "search mail for {file}",
        ],
        "mail.inbox" => vec![
            "check my inbox",
            "how many emails are unread",
            "what mail came in",
            "any new messages in email",
            "open my emails",
            "inbox summary please",
            "mere emails dikhao",
        ],
        "mail.send" => vec![
            "email {person} that {msg}",
            "send {person} a mail saying {msg}",
            "write to {person}: {msg}",
            "mail {person} {msg}",
            "shoot {person} an email, {msg}",
            "{person} ko mail karo ki {msg}",
        ],
        "mail.summarize" => vec![
            "summarise the email from {person}",
            "what's the gist of my latest mail",
            "short summary of {person}'s message",
            "tldr the last email",
            "read me my newest email in brief",
        ],
        "mail.unanswered" => vec![
            "which mails do i still owe a reply",
            "emails waiting for my answer",
            "who is waiting on me",
            "unreplied emails",
            "show pending replies",
        ],
        "mail.urgent" => vec![
            "any urgent mail",
            "important emails today",
            "anything critical in my inbox",
            "high priority messages",
            "what needs my attention in email first",
        ],
        "music.control" => vec![
            "pause",
            "stop the music",
            "next song",
            "previous track",
            "resume the song",
            "skip",
            "play pause",
            "song badlo",
            "go to the next one",
        ],
        "music.play" => vec![
            "play {artist}",
            "put on {artist}",
            "i feel like hearing {artist}",
            "start some {artist} on spotify",
            "{artist} lagao",
            "queue up {artist}",
            "music: {artist}",
        ],
        "reminders.remind" => vec![
            "remind me to {msg} at {time}",
            "set a reminder: {msg} {time}",
            "ping me {time} to call {person}",
            "don't forget to remind me about {topic} {time}",
            "{time} pe yaad dilana {msg}",
            "alert me {time} for {topic}",
        ],
        "reminders.timer" => vec![
            "set a timer for {dur}",
            "start a {dur} countdown",
            "timer {dur}",
            "alarm in {dur}",
            "ring me after {dur}",
            "{dur} ka timer",
            "count {dur} for me",
        ],
        "settings.place" => vec![
            "set my city to {place}",
            "i live in {place}",
            "my location is {place}",
            "use {place} as my default city",
            "remember that i'm in {place}",
        ],
        "system.dnd" => vec![
            "turn on do not disturb",
            "mute notifications for {dur}",
            "focus mode on",
            "stop the pop-ups",
            "disable do not disturb",
            "quiet mode please",
        ],
        "system.lock" => vec![
            "lock the computer",
            "lock screen now",
            "secure my pc",
            "win+l",
            "pc lock karo",
        ],
        "system.radio" => vec![
            "turn on {radio}",
            "switch off {radio}",
            "enable {radio}",
            "disable {radio}",
            "{radio} on karo",
            "toggle {radio}",
        ],
        "system.screenshot" => vec![
            "screenshot",
            "snap my screen",
            "capture what's on screen",
            "take a screen grab",
            "save a picture of the display",
        ],
        "system.volume" => vec![
            "volume {level}",
            "set the sound to {level} percent",
            "louder",
            "quieter",
            "mute",
            "unmute the pc",
            "volume up a bit",
            "awaaz {level} karo",
        ],
        "web.fact" => vec![
            "what is {fact}",
            "tell me {fact}",
            "who or what is {fact}",
            "quick question, {fact}",
            "do you know {fact}",
            "{fact}?",
        ],
        "web.weather" => vec![
            "weather in {place}",
            "will it rain in {place} {time}",
            "temperature in {place}",
            "is it hot in {place}",
            "forecast for {place}",
            "{place} ka mausam",
            "do i need a jacket in {place}",
        ],
        "whatsapp.send_file" => vec![
            "whatsapp {file} to {person}",
            "send {file} to {person} on whatsapp",
            "share {file} with {person} via whatsapp",
        ],
        "whatsapp.send_message" => vec![
            "whatsapp {person} {msg}",
            "text {person} on whatsapp: {msg}",
            "message {person} on whatsapp that {msg}",
            "{person} ko whatsapp karo {msg}",
        ],
        "work.morning_brief" => vec![
            "morning briefing",
            "brief me for today",
            "start my day summary",
            "what's my day look like",
            "daily digest please",
        ],
        "work.event_from_email" => vec![
            "make an event from that email",
            "put the email's meeting on my calendar",
            "calendar entry from the latest mail",
            "turn this mail into an event",
        ],
        "work.save_attachments" => vec![
            "save the attachments to onedrive",
            "put the mail's files in my cloud folder",
            "store attachments from that email",
            "download the email attachments to drive",
        ],
        "work.forward_attachment" => vec![
            "share the email attachment in teams",
            "send that attachment to the team chat",
            "post the mail's file in the channel",
        ],
        "work.send_report" => vec![
            "send the weekly report to my manager",
            "report to my boss",
            "email this week's status to my manager",
            "weekly update to the boss",
        ],
        _ => Vec::new(),
    }
}

/// Requests no tool should handle: the model must answer "none".
const NONE_REQUESTS: &[&str] = &[
    "why is the sky blue",
    "write a short story about a dragon",
    "what should i cook tonight",
    "teach me the pythagorean theorem",
    "give me a motivational quote",
    "explain how a car engine works",
    "who would win a fight, a lion or a tiger",
    "recommend a good book",
    "how do i get better at chess",
    "summarise world war one",
    "what is the meaning of dharma",
    "help me plan a birthday party",
    "tell me something interesting",
    "do my homework",
    "write a cover letter for me",
    "how do i lose weight",
    "turn off the pc",
    "restart the laptop",
    "close all my apps",
    "delete all my emails",
    "empty the recycle bin",
    "forward the mail to everyone",
    "call my mother",
    "order food from zomato",
    "hack my neighbour's wifi",
    "reply to all my mail with yes",
    "email everyone in my contacts",
    "what is my password",
    "ok",
    "hmm",
    "send",
    "open",
    "play",
];

fn fill(template: &str, rng: &mut Rng) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else { break };
        out.push_str(rng.pick(fillers(&after[..j])));
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

fn roughen(text: &str, rng: &mut Rng) -> String {
    let mut s = text.to_string();
    if rng.chance(22) {
        // A typo: swap two neighbouring letters or drop one in a longer word.
        let mut words: Vec<String> = s.split(' ').map(str::to_string).collect();
        let i = rng.below(words.len());
        let w: Vec<char> = words[i].chars().collect();
        if w.len() > 4 {
            let p = 1 + rng.below(w.len() - 2);
            let mut w2 = w.clone();
            if rng.chance(50) {
                w2.swap(p, p + 1);
            } else {
                w2.remove(p);
            }
            words[i] = w2.into_iter().collect();
        }
        s = words.join(" ");
    }
    if rng.chance(12) {
        s = format!(
            "{} {}",
            rng.pick(&["bhai", "yaar", "hey", "ok", "so", "um"]),
            s
        );
    }
    if rng.chance(12) {
        s.push_str(rng.pick(&[" please", " pls", " now", " asap", " thanks"]));
    }
    match rng.below(10) {
        0 => s = s.to_uppercase(),
        1 => {
            s.push(rng.pick(&['.', '?', '!']).to_owned());
        }
        _ => {}
    }
    s
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_string)
        .collect()
}

struct Retriever {
    docs: Vec<(String, HashSet<String>)>,
    idf: HashMap<String, f64>,
}

impl Retriever {
    fn new(skills: &[Skill]) -> Retriever {
        let docs: Vec<(String, HashSet<String>)> = skills
            .iter()
            .map(|s| {
                let mut text = s.title.clone().unwrap_or_default();
                for e in &s.examples {
                    text.push(' ');
                    text.push_str(e);
                }
                text.push(' ');
                text.push_str(&s.id.replace(['.', '_'], " "));
                (s.id.clone(), words(&text).into_iter().collect())
            })
            .collect();
        let n = docs.len() as f64;
        let mut df: HashMap<String, f64> = HashMap::new();
        for (_, w) in &docs {
            for t in w {
                *df.entry(t.clone()).or_default() += 1.0;
            }
        }
        let idf = df
            .into_iter()
            .map(|(t, d)| (t, ((n + 1.0) / (d + 0.5)).ln()))
            .collect();
        Retriever { docs, idf }
    }

    fn top(&self, request: &str, k: usize) -> Vec<String> {
        let q: HashSet<String> = words(request).into_iter().collect();
        let mut scored: Vec<(f64, &String)> = self
            .docs
            .iter()
            .map(|(id, w)| {
                let s: f64 = q
                    .iter()
                    .filter(|t| w.contains(*t))
                    .map(|t| self.idf.get(t).copied().unwrap_or(0.0))
                    .sum();
                (s, id)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(k)
            .map(|(_, id)| id.clone())
            .collect()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_path = flag(&args, "--out").unwrap_or_else(|| "training/router_train.jsonl".into());
    let per: usize = flag(&args, "--per-skill")
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let seed: u64 = flag(&args, "--seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let skills_dir = flag(&args, "--skills").unwrap_or_else(|| "skills".into());
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);

    let mut skills: Vec<Skill> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&skills_dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("yaml") {
                if let Ok(y) = std::fs::read_to_string(e.path()) {
                    if let Ok(s) = Skill::parse(&y) {
                        if !s.id.starts_with("account.") && s.id != "stub.send" {
                            skills.push(s);
                        }
                    }
                }
            }
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<String> = skills.iter().map(|s| s.id.clone()).collect();
    let by_id: HashMap<&str, &Skill> = skills.iter().map(|s| (s.id.as_str(), s)).collect();
    let retriever = Retriever::new(&skills);
    let router: Option<Router> = flag(&args, "--worker").map(|w| {
        let launcher = WorkerLauncher {
            program: w.into(),
            args: vec![
                "--models".into(),
                flag(&args, "--models").unwrap_or_else(|| "models/router".into()),
                "--idle-secs".into(),
                "0".into(),
                "--extractor".into(),
                flag(&args, "--ner").unwrap_or_else(|| "models/ner".into()),
            ],
        };
        Router::new(skills.clone())
            .with_worker(WorkerClient::new(launcher, Duration::from_secs(20)))
    });
    let candidates = |text: &str| -> Vec<String> {
        match &router {
            Some(r) => r
                .route(text)
                .tasks
                .iter()
                .filter(|t| ids.contains(&t.id))
                .map(|t| t.id.clone())
                .take(3)
                .collect(),
            None => retriever.top(text, 3),
        }
    };

    let system = |cand: &[String]| -> String {
        let catalog: String = cand
            .iter()
            .enumerate()
            .filter_map(|(n, id)| by_id.get(id.as_str()).map(|s| (n, *s)))
            .map(|(n, s)| format!("{}: {}", n + 1, s.title.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "You route requests for a Windows assistant. Pick the single tool that does what the \
             user asks, or \"none\" when no tool fits, when it is general chat or a question the \
             tools cannot answer, or when it is unsafe or unsupported. Answer with the tool \
             number, or 0 for none.\n\nTools:\n{catalog}"
        )
    };

    let mut lines: Vec<String> = Vec::new();
    let mut emit = |text: &str, gold: Option<&str>, rng: &mut Rng| {
        let mut cand = candidates(text);
        let answer = match gold {
            Some(g) => {
                if !cand.contains(&g.to_string()) {
                    // Retrieval missed: usually put the right tool in (so the model learns to
                    // read the tools), sometimes leave it out and expect 0.
                    if rng.chance(85) {
                        cand.truncate(2);
                        let at = rng.below(cand.len() + 1);
                        cand.insert(at, g.to_string());
                    }
                }
                cand.iter()
                    .position(|c| c == g)
                    .map(|p| (p + 1).to_string())
                    .unwrap_or_else(|| "0".into())
            }
            None => "0".into(),
        };
        let line = json!({ "messages": [
            { "role": "system", "content": system(&cand) },
            { "role": "user", "content": text },
            { "role": "assistant", "content": json!({ "tool": answer }).to_string() }
        ]});
        lines.push(line.to_string());
    };

    for s in &skills {
        let t = templates(&s.id);
        if t.is_empty() {
            eprintln!("note: no templates for {}", s.id);
            continue;
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut tries = 0;
        while seen.len() < per && tries < per * 20 {
            tries += 1;
            let text = roughen(&fill(rng.pick(&t), &mut rng), &mut rng);
            if seen.insert(text.clone()) {
                emit(&text, Some(&s.id), &mut rng);
            }
        }
    }
    for _ in 0..(per * 3) {
        let base = *rng.pick(NONE_REQUESTS);
        let text = roughen(base, &mut rng);
        emit(&text, None, &mut rng);
    }
    // A stable shuffle.
    for i in (1..lines.len()).rev() {
        let j = rng.below(i + 1);
        lines.swap(i, j);
    }
    if let Some(dir) = std::path::Path::new(&out_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&out_path, lines.join("\n") + "\n").expect("could not write the training file");
    println!("wrote {} examples to {out_path}", lines.len());
}
