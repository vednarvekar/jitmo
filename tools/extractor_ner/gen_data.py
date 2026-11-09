"""Generate a labelled, diverse synthetic corpus for the slot extractor (BIO token classification).

Output: one sentence per line in the SP2 bracket format `ID|text with [type|span] marks`.

    python -I gen_data.py --n 6000 --seed 1 --out data/synth_train.txt \
        [--exclude-spans ../../spikes/sp2-extractor/data/fresh.txt]

Nothing here reads the blind set's sentences. `--exclude-spans` only removes entity strings that
occur as gold spans in the given file from the pools (so evaluation is entity-disjoint); the file's
text is parsed in memory and never printed.
"""
import argparse
import random
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import vocab as V  # noqa: E402

R = random.Random(0)
BR = re.compile(r"\[(\w+)\|(.*?)\]")


def pick(xs):
    return R.choice(xs)


def chance(p):
    return R.random() < p


# ---------------------------------------------------------------- slot value samplers
def hour():
    return R.choice([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])


def clock():
    h = hour()
    k = R.random()
    if k < 0.45:
        return f"{h}{pick(['am', 'pm'])}"
    if k < 0.65:
        return f"{h}:{pick(['00', '15', '30', '45'])}{pick([' am', ' pm', 'am', 'pm', ''])}".replace(":00", ":00")
    if k < 0.75:
        return f"{h} {pick(['am', 'pm'])}"
    if k < 0.85:
        return pick(["noon", "midnight", "6 in the evening", "half past five", "quarter to nine", "10 sharp"])
    return str(h)


def day_word():
    return pick(["today", "tonight", "tomorrow", "tmrw", "tomorrow", "yesterday", "this evening", "this morning", "this afternoon", "this weekend",
                 "next week", "next month", "next " + pick(V.DAYS), "this " + pick(V.DAYS), "last " + pick(V.DAYS), pick(V.DAYS), pick(V.DAYS),
                 "day after tomorrow", "tomorrow morning", "tomorrow evening", "tomorrow night", "Friday evening", "Monday morning", "Saturday afternoon",
                 "the weekend", "later today", "this week", "next weekend", "end of the month", "early next week", "this friday"])


def date_time():
    k = R.random()
    if k < 0.3:
        return day_word()
    if k < 0.55:
        return f"{day_word()} at {clock()}"
    if k < 0.65:
        return f"at {clock()}"
    if k < 0.72:
        return pick([f"on the {R.randint(1, 31)}th", f"on {pick(V.MONTHS)} {R.randint(1, 28)}", f"{R.randint(1, 28)} {pick(V.MONTHS)}", f"the {R.randint(1, 28)}th", f"on {pick(V.DAYS)}"]).replace("1th", "1st").replace("2th", "2nd").replace("3th", "3rd").replace("11st", "11th").replace("12nd", "12th").replace("13rd", "13th")
    if k < 0.8:
        return pick([f"{pick(V.WORD_NUMS)} {pick(['days', 'weeks', 'months'])} ago", f"{R.randint(2, 9)} days ago", "last week", "last month", "last summer", "last year", "a couple of weeks ago", "a week ago", "earlier this month"])
    if k < 0.88:
        return pick(["every day at " + clock(), "every " + pick(V.DAYS) + " at " + clock(), "every morning", "every weekday at " + clock(), "daily at " + clock(), "every " + pick(V.DAYS)])
    if k < 0.94:
        return clock()
    return pick(["in the morning", "in the evening", "after lunch", "before noon", "by friday", "by tonight", "this month", "within the week"])


def duration():
    k = R.random()
    n = R.choice(["5", "10", "15", "20", "30", "45", "90", "2", "3", "1", "12", "25", "40", "60"])
    unit_m = pick(["minutes", "mins", "min", "minute"])
    if k < 0.45:
        return f"{n} {unit_m}"
    if k < 0.6:
        return pick(["an hour", "one hour", "two hours", "three hours", "half an hour", "an hour and a half", "2 hours", "1 hr", "2 hrs", "an hour", "a few hours", "30 seconds", "45 seconds", "ten seconds"])
    if k < 0.75:
        return f"{pick(V.WORD_NUMS)} {pick(['minutes', 'hours', 'mins', 'seconds'])}"
    if k < 0.85:
        return pick(["the next hour", "the next 30 minutes", "the next two hours", "the rest of the day", "till tomorrow", "until 5pm", "all day", "the whole afternoon"])
    return f"{n}-{pick(['minute', 'hour', 'second'])}"


def number():
    return pick([str(R.randint(2, 99)), f"{R.randint(2, 40)}%", f"{R.randint(1, 3)},{R.randint(100, 999)}", str(R.randint(100, 9999)), f"{R.randint(1, 30)}.{R.randint(1, 9)}", f"{R.randint(5, 90)} percent", str(R.randint(2, 20))])


