"""Applies the rule patches to extract2.py (kept as a script so edits can be replayed from the baseline)."""
import re, sys
p = "extract2.py"
s = open("data/extract_v1_baseline.py.txt", encoding="utf-8").read()


def rep(a, b):
    global s
    assert s.count(a) == 1, a[:60]
    s = s.replace(a, b)


# ---- dates, numbers
rep(r'''DATE_RE = re.compile(rf"\b\d{{1,2}}\s+(?:tomorrow|tmrw|today|tonight)\b|''',
    r'''MONTH = r"(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sept?(?:ember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)"
AGO = r"(?:(?:\d+|an?|one|two|three|four|five|six|a couple of|a few)\s+(?:days?|weeks?|months?|years?)\s+ago)"
ORD = r"(?:the\s+)?\d{1,2}(?:st|nd|rd|th)(?:\s+of\s+" + MONTH + r")?"
DATE_RE = re.compile(rf"\b{AGO}\b|\b{ORD}\b|\b{MONTH}\s+\d{{1,2}}(?:st|nd|rd|th)?\b|\b\d{{1,2}}\s+{MONTH}\b|\b\d{{1,2}}\s+(?:tomorrow|tmrw|today|tonight)\b|''')
rep(r'''NUM_RE = re.compile(r"\b\d+(?:\.\d+)?\s?(?:%|percent)|\b\d{1,3}(?:,\d{3})+\b|\b\d{3,}\b", re.I)''',
    r'''_NOT_TIME = r"(?![\d:]|\s?(?:am|pm|st|nd|rd|th)\b)"
NUM_RE = re.compile(r"\b\d+(?:\.\d+)?\s?(?:%|percent)|\b\d{1,3}(?:,\d{3})+\b|\b\d{3,}\b"
                    rf"|(?<=between )\d+{_NOT_TIME}|(?<=among )\d+{_NOT_TIME}|(?<=times )\d+{_NOT_TIME}|(?<=plus )\d+{_NOT_TIME}"
                    rf"|(?<=minus )\d+{_NOT_TIME}|(?<=divided by )\d+{_NOT_TIME}|\b\d+(?=\s+(?:between|among|ways|times|plus|minus|divided)\b)", re.I)''')

# ---- gazetteers
rep(r'''        "calculator", "vs code", "edge", "gmail", "drive", "onedrive"]''',
    r'''        "calculator", "vs code", "edge", "gmail", "drive", "onedrive",
        "slack", "zoom", "telegram", "discord", "skype", "firefox", "brave", "paint", "vlc", "steam", "onenote",
        "photoshop", "figma", "notion", "terminal", "file explorer", "task manager", "google meet", "microsoft teams", "visual studio code"]''')
rep(r'''"reddit", "google", "facebook", "twitter"]''',
    r'''"reddit", "google", "facebook", "twitter",
         "myntra", "zomato", "swiggy", "instagram", "stack overflow", "hacker news", "irctc", "bookmyshow", "quora",
         "ebay", "imdb", "hotstar", "prime video", "makemytrip", "paytm", "x.com"]''')
rep(r'''            "sound", "airplane mode"]''',
    r'''            "sound", "airplane mode", "quiet mode", "silent mode", "dark mode", "night light", "battery saver", "focus assist"]''')

# ---- file nouns
rep(r'''FILE_NOUN = r"(?:sheet|deck|report|resume|cv|contract|proposal|invoices?|document|pdfs?|photos?|screenshots?|receipts?|letter|thing|ticket|agreement|attachments?|notes|slides|spreadsheet|file|files)"
FILE_NP = re.compile(rf"\b(?:the|my|that|these|this)\s+(?:[\w-]+\s+){{0,2}}?{FILE_NOUN}\b", re.I)''',
    r'''FILE_NOUN = (r"(?:sheet|deck|report|resume|cv|contract|proposal|invoices?|documents?|docs?|pdfs?|photos?|pictures?|pics?|images?|screenshots?|receipts?|letter|thing|"
             r"ticket|agreement|attachments?|notes|slides|spreadsheet|file|files|slip|lease|scan|return|pass|policy|bills?|statements?|forms?|certificates?|"
             r"presentation|plan|itinerary|manual|brochure|transcript|payslip|licen[sc]e|summary|minutes|draft|ppt|essay|assignment|"
             r"timetable|budget|memo)")
FILE_NP = re.compile(rf"\b(?:the|my|that|these|this|those|our|his|her)\s+(?:[\w-]+\s+){{0,2}}?{FILE_NOUN}\b(?:\s+{FILE_NOUN}\b)*(?!\s+(?:folder|directory))", re.I)
FILE_POSS = re.compile(rf"\b(?:today|yesterday|tomorrow|tonight|(?:this|last|next)\s+\w+)'s\s+((?:[\w-]+\s+)?{FILE_NOUN}\b(?:\s+{FILE_NOUN}\b)*)", re.I)''')
rep(r'''        out += [("file", m.group(0), m.start()) for m in FILE_NP.finditer(text)]
''',
    r'''        out += [("file", m.group(0), m.start()) for m in FILE_NP.finditer(text)]
        out += [("file", m.group(1), m.start(1)) for m in FILE_POSS.finditer(text)]
''')

