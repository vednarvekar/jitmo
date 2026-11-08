"""SP3 planner: turn a chained request into a JSON plan with a small local model.

Output is forced by a GBNF grammar built from the skill catalog, so the model can only
emit known skill ids and the slot names that belong to that skill.
"""
import json
import re
import time

# (skill id, slot names in fixed order, short description, scope task id)
SKILLS = [
    ("app.open", ["app"], "open an app", "A1"),
    ("app.close", ["app"], "close an app", "A1"),
    ("music.play", ["query"], "play a song, artist or playlist on Spotify", "A2"),
    ("music.control", ["action", "value"], "pause, resume, skip, previous, volume of the music player", "A3"),
    ("youtube.latest", ["creator"], "open a creator's latest YouTube video", "A4"),
    ("web.search", ["query", "site"], "search YouTube or the web (site = youtube or web)", "A5"),
    ("web.open", ["site"], "open a website", "A6"),
    ("whatsapp.send", ["contact", "text"], "send a WhatsApp message", "M1"),
    ("whatsapp.send_file", ["contact", "file"], "send a file on WhatsApp", "M2"),
    ("whatsapp.read", ["contact"], "read latest WhatsApp messages", "M3"),
    ("teams.post", ["channel", "text"], "post in a Teams chat or channel", "M4"),
    ("email.urgent", [], "show urgent unread emails", "E1"),
    ("email.summarize", ["target"], "summarize an email or thread", "E2"),
    ("email.find", ["person", "topic", "date"], "find emails by person, topic or date", "E3"),
    ("email.awaiting_reply", [], "show emails waiting for my reply", "E4"),
    ("email.draft_reply", ["target", "text"], "draft a reply for review", "E5"),
    ("email.organize", ["query", "action"], "archive, move or mark as read emails", "E6"),
    ("email.send", ["to", "subject", "body", "file"], "send an email, optionally with a file", "E7"),
    ("info.astro", ["what", "place", "date"], "moon phase, sunrise, sunset", "Q1"),
    ("info.weather", ["place", "date"], "weather forecast", "Q2"),
    ("info.fact", ["question"], "quick fact lookup", "Q3"),
    ("info.math", ["expression"], "math or unit conversion", "Q4"),
    ("info.time", ["place", "date"], "time or date in a place or time zone", "Q5"),
    ("files.find", ["query", "type", "date"], "find a file by name, type or description", "F1"),
    ("files.recent", ["kind", "date"], "show recent files or downloads", "F2"),
    ("files.open", ["file"], "open a file", "F3"),
    ("files.move", ["file", "dest"], "move files to a folder", "F4"),
    ("files.rename", ["file", "new_name"], "rename a file", "F4"),
    ("files.copy", ["file", "dest"], "copy files to a folder", "F4"),
    ("files.send", ["file", "to", "via"], "send a file by email or Teams (via = email or teams)", "F5"),
    ("files.share_link", ["item", "with"], "share a cloud folder or file link", "F6"),
    ("calendar.show", ["range"], "show today's or this week's schedule", "C1"),
    ("calendar.free_slot", ["people", "range", "duration"], "find a free slot with people", "C2"),
    ("calendar.meeting_prep", ["event"], "show emails and files for a meeting", "C3"),
    ("calendar.add", ["title", "when"], "add an event to the calendar", "C4"),
    ("calendar.block_focus", ["duration", "when"], "block focus time", "C5"),
    ("calendar.schedule", ["title", "people", "when", "duration"], "schedule a meeting and send the invite", "C6"),
    ("calendar.reschedule", ["event", "new_time"], "reschedule a meeting", "C7"),
    ("calendar.cancel", ["event"], "cancel a meeting", "C7"),
    ("calendar.respond", ["event", "response"], "accept or decline an invite", "C8"),
    ("reminder.set", ["text", "when"], "set a reminder", "R1"),
    ("timer.set", ["duration", "label"], "set a timer or alarm", "R2"),
    ("workflow.morning_brief", [], "morning brief: urgent emails and today's meetings", "W1"),
    ("workflow.email_to_event", ["email"], "turn an email into a calendar event", "W2"),
    ("workflow.save_attachments", ["email", "folder"], "save email attachments to a cloud folder", "W3"),
    ("workflow.forward_attachment", ["email", "chat"], "forward an email's attachment to a chat", "W4"),
    ("workflow.weekly_report", ["to"], "find the latest weekly report file and email it", "W5"),
    ("workflow.focus_mode", ["playlist", "duration"], "focus mode: music plus Do Not Disturb in one go", "W6"),
    ("system.volume", ["direction", "level"], "change system volume (direction = up, down, mute)", "S1"),
    ("system.brightness", ["direction", "level"], "change screen brightness (direction = up, down)", "S1"),
    ("system.lock", [], "lock the screen", "S2"),
    ("system.toggle", ["device", "state"], "turn Wi-Fi or Bluetooth on or off", "S3"),
    ("system.dnd", ["duration"], "turn on Do Not Disturb, mute notifications", "S4"),
    ("system.screenshot", [], "take a screenshot", "S5"),
]
SKILL_SLOTS = {s: slots for s, slots, _, _ in SKILLS}