def person():
    k = R.random()
    if k < 0.5:
        return pick(V.FIRST)
    if k < 0.6:
        return pick(V.FIRST) + " " + pick(V.SURNAMES)
    if k < 0.78:
        return pick(V.RELATIONS)
    if k < 0.88:
        return pick(V.GROUPS)
    if k < 0.95:
        return pick(V.FIRST).lower()
    return pick(V.FIRST) + pick(["ji", "bhai", " bhaiya", " didi", " sir", " uncle"])


def person_sender():
    k = R.random()
    if k < 0.7:
        return person()
    return pick(V.SENDERS)


def app():
    return pick(V.APPS)


def site():
    return pick(V.SITES)


def artist():
    return pick(V.ARTISTS)


def song():
    return pick(V.SONGS)


def media_any():
    k = R.random()
    if k < 0.55:
        return artist()
    if k < 0.8:
        return song()
    if k < 0.93:
        return pick(V.GENRES)
    return pick(V.PLAYLISTS)


def file_name():
    k = R.random()
    if k < 0.3:
        return f"{pick(V.FILE_STEM).replace(' ', pick(['_', ' ', '-']))}.{pick(V.FILE_EXT)}"
    if k < 0.4:
        return f"{pick(V.FILE_TOPIC)} {pick(V.FILE_STEM)}.{pick(V.FILE_EXT)}"
    art = pick(["the ", "my ", "the ", "", "that ", "the latest ", "the last ", "the old ", "my old ", "the signed ", "the final ", "the new "])
    if k < 0.75:
        return f"{art}{pick(V.FILE_NOUNS)}"
    if k < 0.85:
        return f"{pick(['the ', 'my ', ''])}{pick(V.FILE_TOPIC)} {pick(V.FILE_NOUNS)}"
    if k < 0.92:
        return f"{pick(['the ', 'my '])}{pick(V.FILE_NOUNS)} from {pick(['last year', 'last week', 'yesterday', 'march', 'last month'])}".replace(" from last year", "").replace(" from last week", "").replace(" from yesterday", "").replace(" from march", "").replace(" from last month", "")
    return pick(V.FILES_PLURAL)


def file_plain():
    """File reference with no trailing date (for frames where the sentence adds its own)."""
    return file_name()


def folder():
    return pick(V.FOLDERS)


def place():
    return pick(V.PLACES)


def event():
    return pick(V.EVENTS)


def setting():
    return pick(V.SETTINGS)


def query():
    k = R.random()
    if k < 0.6:
        return pick(V.SEARCH_NOUN)
    if k < 0.8:
        return pick(V.SEARCH_TOPIC)
    base = pick(["best", "cheap", "top", "how to", "where to buy", "reviews of", "what is", "tips for", "guide to", "price of", "alternatives to", "how much is"])
    obj = pick(["laptops", "headphones", "sourdough", "mountain bikes", "the iphone", "a standing desk", "yoga mats", "protein powder", "tents", "espresso machines", "kindle", "tennis rackets", "air purifiers", "monitors", "keyboards"])
    return f"{base} {obj}"


def message():
    k = R.random()
    if k < 0.75:
        return pick(V.MSG_LEAD) + pick(V.MESSAGES)
    if k < 0.85:
        return pick(V.MSG_SOFT)
    a, b = pick(V.MESSAGES), pick(V.MESSAGES)
    return f"{a}, {b}"


def remind_task():
    if chance(0.8):
        return pick(V.REMIND_TASKS)
    return f"{pick(V.TASK_VERBS)} {pick(['the ', 'my ', ''])}{pick(['bank', 'dentist', 'landlord', 'parcel', 'laundry', 'form', 'tickets', 'rent', 'car', 'bills'])}"


P = {
    "DT": date_time, "DUR": duration, "NUM": number, "PERSON": person, "SENDER": person_sender, "APP": app, "SITE": site,
    "ARTIST": artist, "SONG": song, "MEDIA": media_any, "FILE": file_name, "FOLDER": folder, "PLACE": place, "EVENT": event,
    "SETTING": setting, "QUERY": query, "MSG": message, "TASK": remind_task, "CLOCK": clock,
    "UNIT": lambda: pick(V.CONVERT_UNITS), "COUNTRY": lambda: pick(V.COUNTRIES), "DAY": lambda: pick(V.DAYS),
    "VIDEO": lambda: pick(V.VIDEO_THINGS), "WN": lambda: pick(V.WORD_NUMS), "TOPIC": lambda: pick(V.SEARCH_TOPIC),
    "FNOUN": lambda: pick(V.FILE_NOUNS), "SEARCHAPP": lambda: pick(["YouTube", "Google", "Amazon", "Spotify", "Wikipedia", "Maps", "Reddit", "GitHub"]),
    "MSGAPP": lambda: pick(["WhatsApp", "Telegram", "Slack", "Teams", "Signal", "Discord", "iMessage", "Messenger", "SMS", "Instagram"]),
    "MAILAPP": lambda: pick(["Gmail", "Outlook", "mail"]),
}

# ---------------------------------------------------------------- templates
# [type|{POOL}] marks a labelled span; a bare {POOL} is unlabelled filler. Pools drawn once per mark.
T = []


