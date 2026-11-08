"""Phase 2 additions to the router training templates (merged by gen_data.main).

Goals: short, casual and one-word forms for every Phase 2 task (A1-A6, Q1-Q5, F1-F4,
R1-R2, S1-S5), extra phrasings for the near-neighbour confusions found in SP1
(S1 vs A3, E5 vs F5, E7 vs M1/E5, A2 vs A3) and OUT requests that look in scope.

Rules used when writing these (so the fixed held-out sets stay fair): sentences are
written from the task definitions in docs/01-v1-spec/2 - Scope.md, not copied from
spikes/sp1-router/data/*.txt, and gen_data.py's dedupe in train_router.py removes any
exact match with those files anyway.

Label decisions made here: system volume and brightness (including "louder", "turn it
up", "mute") are S1; A3 is pause / resume / skip / previous / stop playback; naming a
song or artist to play is A2; a file sent by mail (email X to Y) is F5; a plain email is E7;
a reply is E5; a chat message needs the app word (whatsapp / teams) for M1 / M4.
"""

SLOTS = {
    # more names for existing slots (added to the lists in gen_data.SLOTS)
    "app": ["Photoshop", "Discord", "Edge", "Steam", "Figma", "OneNote", "VLC", "Brave", "Zoom", "Task Manager", "File Explorer", "Terminal", "Photos"],
    "song": ["Believer", "Perfect", "Despacito", "Shape of My Heart", "Tum Se Hi", "Senorita", "Bohemian Rhapsody", "Heat Waves", "Levitating", "Channa Mereya", "Kal Ho Naa Ho", "Starboy", "some old Hindi songs", "peaceful piano"],
    "creator": ["Marques", "Linus", "Ali Abdaal", "CarryMinati", "Dhruv Rathee", "Technical Guruji", "Mr Beast"],
    "site": ["YouTube", "Gmail", "BBC", "Swiggy", "Zomato", "Stack Overflow", "Twitter", "Quora"],
    "pct": ["10", "20", "25", "30", "40", "50", "60", "70", "75", "80", "90", "100"],
    "company": ["Apple", "Nvidia", "Infosys", "Reliance", "Microsoft", "Tata Motors", "Amazon", "Google"],
    "appliance": ["fan", "AC", "TV", "heater", "geyser", "lamp", "porch light"],
    "room": ["bedroom", "living room", "kitchen", "hall", "study"],
    "artist": ["Arijit Singh", "Taylor Swift", "Coldplay", "AP Dhillon", "Queen", "Diljit"],
    "unit": ["miles to km", "kg to pounds", "cups to ml", "celsius to fahrenheit", "feet to metres"],
    "ext": ["pdf", "excel sheet", "word doc", "slide deck", "csv", "screenshot"],
}