# Few-shot examples. None of these is a T75-T82 request or a paraphrase of one.
FEWSHOT = [
    ("Pause the music and close Chrome.",
     [("music.control", {"action": "pause"}), ("app.close", {"app": "Chrome"})]),
    ("What's the weather in Mumbai on Sunday, then add trip to Lonavala on Sunday at 9am to my calendar.",
     [("info.weather", {"place": "Mumbai", "date": "Sunday"}),
      ("calendar.add", {"title": "trip to Lonavala", "when": "Sunday at 9am"})]),
    ("Find the signed contract and email it to Kavya with the subject Contract.",
     [("files.find", {"query": "signed contract"}),
      ("email.send", {"to": "Kavya", "subject": "Contract", "file": "$step1.result"})]),
    ("Remind me to pay rent on the 1st.",
     [("reminder.set", {"text": "pay rent", "when": "the 1st"})]),
    ("What's 40 euros in rupees? Then post it in the sales chat.",
     [("info.math", {"expression": "40 euros in rupees"}),
      ("teams.post", {"channel": "sales chat", "text": "$step1.result"})]),
    ("Find emails from the bank about my statement and summarize them.",
     [("email.find", {"person": "the bank", "topic": "statement"}),
      ("email.summarize", {"target": "$step1.result"})]),
    ("Show my calendar for today, then play Coldplay, and set a 5 minute timer.",
     [("calendar.show", {"range": "today"}), ("music.play", {"query": "Coldplay"}),
      ("timer.set", {"duration": "5 minutes"})]),
    ("Turn off Bluetooth.",
     [("system.toggle", {"device": "bluetooth", "state": "off"})]),
    ("Move my 2pm to Thursday and block an hour for focus on Friday afternoon.",
     [("calendar.reschedule", {"event": "2pm", "new_time": "Thursday"}),
      ("calendar.block_focus", {"duration": "an hour", "when": "Friday afternoon"})]),
]


def plan_json(steps):
    return json.dumps({"steps": [{"skill": s, "slots": sl} for s, sl in steps]},
                      separators=(",", ":"))


def system_prompt():
    lines = [
        "You are the planner of a Windows assistant. Turn the user's request into a short plan "
        "(at most 5 steps) that uses ONLY the skills below, in the order the user asked for them.",
        "Output ONLY JSON: {\"steps\":[{\"skill\":\"<id>\",\"slots\":{...}}]}",
        "Rules:",
        "- One step per thing the user asks for. Do not add steps the user did not ask for.",
        "- Fill slots with the user's own words (names, times, topics). Leave out slots the user did not mention.",
        "- If a step needs the output of an earlier step (a found file, a calendar answer, a screenshot, "
        "a calculation), put \"$stepN.result\" as that slot's value, where N is that step's number.",
        "- \"it\" and \"them\" after a find, show or take step refer to that step's result.",
        "",
        "Skills (id(slots): what it does):",
    ]
    for sid, slots, desc, _ in SKILLS:
        lines.append(f"{sid}({','.join(slots)}): {desc}")
    return "\n".join(lines)