def add(w, *tpls):
    for t in tpls:
        T.append((w, t))


# open app / site
add(3, "open [app|{APP}]", "launch [app|{APP}]", "start [app|{APP}]", "fire up [app|{APP}]", "bring up [app|{APP}]", "get [app|{APP}] running", "boot up [app|{APP}]",
    "switch to [app|{APP}]", "run [app|{APP}]", "can you open [app|{APP}] for me", "i need [app|{APP}] open", "open up [app|{APP}] please", "load [app|{APP}]",
    "get [app|{APP}] going", "start up [app|{APP}] for me", "close [app|{APP}]", "quit [app|{APP}]", "kill [app|{APP}]", "shut down [app|{APP}]", "minimize [app|{APP}]",
    "pull up [app|{APP}]", "show me [app|{APP}]", "go to [app|{APP}]", "put [app|{APP}] on screen", "restart [app|{APP}]", "relaunch [app|{APP}]", "mute [app|{APP}] for [duration|{DUR}]",
    "fire [app|{APP}] up", "I want to use [app|{APP}]", "open [app|{APP}] and [app|{APP}]", "open [app|{APP}] then open [app|{APP}]")
add(3, "open [website|{SITE}]", "go to [website|{SITE}]", "take me to [website|{SITE}]", "pull up [website|{SITE}]", "head to [website|{SITE}]", "navigate to [website|{SITE}]",
    "visit [website|{SITE}]", "load [website|{SITE}] in the browser", "launch [website|{SITE}]", "browse [website|{SITE}]", "get me to [website|{SITE}]", "show me [website|{SITE}]",
    "open [website|{SITE}] in chrome", "what's on [website|{SITE}] right now", "check [website|{SITE}]", "bring up [website|{SITE}]", "what's new on [website|{SITE}]",
    "take me to the [website|{SITE}] site", "go on [website|{SITE}]", "i want to see [website|{SITE}]")
# search
add(4, "search [website|{SITE}] for [query|{QUERY}]", "search for [query|{QUERY}]", "google [query|{QUERY}]", "look up [query|{QUERY}]", "lookup [query|{QUERY}]",
    "search [app|{SEARCHAPP}] for [query|{QUERY}]", "find [query|{QUERY}] on [website|{SITE}]", "look for [query|{QUERY}]", "search the web for [query|{QUERY}]",
    "find videos about [query|{QUERY}] on [app|YouTube]", "search for [query|{QUERY}] on [app|YouTube]", "bing [query|{QUERY}]", "i want to know [query|{QUERY}]",
    "can you google [query|{QUERY}]", "find me [query|{QUERY}]", "show me results for [query|{QUERY}]", "search [query|{QUERY}] on [website|{SITE}]", "do a search for [query|{QUERY}]",
    "browse for [query|{QUERY}]", "youtube [query|{QUERY}]", "find tutorials on [query|{QUERY}]", "look up [query|{QUERY}] on [website|{SITE}]", "look [query|{QUERY}] up",
    "search [website|{SITE}] for [query|{QUERY}] under [number|{NUM}]", "find [query|{QUERY}] near [place|{PLACE}]", "find articles about [query|{QUERY}]")
# music / media
add(4, "play [media|{ARTIST}]", "play [media|{SONG}]", "play [media|{SONG}] by [media|{ARTIST}]", "play [media|{SONG}] by [media|{ARTIST}] on [app|Spotify]", "put on [media|{ARTIST}]",
    "play [media|{ARTIST}] on [app|Spotify]", "play some [media|{ARTIST}]", "queue up [media|{SONG}]", "i wanna hear [media|{ARTIST}]", "i feel like listening to [media|{ARTIST}]",
    "throw on [media|{MEDIA}]", "play me some [media|{MEDIA}]", "shuffle [media|{MEDIA}]", "put on some [media|{MEDIA}]", "can you play [media|{MEDIA}]", "play [media|{MEDIA}] please",
    "start playing [media|{MEDIA}]", "i want to listen to [media|{MEDIA}] while i work", "spin some [media|{MEDIA}]", "play something by [media|{ARTIST}]", "put [media|{ARTIST}] on [app|Spotify]",
    "play [media|{SONG}] on [app|YouTube Music]", "play the latest from [media|{ARTIST}]", "play [media|{ARTIST}]'s new album", "play [media|{ARTIST}]'s top songs", "play [media|{SONG}] on repeat",
    "play some [media|{MEDIA}] on [app|{APP}]", "blast [media|{ARTIST}]", "put on [media|{MEDIA}] for the gym", "play me [media|{SONG}]", "i need some [media|{MEDIA}]", "play my [media|{MEDIA}]",
    "what's the newest thing from [media|{ARTIST}]", "open [media|{ARTIST}]'s {VIDEO} on [app|YouTube]", "open that new [media|{ARTIST}] vid", "play [media|{ARTIST}]'s {VIDEO}",
    "show me the {VIDEO} from [media|{ARTIST}]", "any new videos from [media|{ARTIST}]", "latest upload by [media|{ARTIST}]", "queue [media|{SONG}] after this", "listen to [media|{MEDIA}]",
    "turn on some [media|{MEDIA}]", "music by [media|{ARTIST}] please", "put on [media|{SONG}] by [media|{ARTIST}]", "skip to [media|{SONG}]", "add [media|{SONG}] to my queue")