ADD = {
    # ---- apps and media ----
    "A1": [
        "open {app} please", "run {app}", "bring up {app}", "start up {app}", "exit {app}",
        "close {app} now", "close all {app} windows", "i want to use {app}", "{app} open", "open {app} real quick",
        "launch the {app} app", "get {app} open for me", "could you start {app}", "shut {app} down",
        "close this app", "quit everything", "stop {app}", "open my {app}", "ok open {app}", "can u open {app}",
    ],
    "A2": [
        "play {song} by {artist}", "play {artist}", "put on {artist}", "play something by {artist}", "queue {song}",
        "add {song} to the queue", "queue up {artist}", "play {song} next", "play {artist} on spotify", "play me {song}",
        "play a playlist", "play my liked songs", "play some music", "put on some music", "play something chill",
        "play some {song} on spotify", "i feel like listening to {artist}", "can you play {song}", "start playing {song}",
        "play the album by {artist}", "play {song} again", "spotify {artist}", "music by {artist}", "shuffle {artist}",
        "queue up a song by {artist}", "put {song} on", "play {song} please", "i want {song}",
    ],
    "A3": [
        "pause", "pause it", "pause the song", "pause spotify", "play", "resume", "resume the song", "continue playing",
        "keep playing", "unpause the music", "stop the music", "stop playing", "music off", "turn off the music",
        "kill the music", "skip", "skip it", "skip song", "skip this one", "skip this song", "next", "next track",
        "next one", "play the next track", "go to the next song", "previous", "previous song", "go back", "last song",
        "back one song", "play the previous song", "restart this song", "replay this song", "shuffle on", "repeat this song",
        "play pause", "toggle playback", "hold on, pause that", "not this one, skip", "ugh, skip", "next please",
        "can you skip this", "pause the track", "stop spotify", "mute the music player", "back to the last track",
    ],
    "A4": [
        "latest video from {creator}", "{creator}'s newest video", "open {creator}'s latest upload", "what's new from {creator} on youtube",
        "play the newest {creator} video", "newest upload by {creator}", "has {creator} posted a new video", "last video {creator} put out",
        "show the most recent {creator} video", "{creator} new upload", "go to {creator}'s latest youtube video",
    ],
    "A5": [
        "search for {thing}", "search youtube for {thing}", "youtube search {thing}", "google {thing} for me", "search the web for {thing}",
        "look up {thing}", "find videos about {thing}", "search {thing} on google", "web search: {thing}", "can you search {thing}",
        "show me results for {thing}", "look for {thing} online", "search yt for {thing}", "bing {thing}",
    ],
    "A6": [
        "open {site}", "open {site} in the browser", "go to {site}.com", "take me to {site}", "{site}.com", "open the {site} site",
        "browse {site}", "bring up {site} website", "launch {site} in chrome", "load {site}.com", "navigate to {site}",
        "open {site} website", "pull up {site}",
    ],
    # ---- mail and chat (confusable group) ----
    "M1": [
        "whatsapp {person}", "wa {person}: {msg}", "send a whatsapp to {person}: {msg}", "message {person} on whatsapp",
        "whatsapp message to {person}, {msg}", "tell {person} on whatsapp {msg}", "text {person} on whatsapp",
        "send {person} a message on whatsapp saying {msg}", "whatsapp {person} {msg}",
    ],
    "E5": [
        "reply to {person}'s email", "reply to this email", "reply to this", "write a reply", "draft a response",
        "draft a reply saying {msg}", "respond to {person} saying {msg}", "answer this email: {msg}", "reply saying {msg}",
        "help me reply to this email", "write back to this", "reply to {person}", "respond to this mail", "draft something back to {person}",
        "suggest a reply to this email", "write a reply to {person} about {topic}", "prepare a reply: {msg}", "reply back, {msg}",
        "draft my answer to {person}'s email", "reply yes to this email", "reply no thanks to this email",
    ],
    "E7": [
        "email {person}", "send an email to {person}", "send {person} an email", "compose an email to {person}", "new email to {person}",
        "write {person} an email about {topic}", "drop {person} an email", "email {person} about {topic}", "email {person} saying {msg}",
        "send a mail to {person}: {msg}", "write an email to {person}", "mail {person} about {topic}", "send an email about {topic}",
        "start an email to {person}", "email my manager {msg}", "email the team about {topic}",
    ],
    "F5": [
        "email {file} to {person}", "send {file} to {person} by email",
        "attach {file} and email {person}", "send {person} {file} over email", "email the {ext} to {person}",
        "send the {ext} to {person}", "send {file} to {person}", "send {person} {file}", "email this file to {person}",
        "send this file to {person}", "mail the {ext} to {person}",
        "share {file} with {person} on teams", "send {file} to {person} on teams", "email {file}",
    ],
    # ---- queries ----
    "Q1": [
        "sunrise time", "sunset time {day}", "when's sunset", "what time is sunrise", "when is the next full moon",
        "is there a full moon tonight", "moon phase today", "when does it get dark", "sunrise tomorrow in {place}",
        "next full moon", "next new moon", "what's the moon like tonight", "when is sunset in {place}", "what time will the sun come up",
    ],
    "Q2": [
        "weather", "weather today", "weather tomorrow", "how's the weather", "what's the weather in {place}", "is it raining",
        "will it rain {day}", "gonna rain {day}?", "forecast", "forecast for {day}", "temperature", "how cold is it",
        "how warm is it going to be {day}", "do i need a jacket {day}", "is it going to be hot {day}", "will it snow in {place}",
        "rain tmrw?", "weather in {place} {day}", "will it be windy {day}", "check the weather",
    ],
    "Q3": [
        "who is the prime minister of {place}", "who discovered gravity", "how old is the universe", "what is the speed of light",
        "who won the world cup in 2018", "tallest building in the world", "what does dna stand for", "when did world war 2 end",
        "what is the capital of {place}", "largest country by area", "who is the ceo of {company}", "how deep is the ocean",
        "what is inflation", "define serendipity", "how many planets are there", "what is the boiling point of water",
        "longest river in the world", "who founded {company}", "who was the first man on the moon", "how big is {place}",
    ],
    "Q4": [
        "{num} + {num}", "what's {pct} percent of {num}", "{pct}% of {num}", "{num} times {pct}", "convert {pct} {unit}",
        "calculate {num} divided by {pct}", "how much is {num} minus {pct}", "{pct} {unit}", "what is {num} squared",
        "split {num} between 4 people", "{num} * {pct}", "what's {num} / {pct}", "add {num} and {num}", "how many {unit} is {pct}",
        "percentage: {pct} of {num}", "calc {num} plus {pct}", "tip of {pct}% on {num}",
    ],
    "Q5": [
        "time in {place}", "what's the time in {place}", "current time in {place}", "what time is it in {place}", "date today",
        "what's today's date", "what day is it", "what day is {day}'s date", "how many days until {day}", "what's the date on friday",
        "what's the time difference with {place}", "time now in {place}", "what is the date next monday", "days left in the month",
        "which day of the week is the 15th", "{place} time now",
    ],
    # ---- files ----
    "F1": [
        "find {file}", "find my {ext}", "find the {ext} about {topic}", "search for {file}", "where's {file}", "locate {file}",
        "find the {ext} from {day}", "look for the {ext} i made {day}", "find files about {topic}", "search my files for {topic}",
        "can you find {file}", "where did i put {file}", "find that {ext}", "find a {ext} named {topic}",
    ],
    "F2": [
        "recent files", "my recent files", "latest downloads", "what did i download today", "show downloads", "show recent documents",
        "what did i download", "what files did i download {day}", "recently modified files", "last few files i opened",
        "files from today", "show me what i worked on recently", "downloads", "list my downloads",
    ],
    "F3": [
        "open {file}", "open my {ext}", "open the {ext}", "open the {ext} about {topic}", "show me {file}", "open {file} for me",
        "pull up the {ext}", "open that {ext}", "open the file {topic}", "can you open {file}", "bring up my {ext}",
    ],
    "F4": [
        "move {file} to {folder}", "move these to {folder}", "move these files to {folder}", "move the {ext}s to {folder}",
        "copy {file} to {folder}", "copy these files to {folder}", "rename this file", "rename {file} to {topic}",
        "move the selected files to {folder}", "put the selected files in {folder}", "send these files to the {folder} folder",
        "move all pdfs to {folder}", "file {file} under {folder}", "copy the {ext} into {folder}", "rename it to {topic}",
    ],
    # ---- reminders ----
    "R1": [
        "remind me to {topic} at {time}", "remind me to call {person}", "remind me {day} at {time}", "set a reminder to call {person} at {time}",
        "reminder to drink water", "remind me in {dur}", "remind me about {topic}", "remind me later", "remind me to take my medicine at {time}",
        "add a reminder for {topic}", "make a reminder: call {person}", "remind me {day} to call {person}", "don't let me forget {topic}",
    ],
    "R2": [
        "set a timer for {dur}", "timer {dur}", "{dur} timer", "start a timer", "set an alarm for {time}", "alarm for {time}",
        "wake me up at {time}", "set a {dur} timer", "countdown {dur}", "alarm at {time} {day}", "timer for {dur} please",
        "start a {dur} countdown", "ring me in {dur}", "set an alarm", "set a timer",
    ],
    # ---- system (S1 vs A3 is the main confusion) ----
    "S1": [
        "louder", "quieter", "softer", "a bit louder", "a little quieter", "much louder", "turn it up", "turn it down", "turn up",
        "turn down", "volume up", "volume down", "vol up", "vol down", "raise the volume", "lower the volume", "volume {pct}",
        "volume to {pct}", "set the volume to {pct}", "set volume to {pct} percent", "volume at {pct}%", "make it louder", "make it quieter",
        "make it a bit louder", "can you make it louder", "crank it up", "crank up the volume", "max volume", "volume to max",
        "full volume", "half volume", "turn the sound up", "turn the sound down", "sound up", "sound down", "lower the sound",
        "mute", "unmute", "mute my pc", "mute the computer", "mute the speakers", "mute the volume", "unmute the sound", "sound off",
        "sound on", "i can't hear anything, turn it up", "i can't hear it", "it's too quiet", "way too quiet", "it's too loud",
        "too loud", "bring the volume up", "bring the volume down", "increase the volume", "decrease the volume", "bump the volume down",
        "volume a little higher", "volume a bit lower", "set the sound to {pct}", "pc volume to {pct}", "brighter", "dimmer",
        "brightness up", "brightness down", "brightness {pct}", "brightness to {pct}", "set brightness to {pct} percent",
        "set the brightness to {pct}%", "make it brighter", "make the screen dimmer", "make the screen darker", "screen is too bright",
        "screen is too dark", "the screen is too dim", "raise the brightness", "lower the brightness", "increase the screen brightness",
        "decrease brightness", "dim the display", "dim it", "dim the screen a bit", "brighten the screen", "max brightness",
        "full brightness", "less bright", "more brightness", "turn the brightness up", "turn the brightness down", "lower the screen light",
        "turn up the brightness", "turn down the volume", "turn up the volume", "silence my pc", "the laptop is too loud",
    ],
    "S2": [
        "lock", "lock it", "lock pc", "lock the pc", "lock my pc", "lock the computer", "lock the laptop", "lock my laptop",
        "lock screen", "lock my screen", "lock the screen", "lock this machine", "lock up the pc", "lock my computer now",
        "i'm stepping away, lock it", "going to lunch, lock the screen", "secure the screen", "lock the display", "lock windows",
        "please lock my pc", "lock this laptop",
    ],
    "S3": [
        "wifi off", "wifi on", "wi-fi off", "wi-fi on", "bluetooth off", "bluetooth on", "bt on", "bt off", "turn off wifi",
        "turn on wifi", "turn off bluetooth", "turn on bluetooth", "switch off wifi", "switch on bluetooth", "kill the wifi",
        "disable bluetooth", "enable bluetooth", "disable wi-fi", "enable wifi", "toggle wifi", "toggle bluetooth", "disconnect the wifi",
        "turn the bluetooth off", "turn the wifi on", "wifi please", "switch wifi off", "switch bluetooth on", "wireless off",
        "turn wifi back on", "put wifi on", "stop the wifi", "shut off bluetooth",
    ],
    "S4": [
        "dnd", "dnd on", "dnd off", "turn on dnd", "turn off dnd", "do not disturb", "do not disturb on", "turn on do not disturb",
        "turn off do not disturb", "don't disturb me", "don't disturb me for {dur}", "focus assist on", "turn on focus assist",
        "notifications off", "turn off notifications", "mute notifications", "silence my notifications", "hold my notifications",
        "no notifications", "no notifications for {dur}", "stop the pings", "stop the notifications", "snooze notifications for {dur}",
        "quiet the notifications", "i don't want to be disturbed", "block notifications for {dur}", "pause notifications",
        "pause notifications for {dur}", "disable notifications", "dnd for {dur}", "do not disturb until {time}",
    ],
    "S5": [
        "screenshot", "ss", "take a ss", "take a screenshot", "grab a screenshot", "screenshot please", "screenshot this",
        "screenshot my screen", "capture the screen", "capture my screen", "screen capture", "take a screen capture", "print screen",
        "snip", "take a snip", "snap my screen", "save a screenshot", "screenshot of the screen", "take a picture of my screen",
        "grab my screen", "shot of the screen", "screen shot",
    ],
    # ---- OUT: in-scope-looking requests that are not V1 tasks ----
    "OUT": [
        "dim the lights", "dim the {room} lights", "turn off the lights in the {room}", "switch on the {room} light",
        "turn on the {appliance}", "turn off the {appliance}", "switch off the {appliance}", "set the thermostat to 22",
        "make the {room} warmer", "lock the front door", "turn the heater up",
        "stock price of {company}", "what is the {company} stock at", "how is {company} stock doing", "share price of {company}",
        "price of bitcoin", "should i buy {company} shares", "nifty today", "exchange rate for dollars to rupees",
        "set up a zoom meeting", "set up a zoom call with {person}", "start a zoom meeting", "schedule a zoom with {person}",
        "create a google meet link", "host a webex call", "join the zoom call", "set up a video call with {person}",
        "video call {person}", "dial {person}", "phone {person}",
        "restart my computer", "shut down my pc", "put the laptop to sleep", "log me out of windows", "hibernate the pc",
        "play a movie on netflix", "turn on the tv", "change the channel", "watch something on prime video",
        "text {person} on sms", "send {person} a text message", "dm {person} on instagram", "send a telegram to {person}",
        "send a message on slack", "post this on linkedin", "tweet about {topic}", 
        "buy tickets for the movie", "book a table for two", "get me an uber to the airport",
        "order a pizza", "pay my credit card bill", "check my bank balance", "recharge my phone",
        "translate this to hindi", "write a cover letter for me", "write code to sort a list", "solve my homework",
        "what's the meaning of life", "tell me a story", "sing a song", "read me the news",
        "print this document", "scan this page", "connect to the printer", "update windows",
        "backup my phone", "set an alarm on my phone", "turn on the hotspot on my phone", "take a photo", "record a video of me",
        "record my screen", "start screen recording", "make a voice memo", "open a new tab on my phone",
        "delete {file} forever", "wipe my disk", "clear my browser history", "reset my router", "change the wallpaper",
        "enable airplane mode on my phone",
        "fill out this visa application", "fill in the registration form", "complete my college application online",
        "fill out this survey", "apply for this job for me", "submit the online application", "sign up for this newsletter website",
    ],
}