def messages_for(request):
    msgs = [{"role": "system", "content": system_prompt()}]
    for req, steps in FEWSHOT:
        msgs.append({"role": "user", "content": req})
        msgs.append({"role": "assistant", "content": plan_json(steps)})
    msgs.append({"role": "user", "content": request})
    return msgs


def build_grammar_text(max_steps=5):
    r = [
        'root ::= "{\\"steps\\":[" step ("," step){0,%d} "]}"' % (max_steps - 1),
        'str ::= "\\"" ([^"\\\\\\n] | "\\\\" ["\\\\])* "\\""',
    ]
    alts = []
    for i, (sid, slots, _, _) in enumerate(SKILLS):
        n = f"s{i}"
        alts.append(n)
        r.append(f'{n} ::= "{{\\"skill\\":\\"{sid}\\",\\"slots\\":{{" {n}h0 "}}}}"')
        # slots appear in catalog order, each optional, none repeated
        for j in range(len(slots) + 1):
            if j == len(slots):
                r.append(f'{n}h{j} ::= ""')
                r.append(f'{n}t{j} ::= ""')
            else:
                kv = f'"\\"{slots[j]}\\":" str'
                r.append(f'{n}h{j} ::= {n}h{j+1} | {kv} {n}t{j+1}')
                r.append(f'{n}t{j} ::= {n}t{j+1} | "," {kv} {n}t{j+1}')
    r.append("step ::= " + " | ".join(alts))
    return "\n".join(r)


_grammar_cache = {}


def get_grammar():
    from llama_cpp import LlamaGrammar
    if "g" not in _grammar_cache:
        _grammar_cache["g"] = LlamaGrammar.from_string(build_grammar_text(), verbose=False)
    return _grammar_cache["g"]


def load_model(path, n_threads=4, n_ctx=4096):
    from llama_cpp import Llama
    return Llama(model_path=path, n_ctx=n_ctx, n_threads=n_threads, n_batch=512,
                 verbose=False, seed=0)


def make_plan(llm, request, max_tokens=400):
    """Returns (plan_or_None, raw_text, stats)."""
    msgs = messages_for(request)
    t0 = time.perf_counter()
    first = None
    n_out = 0
    text = ""
    prompt_tokens = None
    for chunk in llm.create_chat_completion(messages=msgs, grammar=get_grammar(),
                                            temperature=0.0, max_tokens=max_tokens, stream=True):
        delta = chunk["choices"][0]["delta"].get("content")
        if delta:
            if first is None:
                first = time.perf_counter()
            text += delta
            n_out += 1
    t1 = time.perf_counter()
    first = first or t1
    decode_s = max(t1 - first, 1e-9)
    stats = {
        "seconds": t1 - t0,
        "prefill_seconds": first - t0,
        "out_tokens": n_out,
        # the first token is produced by prefill, so decode rate counts the rest
        "tok_per_sec": (n_out - 1) / decode_s if n_out > 1 else 0.0,
    }
    try:
        plan = json.loads(text)
        ok = isinstance(plan, dict) and isinstance(plan.get("steps"), list)
    except Exception:
        plan, ok = None, False
    stats["json_valid"] = ok
    return (plan if ok else None), text, stats


if __name__ == "__main__":
    import sys
    llm = load_model(sys.argv[1])
    for q in sys.argv[2:]:
        p, raw, st = make_plan(llm, q)
        print(raw, st)