add(1, "pause the music", "skip this song", "next song", "pause", "resume", "previous track", "stop the music", "resume the music", "pause [app|{APP}]", "next track on [app|Spotify]")
# messaging
add(6, "tell [contact|{PERSON}] [message|{MSG}]", "tell [contact|{PERSON}] that [message|{MSG}]", "text [contact|{PERSON}] [message|{MSG}]", "message [contact|{PERSON}] [message|{MSG}]",
    "tell [contact|{PERSON}] on [app|{MSGAPP}] [message|{MSG}]", "text [contact|{PERSON}] on [app|{MSGAPP}] saying [message|{MSG}]", "send [contact|{PERSON}] a message saying [message|{MSG}]",
    "drop [contact|{PERSON}] a text, [message|{MSG}]", "let [contact|{PERSON}] know [message|{MSG}]", "let [contact|{PERSON}] know on [app|{MSGAPP}] that [message|{MSG}]",
    "ask [contact|{PERSON}] [message|{MSG}]", "reply to [contact|{PERSON}] with [message|{MSG}]", "reply [message|{MSG}] to [contact|{PERSON}]", "whatsapp [contact|{PERSON}] [message|{MSG}]",
    "send [message|{MSG}] to [contact|{PERSON}]", "send \"[message|{MSG}]\" to [contact|{PERSON}]", "say [message|{MSG}] to [contact|{PERSON}]", "message [contact|{PERSON}] on [app|{MSGAPP}]: [message|{MSG}]",
    "ping [contact|{PERSON}] [message|{MSG}]", "dm [contact|{PERSON}] [message|{MSG}]", "shoot [contact|{PERSON}] a text [message|{MSG}]", "write to [contact|{PERSON}]: [message|{MSG}]",
    "email [contact|{PERSON}] that [message|{MSG}]", "send an email to [contact|{PERSON}] saying [message|{MSG}]", "mail [contact|{PERSON}] [message|{MSG}]", "tell my [contact|{RELATIONS}] [message|{MSG}]",
    "tell [contact|{PERSON}] to [message|{MSG}]", "can you tell [contact|{PERSON}] [message|{MSG}]", "i want to tell [contact|{PERSON}] [message|{MSG}]", "text [contact|{PERSON}]: [message|{MSG}]",
    "send [contact|{PERSON}] a whatsapp saying [message|{MSG}]", "post in [contact|{PERSON}]: [message|{MSG}]", "leave [contact|{PERSON}] a note saying [message|{MSG}]", "tell [contact|{PERSON}] and [contact|{PERSON}] [message|{MSG}]",
    "send [message|{MSG}] to [contact|{PERSON}] on [app|{MSGAPP}]", "message [contact|{PERSON}] on [app|{MSGAPP}] and say [message|{MSG}]", "call [contact|{PERSON}] and tell [contact|{PERSON}] [message|{MSG}]")
add(3, "call [contact|{PERSON}]", "call [contact|{PERSON}] [date_time|{DT}]", "phone [contact|{PERSON}]", "ring [contact|{PERSON}]", "video call [contact|{PERSON}] on [app|{MSGAPP}]", "dial [contact|{PERSON}]",
    "call [contact|{PERSON}] on [app|{MSGAPP}]", "set up a call with [contact|{PERSON}] [date_time|{DT}]", "schedule a call with [contact|{PERSON}] [date_time|{DT}]", "get [contact|{PERSON}] on the phone",
    "message [contact|{PERSON}] on [app|{MSGAPP}]", "open my chat with [contact|{PERSON}]", "open [contact|{PERSON}]'s chat on [app|{MSGAPP}]", "text [contact|{PERSON}]", "whatsapp [contact|{PERSON}]")
add(3, "did [contact|{PERSON}] message me on [app|{MSGAPP}]", "show me unread stuff from [contact|{SENDER}]", "any mail from [contact|{SENDER}] [date_time|{DT}]", "what's [contact|{PERSON}]'s latest email say",
    "show emails from [contact|{SENDER}]", "any new messages from [contact|{PERSON}]", "find the email from [contact|{SENDER}] about [query|{TOPIC}]", "dig out emails about [query|{TOPIC}] from [date_time|{DT}]",
    "summarize the thread with [contact|{PERSON}] about [query|{TOPIC}]", "archive everything from [contact|{SENDER}]", "delete all mail from [contact|{SENDER}]", "any new [app|{MSGAPP}] messages",
    "what did [contact|{PERSON}] say", "read my last message from [contact|{PERSON}]", "find emails about [query|{TOPIC}] from [date_time|{DT}]", "show unread email from [contact|{SENDER}] [date_time|{DT}]",
    "mark everything from [contact|{SENDER}] as read", "what did [contact|{PERSON}] send me [date_time|{DT}]", "is there anything from [contact|{SENDER}]", "search my inbox for [query|{TOPIC}]",
    "find the email about [query|{TOPIC}]", "emails about [query|{TOPIC}] [date_time|{DT}]", "what did [contact|{PERSON}] email me about")
