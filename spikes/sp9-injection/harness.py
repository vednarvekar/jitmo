"""SP9 harness: content/command separation (docs 06 'Security design', SR5).
Rules only, no models, no network.

Layers
 1. Data tagging: all email/message/web/file text becomes Data(untrusted=True); never parsed for commands.
 2. Command channel: only the user's typed request goes to route(); it alone yields the allowed skills.
 3. Sanitizer: web HTML -> visible text only (hidden/CSS-hidden/comment/zero-width removed).
 4. Plan guard: any action not selected by the user's request is rejected; args (recipient, path, url)
    must be grounded in the typed request or trusted metadata (sender, contacts), never in content.
 5. Flagger: injection heuristics on content (multilingual) -> user-visible warning (does not gate actions).
"""
import re, unicodedata
from dataclasses import dataclass, field
from html.parser import HTMLParser

# ---------- 1. data tagging ----------
@dataclass(frozen=True)
class Data:
    source: str          # email|whatsapp|web|file
    text: str
    untrusted: bool = True
    def marked(self):    # what a model would see
        return f"<untrusted_data source={self.source}>\n{self.text}\n</untrusted_data>"

# ---------- 2. router: ONLY the typed request selects skills ----------
ROUTES = [
    (r"\bforward\b", "email.forward"),
    (r"\breply\b|\brespond\b", "email.reply"),
    (r"\bsend\b.*\b(file|message|whatsapp)|\bwhatsapp\b.*\bsend\b|\bsend\b", "whatsapp.send"),
    (r"\bsummari[sz]e\b|\btl;?dr\b", "summarize"),
    (r"\bread\b.*\b(messages?|email|inbox)|\bnew messages\b|\bwhat did .* say\b", "messages.read"),
    (r"\bsearch\b|\blook up\b|\bfind out\b", "web.search"),
    (r"\bopen\b.*\bpage\b|\bread\b.*\bpage\b|\bwhat does .* say\b", "web.read"),
]
READ_ONLY = {"summarize", "messages.read", "web.search", "web.read"}
def route(user_request: str):
    u = user_request.lower()
    return {s for pat, s in ROUTES if re.search(pat, u)}

# ---------- 3. sanitizer ----------
INVISIBLE = re.compile("[​-‏‪-‮⁠-⁤⁦-⁩﻿­᠎\U000e0000-\U000e007f]")
VOID = {"br","hr","img","input","meta","link","area","base","col","embed","source","track","wbr"}
SKIP_TAGS = {"script","style","template","noscript","head","svg","iframe","object","canvas","select","option","textarea"}
BLOCK = {"p","div","li","tr","h1","h2","h3","h4","h5","h6","br","section","article","ul","ol","table"}
HIDE_DECL = [
    r"display\s*:\s*none", r"visibility\s*:\s*(hidden|collapse)", r"opacity\s*:\s*0(\.0+)?\s*(;|$)",
    r"font-size\s*:\s*0(px|pt|em|rem|%)?\s*(;|$)", r"font-size\s*:\s*[01](\.\d+)?px", r"color\s*:\s*transparent",
    r"text-indent\s*:\s*-\d{3,}", r"(left|top|margin-left|margin-top)\s*:\s*-\d{3,}",
    r"(height|width|max-height|max-width)\s*:\s*0(px)?\s*(;|!|$)", r"clip(-path)?\s*:\s*(rect\(0|inset\(50%|polygon\(0)",
    r"position\s*:\s*absolute[^\"]*(left|top)\s*:\s*-\d{3,}",
]
def _css_hidden(style: str, bg_ctx):
    s = style.lower().replace("!important", "")
    if any(re.search(p, s) for p in HIDE_DECL): return True
    m = re.search(r"(?<![-\w])color\s*:\s*([#\w(),. %]+)", s)
    bg = re.search(r"background(-color)?\s*:\s*([#\w(),. %]+)", s)
    col = _norm_color(m.group(1)) if m else None
    bgc = _norm_color(bg.group(2)) if bg else bg_ctx
    if col and bgc and _close(col, bgc): return True      # same / near-same colour as background
    return False
