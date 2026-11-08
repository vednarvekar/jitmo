s = open("extract2.py", encoding="utf-8").read()


def rep(a, b):
    global s
    assert s.count(a) == 1, a[:70]
    s = s.replace(a, b)


# 1. TALK anywhere after a conjunction ("call mom and tell Neha ...")
rep(r'''    def add(kind, m, g=1):
        if m and m.group(g) and m.group(g).strip():
            out.append((kind, m.group(g), m.start(g), m.end(g)))

    m = TALK.match(text)
    if m and m.group(2).lower() not in NOT_NAME:
        add("contact", m, 2)
        add("message", m, 4)
        rest = re.sub(r"^(?:that|to)\s+", "", m.group(4), flags=re.I)
        if rest != m.group(4):
            out.pop()
            out.append(("message", rest, m.end(4) - len(rest), m.end(4)))
    else:''', r'''    def add(kind, m, g=1, off=0):
        if m and m.group(g) and m.group(g).strip():
            out.append((kind, m.group(g), m.start(g) + off, m.end(g) + off))

    m, off = TALK.match(text), 0
    if not m:
        for c in re.finditer(r"\b(?:and|then|and then)\s+(?=(?:tell|message|msg|ping|text|ask|dm|notify)\b)", text, re.I):
            m, off = TALK.match(text[c.end():]), c.end()
            if m:
                break
    if m and m.group(2).lower() not in NOT_NAME:
        add("contact", m, 2, off)
        add("message", m, 4, off)
        rest = re.sub(r"^(?:that|to)\s+", "", m.group(4), flags=re.I)
        if rest != m.group(4):
            out.pop()
            out.append(("message", rest, m.end(4) + off - len(rest), m.end(4) + off))
    else:''')

# 2. "send <lowercase name> the <file>"
rep(r'''    m = POST.match(text)
    if m:
        add("message", m, 1)''', r'''    m = SEND2.search(text)
    if m and m.group(1).lower() not in NOT_NAME and not re.fullmatch(FILE_NOUN, m.group(1), re.I) and not any(o[0] == "contact" for o in out):
        add("contact", m, 1)
    m = POST.match(text)
    if m:
        add("message", m, 1)''')
rep(r'''PLAY = re.compile(''', r'''SEND2 = re.compile(r"\b(?:send|give|forward|share|text|mail|email|show)\s+([A-Za-z][\w'.-]*)\s+(?:the|that|my|this|a|an|these|those)\s+", re.I)
PLAY = re.compile(''')

# 3. trailing politeness after the song name
rep(r'''|\s+songs?\b.*|\s+music\b.*)?\s*[.?!]*$", re.I)''', r'''|\s+songs?\b.*|\s+music\b.*|\s+(?:please|pls|plz|for me|now|thanks)\b.*)?\s*[.?!]*$", re.I)''')

# 4. creator: drop a leading command verb
rep(r'''            if not any(o[0] == "media" for o in out) and m.group(1).lower() not in {"the", "my", "this", "that", "his", "her"}:
                add("media", m)''', r'''            if not any(o[0] == "media" for o in out) and m.group(1).lower() not in {"the", "my", "this", "that", "his", "her"}:
                g = re.match(r"(?:open|play|show|watch|get|see|find|pull up|put on)\s+", m.group(1), re.I)
                if g:
                    out.append(("media", m.group(1)[g.end():], m.start(1) + g.end(), m.end(1)))
                else:
                    add("media", m)''')

# 5. 'put me in X' is not an event
rep(r'''        if rx is ADD_EV and re.search(r"\bput\s+on\b"''', r'''        if m.group(1).strip().lower() in NOT_NAME | {"me", "us"}:
            continue
        if rx is ADD_EV and re.search(r"\bput\s+on\b"''')

# 6. short numbers survive the min-length filter
rep(r'''            if len(t.strip()) < 2:''', r'''            if len(t.strip()) < 2 and not t.strip().isdigit():''')

# 7. place spans: strip "capital of" style prefixes
rep(r'''            out.append((kind, e["text"], e["start"], e["end"]))
        if self.rules:''', r'''            if kind == "place":
                pre = re.match(r"(?:the\s+)?(?:capital|population|weather|time|temperature|flights?|hotels?|currency)\s+(?:of|in|at|to|for)\s+", e["text"], re.I)
                if pre:
                    e = dict(e, text=e["text"][pre.end():], start=e["start"] + pre.end())
            out.append((kind, e["text"], e["start"], e["start"] + len(e["text"])))
        if self.rules:''')
open("extract2.py", "w", encoding="utf-8").write(s)
print("patch2 ok")