# send file
add(4, "send [file|{FILE}] to [contact|{PERSON}]", "send [file|{FILE}] to [contact|{PERSON}] on [app|{MSGAPP}]", "email [contact|{PERSON}] [file|{FILE}]", "share [file|{FILE}] with [contact|{PERSON}]",
    "whatsapp [file|{FILE}] to [contact|{PERSON}]", "can you fire off [file|{FILE}] to [contact|{PERSON}] on [app|{MSGAPP}]", "forward [file|{FILE}] to [contact|{PERSON}]", "attach [file|{FILE}] and mail it to [contact|{PERSON}]",
    "send this file to [contact|{PERSON}] on [app|{MSGAPP}]", "share the [folder|{FOLDER}] folder with [contact|{PERSON}]", "send [contact|{PERSON}] [file|{FILE}]", "drop [file|{FILE}] in [contact|{PERSON}]'s inbox",
    "send [file|{FILE}] from [contact|{PERSON}]'s email to [contact|{PERSON}]", "upload [file|{FILE}] to [app|Drive] and share with [contact|{PERSON}]", "give [contact|{PERSON}] access to [file|{FILE}]",
    "send [file|{FILE}] to [contact|{PERSON}] and [contact|{PERSON}]", "pass [file|{FILE}] to [contact|{PERSON}]", "text [contact|{PERSON}] [file|{FILE}]", "get [file|{FILE}] over to [contact|{PERSON}]")
# files
add(5, "open [file|{FILE}]", "find [file|{FILE}]", "where is [file|{FILE}]", "wheres [file|{FILE}]", "look for [file|{FILE}]", "where did i put [file|{FILE}]", "where did i save [file|{FILE}]", "show me [file|{FILE}]",
    "pull up [file|{FILE}]", "open up [file|{FILE}]", "gimme [file|{FILE}]", "dig out [file|{FILE}]", "find [file|{FILE}] from [date_time|{DT}]", "open [file|{FILE}] in [app|{APP}]", "locate [file|{FILE}]",
    "can you find [file|{FILE}]", "i need [file|{FILE}]", "get [file|{FILE}] for me", "print [file|{FILE}]", "delete [file|{FILE}]", "rename [file|{FILE}]", "show [file|{FILE}] in the folder",
    "what's in [file|{FILE}]", "read [file|{FILE}] to me", "summarize [file|{FILE}]", "open [app|{APP}] and find [file|{FILE}]", "search my files for [file|{FILE}]", "zip up [file|{FILE}]", "compress [file|{FILE}]",
    "scan my downloads for [file|{FILE}]", "attach [file|{FILE}]")
add(5, "move [file|{FILE}] to the [folder|{FOLDER}] folder", "move [file|{FILE}] to [folder|{FOLDER}]", "move [file|{FILE}] into [folder|{FOLDER}]", "put [file|{FILE}] in [folder|{FOLDER}]",
    "drag [file|{FILE}] into the [folder|{FOLDER}] folder", "copy [file|{FILE}] into the [folder|{FOLDER}] folder", "copy [file|{FILE}] to [folder|{FOLDER}]", "file [file|{FILE}] under [folder|{FOLDER}]",
    "save [file|{FILE}] to the [folder|{FOLDER}] folder", "stick [file|{FILE}] in [folder|{FOLDER}]", "shove [file|{FILE}] into [folder|{FOLDER}]", "organize [file|{FILE}] into [folder|{FOLDER}]",
    "move the [folder|{FOLDER}] folder to [folder|{FOLDER}]", "open the [folder|{FOLDER}] folder", "open [folder|{FOLDER}]", "show me the [folder|{FOLDER}] folder", "go to my [folder|{FOLDER}] folder",
    "put [file|{FILE}] on my [folder|{FOLDER}]", "relocate [file|{FILE}] to [folder|{FOLDER}]", "send [file|{FILE}] to the [folder|{FOLDER}] folder", "create a [folder|{FOLDER}] folder", "make a new folder called [folder|{FOLDER}]",
    "save [file|{FILE}] from [contact|{PERSON}]'s email to the [folder|{FOLDER}] folder", "move everything in [folder|{FOLDER}] to [folder|{FOLDER}]")
add(3, "what did i download [date_time|{DT}]", "show my recent downloads", "show me what i downloaded [date_time|{DT}]", "my latest screenshot", "latest download", "show recent files", "recent files from [date_time|{DT}]",
    "what files did i edit [date_time|{DT}]", "anything new in [folder|{FOLDER}]", "list files in [folder|{FOLDER}]", "what's in my [folder|{FOLDER}] folder")