# ---- MAIL frame: lowercase names, file object after the name
rep(r'''        if m and m.group(2).lower() not in NOT_NAME and not re.search(FILE_NOUN + r"$", m.group(2), re.I) \
                and "." not in m.group(2) and not FILE_NP.search(m.group(3)) and not FILE_EXT.search(m.group(3)) \
                and m.group(1).lower() != "forward":
            add("contact", m, 2)
            if not re.match(r"(?:to|from|with)\b", m.group(3), re.I) and not DATE_RE.fullmatch(m.group(3)):
                add("message", m, 3)''',
    r'''        if m and m.group(2).lower() not in NOT_NAME and not re.fullmatch(FILE_NOUN, m.group(2), re.I) \
                and "." not in m.group(2) and m.group(1).lower() != "forward" \
                and not re.match(r"(?:to|from|with)\b", m.group(3), re.I):
            add("contact", m, 2)
            explicit = bool(text[m.end(2):m.start(3)].strip())  # "that" / "saying" / ":" before the message
            if explicit or (not DATE_RE.fullmatch(m.group(3)) and not FILE_NP.match(m.group(3)) and not FILE_EXT.search(m.group(3))
                            and not re.match(rf"(?:{FILE_NOUN})\b", m.group(3), re.I)):
                add("message", m, 3)''')

# ---- family words: only as addressee/owner; "1 hr" is a duration
rep(r'''        for m in re.finditer(rf"(?<![\w-]){w}(?![\w-])", text, re.I):
            if not any(''',
    r'''        for m in re.finditer(rf"(?<![\w-]){w}(?![\w-])", text, re.I):
            if w == "hr" and re.search(rf"(?:\d|\b{NUMW})\s*$", text[:m.start()], re.I):  # "1 hr" is a duration
                continue
            before = text[:m.start()].rstrip()
            if before and not re.search(r"\b(?:to|with|from|for|tell|ask|text|message|msg|ping|email|mail|dm|notify|whatsapp|forward|call|invite|share|cc)"
                                        r"\s*(?:my)?$", before, re.I) \
                    and not re.match(r"'s\b", text[m.end():], re.I) and not re.match(r"\s+(?:on|via|over)\s+\w+\s*$", text[m.end():], re.I):
                continue
            if not any(''')

# ---- put on <artist> is not an event
rep(r'''        m = rx.search(text)
        if not m:
            continue
        x = m.group(1)''',
    r'''        m = rx.search(text)
        if not m:
            continue
        if rx is ADD_EV and re.search(r"\bput\s+on\b", text, re.I) and not re.search(r"calendar|schedule", text, re.I):
            continue  # "put on <artist>" is music, not an event
        x = m.group(1)''')

# ---- nothing inside a message is an entity; unknown apps
rep(r'''            fixed += frames_
            kept = ''',
    r'''            fixed += frames_
            fixed += unknown_apps(text, raw, self.rev, self.threshold, fixed + out)
            kept = ''')
rep(r'''            out = kept + fixed
''',
    r'''            out = kept + fixed
            # nothing inside a message is an entity of its own ("tell Neha ask mom to call")
            msgs = [o for o in out if o[0] == "message"]
            out = [o for o in out if o[0] == "message" or not any(g[2] <= o[2] and o[3] <= g[3] for g in msgs)]
''')
rep(r'''_CACHE_LOADED = {}  #''', open("patch_unknown_apps.txt", encoding="utf-8").read() + "\n\n_CACHE_LOADED = {}  #")
open(p, "w", encoding="utf-8").write(s)
print("patched")