def _norm_color(c):
    c = c.strip().split(";")[0].strip()
    names = {"white":"#ffffff","black":"#000000"}
    c = names.get(c, c)
    m = re.fullmatch(r"#([0-9a-f]{3}|[0-9a-f]{6})", c)
    if m:
        h = m.group(1); h = "".join(ch*2 for ch in h) if len(h)==3 else h
        return tuple(int(h[i:i+2],16) for i in (0,2,4))
    m = re.fullmatch(r"rgba?\((\d+),\s*(\d+),\s*(\d+)", c)
    return tuple(map(int, m.groups())) if m else None
def _close(a, b): return sum(abs(x-y) for x,y in zip(a,b)) <= 24

class _Vis(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.stack=[]; self.out=[]; self.hidden_removed=[]; self.classrules={}; self.in_style=False; self.bg=(255,255,255)
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag in VOID:
            return
        hid = tag in SKIP_TAGS or "hidden" in a or a.get("aria-hidden","").lower()=="true" \
              or a.get("type","")=="hidden"
        st = a.get("style","")
        for c in (a.get("class") or "").split():
            st += ";" + self.classrules.get("."+c, "")
        if a.get("id"): st += ";" + self.classrules.get("#"+a["id"], "")
        bgm = re.search(r"background(?:-color)?\s*:\s*([#\w(),. %]+)", st.lower())
        if tag in ("body","html") and bgm: self.bg = _norm_color(bgm.group(1)) or self.bg
        if st and _css_hidden(st, self.bg): hid = True
        self.stack.append((tag, bool(hid or (self.stack and self.stack[-1][1]))))
        if tag in BLOCK: self.out.append("\n")
    def handle_endtag(self, tag):
        for i in range(len(self.stack)-1, -1, -1):
            if self.stack[i][0]==tag: del self.stack[i:]; break
        if tag in BLOCK: self.out.append("\n")
    def handle_data(self, d):
        if self.stack and self.stack[-1][0]=="style":   # collect simple class/id rules
            for sel, body in re.findall(r"([.#][\w-]+)\s*\{([^}]*)\}", d):
                self.classrules[sel] = self.classrules.get(sel,"")+";"+body
            return
        if self.stack and self.stack[-1][1]:
            if d.strip() and not any(t[0]=='head' for t in self.stack): self.hidden_removed.append(d.strip())
            return
        self.out.append(d)
    def handle_comment(self, d):   # dropped from model input, kept only for the user-facing flag
        if d.strip(): self.hidden_removed.append(d.strip())

def sanitize_html(html: str):
    # pre-pass: <style> may come after use; collect rules first
    p = _Vis()
    for sel, body in re.findall(r"([.#][\w-]+)\s*\{([^}]*)\}", " ".join(re.findall(r"<style[^>]*>(.*?)</style>", html, re.S|re.I))):
        p.classrules[sel] = p.classrules.get(sel,"")+";"+body
    p.feed(html); p.close()
    text = "".join(p.out)
    return clean_text(text), p.hidden_removed

def clean_text(t: str):
    t = unicodedata.normalize("NFKC", t)
    t = INVISIBLE.sub("", t)
    t = re.sub(r"[ \t\r\f\v]+", " ", t)
    return re.sub(r"\n\s*\n+", "\n", t).strip()

# ---------- 5. flagger (best effort; does NOT gate actions) ----------
FLAG_PATTERNS = [
  r"\b(ignore|disregard|forget|override)\b.{0,30}\b(previous|prior|above|earlier|all)\b.{0,20}\b(instructions?|rules|prompt)",
  r"\b(assistant|ai|chatbot|bot|claude|gpt|copilot|agent)\b\s*[:,\-]\s*\w",
  r"\bsystem\s*(prompt|message|override|note)\b|\[\s*system\s*\]|<\s*/?\s*(system|instructions?)\s*>",
  r"\b(forward|send|email|share|upload|transfer|delete|wipe|execute)\b.{0,60}\b(to)\b.{0,40}(@|https?://)",
  r"\b(send|share|forward)\b.{0,15}\b(me|us)\b.{0,40}\b(password|passwords|credential|file|files|invoice|invoices|keys?|token|otp|2fa|code)",
  r"\b(passwords?|credentials?|api[ _-]?keys?|otp|seed phrase)\b.{0,40}\b(send|forward|share|reply)",
  r"\bdo not (tell|inform|mention|show|alert) (the )?user\b|\bsilently\b|\bwithout (telling|asking|confirm)",
  r"\bwhen (the )?(assistant|ai|you) (reads?|summari[sz]es?|sees?)\b",
  # multilingual: ignore-previous / send-to / assistant-address
  r"ignora(r)? (todas )?(las )?instrucciones", r"envía(r)? .{0,40}(a|@)", r"asistente\s*[:,]",
  r"ignore[zr]? (toutes )?(les )?instructions", r"envoie[zr]? .{0,40}(à|@)", r"assistant\s*[:,]\s*\w",
  r"ignoriere? (alle )?(vorherigen )?anweisungen", r"sende?n? .{0,40}(an|@)", r"assistent\s*[:,]",
  r"忽略.{0,6}(之前|以上|先前).{0,6}(指令|说明)", r"(发送|转发).{0,20}(到|给).{0,40}@", r"助手\s*[:：,，]",
  r"игнорируй.{0,20}инструкци", r"(отправь|перешли).{0,40}@", r"ассистент\s*[:,]",
  r"पिछले निर्देश", r"असिस्टेंट\s*[:,]", r"(भेज|फॉरवर्ड).{0,40}@",
  r"تجاهل.{0,20}التعليمات", r"أرسل.{0,40}@",
]
def flag(text: str, hidden_removed=()):
    t = clean_text(text)
    hits = [p for p in FLAG_PATTERNS if re.search(p, t, re.I|re.S)]
    reasons = ["injection-pattern"] * bool(hits)
    if hidden_removed and any(re.search(p, " ".join(hidden_removed), re.I|re.S) for p in FLAG_PATTERNS):
        reasons.append("hidden-text-with-commands")
    elif hidden_removed and len(" ".join(hidden_removed)) > 40:
        reasons.append("hidden-text")
    if INVISIBLE.search(text) and (len(INVISIBLE.findall(text)) >= 8): reasons.append("invisible-chars")
    return reasons

# ---------- 4. plan guard ----------
@dataclass
class Action:
    skill: str
    args: dict = field(default_factory=dict)
    origin: str = "plan"      # as proposed by the (possibly compromised) planner

ARG_TOKEN = re.compile(r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+|https?://\S+|\+?\d[\d ()-]{7,}\d|[\w .\\/-]+\.(?:xlsx?|docx?|pdf|txt|csv|zip|kdbx|json|pem)\b", re.I)
def guard(action: Action, allowed: set, user_request: str, trusted_ctx: set):
    """-> (ok, reason). Plain code; no model can override."""
    if action.skill not in allowed:
        return False, f"skill '{action.skill}' not selected by user request"
    grounding = user_request.lower() + " " + " ".join(trusted_ctx).lower()
    for k, v in action.args.items():
        for tok in ARG_TOKEN.findall(str(v)) or ([str(v)] if k in ("to","recipient","path","file","url","contact") else []):
            if tok.lower().strip() not in grounding:
                return False, f"arg {k}={tok!r} not grounded in user request/trusted context (came from content)"
    return True, "ok"

# ---------- pipeline ----------
def run(fx, use_guard=True, use_sanitizer=True):
    """fx: dict(user, channel, content, trusted_ctx, attack)"""
    allowed = route(fx["user"])
    raw = fx["content"]
    if fx["channel"] == "web" and use_sanitizer:
        vis, hidden = sanitize_html(raw)
    else:
        vis, hidden = clean_text(raw), []
    data = Data(fx["channel"], vis)
    flags = flag(vis if fx["channel"]!="web" else vis, hidden)
    if fx["channel"] == "web" and not hidden:
        flags = flag(vis)
    executed, rejected = [], []
    # the user's own command (derived from the typed request only)
    for s in sorted(allowed):
        args = fx.get("user_args", {}).get(s, {})
        executed.append(Action(s, args, "user"))
    # worst-case planner: a model that obeys the injected instruction (it sees `data.marked()`)
    proposals = [Action(**fx["attack"], origin="content")] if fx.get("attack") else []
    for a in proposals:
        ok, why = guard(a, allowed, fx["user"], set(fx.get("trusted_ctx", []))) if use_guard else (True, "guard off")
        (executed if ok else rejected).append((a, why) if not ok else a)
    return dict(allowed=allowed, visible=vis, hidden_removed=hidden, flags=flags, executed=executed, rejected=rejected)