# calendar / events
add(5, "put [event|{EVENT}] on my calendar for [date_time|{DT}]", "add [event|{EVENT}] [date_time|{DT}]", "schedule [event|{EVENT}] [date_time|{DT}]", "book [event|{EVENT}] [date_time|{DT}]",
    "move my [event|{EVENT}] to [date_time|{DT}]", "push the [event|{EVENT}] to [date_time|{DT}]", "reschedule [event|{EVENT}] to [date_time|{DT}]", "cancel [event|{EVENT}]", "cancel my [event|{EVENT}] [date_time|{DT}]",
    "decline the [event|{EVENT}] on [date_time|{DT}]", "say no to the [event|{EVENT}]", "accept the [event|{EVENT}] invite", "add [event|{EVENT}] to my calendar [date_time|{DT}]", "set up [event|{EVENT}] with [contact|{PERSON}] [date_time|{DT}]",
    "invite [contact|{PERSON}] to [event|{EVENT}] [date_time|{DT}]", "block [duration|{DUR}] on [date_time|{DT}] for [event|{EVENT}]", "block [duration|{DUR}] [date_time|{DT}]", "keep [duration|{DUR}] free [date_time|{DT}]",
    "am i free [date_time|{DT}]", "am i busy [date_time|{DT}]", "what's on my calendar [date_time|{DT}]", "what do i have [date_time|{DT}]", "when is [event|{EVENT}]", "when's my next [event|{EVENT}]",
    "when is [contact|{PERSON}] free [date_time|{DT}]", "when are [contact|{PERSON}] and i free [date_time|{DT}]", "find a time for [event|{EVENT}] with [contact|{PERSON}]", "move my [date_time|{CLOCK}] to [date_time|{CLOCK}]",
    "remove [event|{EVENT}] from my calendar", "delete the [event|{EVENT}] on [date_time|{DT}]", "make [event|{EVENT}] [duration|{DUR}] long", "extend [event|{EVENT}] by [duration|{DUR}]", "reserve [event|{EVENT}] [date_time|{DT}]",
    "put me down for [event|{EVENT}] [date_time|{DT}]", "i have [event|{EVENT}] [date_time|{DT}]", "set a meeting with [contact|{PERSON}] [date_time|{DT}]", "schedule a meeting with [contact|{PERSON}] about [query|{TOPIC}] [date_time|{DT}]",
    "add [event|{EVENT}] to my calendar", "calendar entry for [event|{EVENT}] [date_time|{DT}]", "remind me about [event|{EVENT}] [date_time|{DT}]", "join the [event|{EVENT}]", "skip [event|{EVENT}] [date_time|{DT}]")
# reminders
add(5, "remind me to [message|{TASK}] at [date_time|{CLOCK}]", "remind me [date_time|{DT}] to [message|{TASK}]", "remind me to [message|{TASK}] [date_time|{DT}]", "remind me in [duration|{DUR}] to [message|{TASK}]",
    "ping me to [message|{TASK}] [date_time|{DT}]", "set a reminder to [message|{TASK}] [date_time|{DT}]", "reminder: [message|{TASK}] [date_time|{DT}]", 
    "nudge me to [message|{TASK}] [date_time|{DT}]", "don't let me forget to [message|{TASK}]", "[date_time|{DT}] remind me to [message|{TASK}]", "[date_time|every {DAY} at {CLOCK}] remind me to [message|{TASK}]",
    "remind me [date_time|every day at {CLOCK}] to [message|{TASK}]", "alert me to [message|{TASK}] [date_time|{DT}]", "make a note to [message|{TASK}]", "remind me about [message|{TASK}] [date_time|{DT}]",
    "i need a reminder to [message|{TASK}] [date_time|{DT}]", "remind [contact|{PERSON}] to [message|{TASK}] [date_time|{DT}]")
add(3, "set a timer for [duration|{DUR}]", "timer [duration|{DUR}]", "set a [duration|{DUR}] timer", "start a [duration|{DUR}] timer", "countdown [duration|{DUR}]", "wake me up in [duration|{DUR}]", "start a timer, [duration|{DUR}]",
    "set an alarm for [date_time|{CLOCK}]", "set an alarm [date_time|{DT}]", "alarm at [date_time|{CLOCK}]", "wake me at [date_time|{CLOCK}]", "set a timer for [duration|{DUR}] for the pasta", "cancel the timer", "how long is left on my timer")
# system
add(4, "turn [setting|{SETTING}] on", "turn [setting|{SETTING}] off", "turn on [setting|{SETTING}]", "turn off [setting|{SETTING}]", "switch [setting|{SETTING}] on", "enable [setting|{SETTING}]", "disable [setting|{SETTING}]",
    "toggle [setting|{SETTING}]", "turn the [setting|{SETTING}] up", "turn the [setting|{SETTING}] down", "make the [setting|{SETTING}] higher", "make the screen [setting|{SETTING}] lower", "set [setting|{SETTING}] to [number|{NUM}]",
    "put the laptop on [setting|{SETTING}]", "silence [setting|{SETTING}] for [duration|{DUR}]", "mute [setting|{SETTING}] for [duration|{DUR}]", "switch on [setting|{SETTING}]", "kill the [setting|{SETTING}]", "clear [setting|{SETTING}]",
    "[setting|{SETTING}] to [number|{NUM}] percent", "raise the [setting|{SETTING}]", "lower the [setting|{SETTING}]", "crank up the [setting|{SETTING}]", "{SETTING_ADJ} the [setting|{SETTING}]", "keep [setting|{SETTING}] on for [duration|{DUR}]",
    "don't disturb me for [duration|{DUR}]", "turn on [setting|do not disturb] for [duration|{DUR}]", "put me on [setting|do not disturb]", "stop notifications for [duration|{DUR}]", "pause [setting|notifications] for [duration|{DUR}]",
    "can you turn [setting|{SETTING}] off", "i need [setting|{SETTING}] off", "make it brighter", "make it louder", "lock the screen", "lock my laptop", "take a screenshot", "grab a screenshot", "mute", "unmute")
# calc / facts / weather / place
add(4, "what is [number|{NUM}] percent of [number|{NUM}]", "what's [number|{NUM}] of [number|{NUM}]", "[number|{NUM}] percent tip on [number|{NUM}]", "convert [number|{NUM}] {UNIT_A} to {UNIT_B}",
    "how many {UNIT_B} in [number|{NUM}] {UNIT_A}", "what's [number|{NUM}] times [number|{NUM}]", "[number|{NUM}] plus [number|{NUM}]", "[number|{NUM}] divided by [number|{NUM}]", "what's the tip on [number|{NUM}] at [number|{NUM}]",
    "split [number|{NUM}] between [number|{NUM}] people", "[number|{NUM}] minus [number|{NUM}]", "square root of [number|{NUM}]", "calculate [number|{NUM}] times [number|{NUM}]", "how much is [number|{NUM}] dollars in rupees")
add(4, "what's the weather in [place|{PLACE}]", "will it rain in [place|{PLACE}] [date_time|{DT}]", "weather in [place|{PLACE}] [date_time|{DT}]", "is it gonna pour in [place|{PLACE}] [date_time|{DT}]", "how hot is it in [place|{PLACE}]",
    "what time is it in [place|{PLACE}]", "time right now in [place|{PLACE}]", "when does the sun set in [place|{PLACE}]", "sunrise [date_time|{DT}] in [place|{PLACE}]", "is it raining in [place|{PLACE}]", "temperature in [place|{PLACE}]",
    "set my city to [place|{PLACE}]", "my location is [place|{PLACE}]", "i live in [place|{PLACE}]", "change my city to [place|{PLACE}]", "how far is [place|{PLACE}] from here", "what's the time in [place|{PLACE}] now",
    "forecast for [place|{PLACE}] this weekend", "is it cold in [place|{PLACE}]", "distance to [place|{PLACE}]", "directions to [place|{PLACE}]", "population of [place|{PLACE}]", "will it rain [date_time|{DT}]", "do i need an umbrella [date_time|{DT}]",
    "gonna rain tmrw", "how's the weather [date_time|{DT}]", "what time is it in [place|{PLACE}] right now", "what's the capital of {COUNTRY}", "who wrote {BOOK}", "how tall is {PEAK}", "when is the next full moon", "next new moon")
add(2, "tell me a joke", "what time is it", "what's the date today", "good morning", "hello", "thanks", "never mind", "who are you", "what can you do", "how are you", "help", "dim the lights in my living room", "order me a pizza",
    "book me a cab", "what's the meaning of life", "write me a poem", "translate hello to french", "what's the news", "turn on the tv", "play a video game", "what is photosynthesis", "do my homework", "who won the match last night")

# ------------------------------------------------ text surface noise
FILLER_PRE = ["hey ", "hey, ", "ok ", "okay, ", "pls ", "please ", "yo ", "um ", "quick, ", "can you ", "could you ", "could you please ", "would you ", "i want you to ", "i'd like you to ", "i need you to ",
              "hey could you ", "hi, ", "just ", "go ahead and ", "can u ", "plz ", "hey assistant, ", "so ", "right, ", "alright ", "if you can ", "wanna ", "hey can you "]
FILLER_POST = [" please", " pls", " for me", " real quick", " thanks", " now", " asap", " if you can", " when you get a chance", " right now", " thx", " okay", " quickly", " for me please", " would you", " plz"]
CONTR = [("going to", "gonna"), ("want to", "wanna"), ("you", "u"), ("please", "pls"), ("tomorrow", "tmrw"), ("what is", "what's"), ("do not", "don't"), ("I am", "i'm")]


def typo(word):
    if len(word) < 4:
        return word
    i = R.randrange(1, len(word) - 1)
    k = R.random()
    if k < 0.35:
        return word[:i] + word[i + 1:]
    if k < 0.65:
        return word[:i] + word[i + 1] + word[i] + word[i + 2:] if i + 2 <= len(word) else word
    if k < 0.85:
        return word[:i] + word[i] + word[i:]
    return word[:i] + R.choice("aeiouqwrtyp") + word[i + 1:]


def fill_template(tpl):
    """Return list of (text, label|None) segments."""
    extra = {
        "{RELATIONS}": lambda: pick([r.replace("my ", "") for r in V.RELATIONS]),
        "{SETTING_ADJ}": lambda: pick(["increase", "decrease", "boost", "reduce", "bump up", "drop", "lower", "raise"]),
        "{UNIT_A}": None, "{UNIT_B}": None, "{BOOK}": lambda: pick(["Pride and Prejudice", "Hamlet", "The Alchemist", "1984", "Dune", "Moby Dick", "The Hobbit", "Ulysses"]),
        "{PEAK}": lambda: pick(["Mount Everest", "K2", "the Eiffel Tower", "Burj Khalifa", "Mount Fuji", "the Statue of Liberty"]),
    }
    ua, ub = pick(V.CONVERT_UNITS)
    out = []
    pos = 0
    pieces = re.split(r"(\[\w+\|.*?\])", tpl)
    for piece in pieces:
        m = BR.fullmatch(piece)
        if m:
            label, inner = m.group(1), m.group(2)
            out.append((expand(inner, extra, ua, ub), label))
        elif piece:
            out.append((expand(piece, extra, ua, ub), None))
    return out


def expand(s, extra, ua, ub):
    def rep(m):
        key = m.group(0)
        name = key[1:-1]
        if key == "{UNIT_A}":
            return ua
        if key == "{UNIT_B}":
            return ub
        if key in extra and extra[key]:
            return extra[key]()
        if name in P:
            return P[name]()
        return key
    return re.sub(r"\{[A-Z_]+\}", rep, s)


def render(segs, lower_p=0.35):
    """Apply surface noise; return the bracket-format string."""
    segs = [list(s) for s in segs]
    # merge adjacent unlabeled
    if chance(0.55):
        pre = pick(FILLER_PRE)
        segs.insert(0, [pre, None])
    if chance(0.3):
        segs.append([pick(FILLER_POST), None])
    # contractions on plain text
    for s in segs:
        if s[1] is None and chance(0.2):
            for a, b in CONTR:
                if a in s[0] and chance(0.5):
                    s[0] = s[0].replace(a, b, 1)
    # typos
    for s in segs:
        if chance(0.04 if s[1] is None else 0.05):
            words = s[0].split(" ")
            cand = [i for i, w in enumerate(words) if len(w) >= 4 and w.isalpha()]
            if cand:
                i = pick(cand)
                words[i] = typo(words[i])
                s[0] = " ".join(words)
    lower = chance(lower_p)
    cap = (not lower) and chance(0.35)
    text = ""
    marks = []
    for s in segs:
        t = s[0].lower() if lower else s[0]
        if s[1]:
            marks.append((len(text), len(text) + len(t), s[1]))
        text += t
    if cap and text:
        text = text[0].upper() + text[1:]
    tail = pick(["", "", "", ".", "?", "!"]) if not lower else pick(["", "", "", "", ".", "?"])
    if tail and not text.endswith(("?", ".", "!")):
        text += tail
    # re-bracket
    res = ""
    last = 0
    for a, b, lab in marks:
        res += text[last:a] + f"[{lab}|{text[a:b]}]"
        last = b
    res += text[last:]
    return res


def ok_sentence(s):
    # no empty spans, and spans must not contain brackets/pipes
    for m in BR.finditer(s):
        if not m.group(2).strip():
            return False
    return True


def build_pools_exclusion(path):
    """Drop pool entries equal to a gold span of the given file (entity-disjoint evaluation)."""
    bad = set()
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        for m in BR.finditer(line):
            bad.add(m.group(2).lower().strip())
    n0 = 0
    for name in dir(V):
        val = getattr(V, name)
        if name.isupper() and isinstance(val, list) and val and isinstance(val[0], str):
            keep = [x for x in val if x.lower() not in bad]
            n0 += len(val) - len(keep)
            setattr(V, name, keep)
    return n0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=6000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", required=True)
    ap.add_argument("--exclude-spans", default=None)
    ap.add_argument("--prefix", default="S")
    a = ap.parse_args()
    R.seed(a.seed)
    if a.exclude_spans:
        removed = build_pools_exclusion(a.exclude_spans)
        print(f"pool entries removed (entity-disjoint): {removed}")
    weights = [w for w, _ in T]
    seen = set()
    lines = []
    tries = 0
    while len(lines) < a.n and tries < a.n * 20:
        tries += 1
        w, tpl = R.choices(T, weights=weights)[0]
        s = render(fill_template(tpl))
        if not ok_sentence(s) or s in seen:
            continue
        seen.add(s)
        lines.append(f"{a.prefix}{len(lines) + 1:05d}|{s}")
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(lines)} sentences ({len(T)} templates) to {a.out}")


if __name__ == "__main__":
    main()
