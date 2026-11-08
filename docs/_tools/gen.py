"""Doc page generator: builds docs/src/<path>.md from src/<path>.rs plus prose.

Usually run through refresh.py, which harvests the prose from the existing page first.
Direct use (paths relative to the repo root):

  python docs/_tools/gen.py keys   src/x.rs   every documentable key (fns, types, tests)
  python docs/_tools/gen.py digest src/x.rs   signatures + doc comments, for writing prose
  python docs/_tools/gen.py gaps   src/x.rs   digest of only what the page lacks prose for
  python docs/_tools/gen.py build  src/x.rs   write the page from source + prose files

Headings, source links, signatures, field/variant tables and test tables come from the
source; only the explanations come from prose. Prose files: $DOCS_PROSE (default
target/docs-prose)/<src path>*.md, merged in name order (later files win).
Directives (each runs until the next line starting with @@):
  @@title <text>
  @@intro            markdown: What this file is for / How it connects / Rust concepts
  @@type <Name>      prose for a struct/enum/trait/const/static/type alias/macro
  @@fields <Name>    lines "field: meaning" (override field doc comments)
  @@variants <Name>  lines "Variant: meaning"
  @@group <text>     optional intro under an impl/trait group heading (text = group title)
  @@fn <key>         prose for a function (key from `keys`); "=doc" uses its doc comment
  @@test <name>      one line for a test or test helper
  @@tests            markdown placed at the top of the Tests section
"""
import bisect
import os
import re
import subprocess
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))
# Prose files are scratch: refresh.py harvests them from the pages, builds, and throws them away.
PROSE_DIR = os.environ.get("DOCS_PROSE") or os.path.join(ROOT, "target", "docs-prose")


# ---------------------------------------------------------------- scanning

def code_only(src):
    """Same length as src; comments and string/char contents blanked (newlines kept)."""
    out = list(src)
    n = len(src)
    i = 0

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
            continue
        if c in "rb" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            m = re.match(r'b?r(#*)"', src[i:i + 12])
            if m:
                hashes = m.group(1)
                start = i + m.end()
                end = src.find('"' + hashes, start)
                end = n if end < 0 else end
                blank(start, end)
                i = end + 1 + len(hashes)
                continue
        if c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(i + 1, min(j, n))
            i = j + 1
            continue
        if c == "'":
            if i + 1 < n and src[i + 1] == "\\":
                j = i + 3
                while j < n and src[j] != "'":
                    j += 1
            elif i + 2 < n and src[i + 2] == "'":
                j = i + 2
            else:
                i += 1  # lifetime
                continue
            blank(i + 1, j)
            i = j + 1
            continue
        i += 1
    return "".join(out)


ITEM_RE = re.compile(
    r'^(?:\s|#!?\[[^\]]*\]\s*)*'
    r'(?P<vis>pub(?:\s*\([^)]*\))?\s+)?'
    r'(?P<quals>(?:(?:const|unsafe|async|default|extern\s*(?:"[^"]*")?)\s+)*)'
    r'(?P<kind>(?:fn|struct|enum|trait|mod|union|impl)\b|macro_rules!)\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)?'
)
IMPL_RE = re.compile(r'^(?:\s|#!?\[[^\]]*\]\s*)*(?:unsafe\s+)?impl\b')
CONST_RE = re.compile(
    r'^(?:\s|#!?\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?(?P<kind>const|static)\s+(?:mut\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*:'
)
TYPE_RE = re.compile(
    r'^(?:\s|#!?\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?type\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)'
)


class Item:
    def __init__(self, **kw):
        self.__dict__.update(kw)


class Parsed:
    pass


def parse(path):
    src = open(path, encoding="utf-8").read()
    code = code_only(src)
    lines = src.split("\n")
    nl = [i for i, ch in enumerate(src) if ch == "\n"]

    def line_of(idx):
        return bisect.bisect_left(nl, idx) + 1  # 1-based

    def col_of(idx):
        ln = line_of(idx)
        start = nl[ln - 2] + 1 if ln >= 2 else 0
        return idx - start

    items = []
    stack = []
    pending = 0
    paren = 0
    bracket = 0
    after_const = None

    def ancestors():
        return [f for f in stack if f["kind"] != "block"]

    def item_level():
        a = [f for f in stack]
        return (not a) or a[-1]["kind"] in ("mod", "impl", "trait")

    def header_info(h):
        m = ITEM_RE.match(h)
        if m and m.group("kind") != "impl":
            return m
        return None

    def kw_line(h, h_start, word):
        m = re.search(r'\b' + re.escape(word) + r'\b', h)
        off = m.start() if m else len(h) - len(h.lstrip())
        return line_of(h_start + off)

    i = 0
    n = len(code)
    while i < n:
        ch = code[i]
        if ch == "(":
            paren += 1
        elif ch == ")":
            paren -= 1
        elif ch == "[":
            bracket += 1
        elif ch == "]":
            bracket -= 1
        elif ch == "{":
            h = code[pending:i]
            frame = {"kind": "block", "name": None, "paren": paren, "bracket": bracket}
            # `impl` opens a statement wherever it appears, a fn body included
            if IMPL_RE.match(h):
                frame["kind"] = "impl"
                frame["header"] = " ".join(src[pending:i].split()) if False else None
                raw = code[pending:i]
                raw = re.sub(r'#!?\[[^\]]*\]', "", raw)
                raw = " ".join(raw.split())
                raw = re.sub(r'\s+where\b.*$', "", raw)
                frame["title"] = raw
                bare = raw.replace("->", " RET ")
                while re.search(r'<[^<>]*>', bare):
                    bare = re.sub(r'<[^<>]*>', '', bare)
                tm = re.search(r'\bfor\s+([A-Za-z_][\w:]*)', bare)
                if not tm:
                    tm = re.search(r'impl\s+(?:[\w:]+::)?([A-Za-z_]\w*)', bare)
                tname = tm.group(1).split("::")[-1] if tm else "?"
                frame["name"] = tname
                frame["line"] = kw_line(h, pending, "impl")
                trm = re.search(r'impl\s+([\w:]+)\s+for\b', bare)
                frame["trait"] = trm.group(1).split("::")[-1] if trm else None
            else:
                m = header_info(h)
                if m:
                    kind = m.group("kind")
                    name = m.group("name")
                    frame["kind"] = {"macro_rules!": "macro"}.get(kind, kind)
                    frame["name"] = name
                    frame["line"] = kw_line(h, pending, "macro_rules" if kind == "macro_rules!" else kind)
                    frame["test"] = bool(re.search(r'#\[\s*test\s*\]', h))
                    frame["cfg_test"] = bool(re.search(r'#\[\s*cfg\s*\(\s*test\s*\)\s*\]', h))
                    frame["header_start"] = pending
                    frame["body_open"] = i
                    frame["item_level"] = item_level() or (ancestors() and ancestors()[-1]["kind"] == "fn")
                elif item_level():
                    cm = CONST_RE.match(h)
                    if cm:
                        it = Item(kind=cm.group("kind"), name=cm.group("name"),
                                  line=kw_line(h, pending, cm.group("kind")), end=None,
                                  parents=[dict(f) for f in ancestors()])
                        items.append(it)
                        frame["const_item"] = it
            frame["anc"] = [dict(f) for f in ancestors()]
            stack.append(frame)
            paren, bracket = 0, 0
            pending = i + 1
        elif ch == "}":
            if stack:
                f = stack.pop()
                paren, bracket = f["paren"], f["bracket"]
                f["end"] = line_of(i)
                if f["kind"] in ("fn", "struct", "enum", "trait", "mod", "union", "impl", "macro"):
                    record_frame(f, items, src, code, lines, line_of, col_of, i)
                if f.get("const_item") is not None:
                    after_const = f["const_item"]
            pending = i + 1
        elif ch == ";" and paren == 0 and bracket == 0:
            h = code[pending:i]
            if after_const is not None:
                after_const.end = line_of(i)
                after_const = None
            elif item_level() or (stack and stack[-1]["kind"] == "block" and False):
                m = header_info(h)
                if m and m.group("kind") == "fn":
                    it = Item(kind="fn", name=m.group("name"), line=kw_line(h, pending, "fn"),
                              end=line_of(i), body=False,
                              sig_end=i, test=False, parents=[dict(f) for f in ancestors()])
                    it.sig = signature(src, lines, it.line, i, col_of)
                    items.append(it)
                elif m and m.group("kind") in ("struct", "union"):
                    it = Item(kind="struct", name=m.group("name"), line=kw_line(h, pending, m.group("kind")),
                              end=line_of(i), fields=tuple_fields(src[pending:i]), tuple=True,
                              parents=[dict(f) for f in ancestors()])
                    items.append(it)
                elif m and m.group("kind") == "mod":
                    pass
                else:
                    cm = CONST_RE.match(h)
                    if cm:
                        items.append(Item(kind=cm.group("kind"), name=cm.group("name"),
                                          line=kw_line(h, pending, cm.group("kind")), end=line_of(i),
                                          parents=[dict(f) for f in ancestors()]))
                    else:
                        tm = TYPE_RE.match(h)
                        if tm:
                            items.append(Item(kind="type", name=tm.group("name"),
                                              line=kw_line(h, pending, "type"), end=line_of(i),
                                              parents=[dict(f) for f in ancestors()]))
            elif stack and stack[-1]["kind"] == "block":
                # an extern block's declarations
                m = header_info(h)
                if m and m.group("kind") == "fn" and stack[-1].get("anc") is not None and not any(
                        f["kind"] == "fn" for f in ancestors()):
                    it = Item(kind="fn", name=m.group("name"), line=kw_line(h, pending, "fn"),
                              end=line_of(i), body=False, test=False,
                              parents=[dict(f) for f in ancestors()])
                    it.sig = signature(src, lines, it.line, i, col_of)
                    items.append(it)
            pending = i + 1
        i += 1

    items.sort(key=lambda it: it.line)
    p = Parsed()
    p.src, p.lines, p.items = src, lines, items
    return p


def signature(src, lines, fn_line, end_idx, col_of):
    end_line = src.count("\n", 0, end_idx) + 1
    first = lines[fn_line - 1]
    indent = len(first) - len(first.lstrip())
    out = []
    for ln in range(fn_line, end_line + 1):
        text = lines[ln - 1]
        if ln == end_line:
            text = text[:col_of(end_idx)]
        out.append(text[indent:] if text[:indent].strip() == "" else text.strip())
    sig = "\n".join(l.rstrip() for l in out).strip()
    return sig


def tuple_fields(text):
    m = re.search(r'\((.*)\)', text, re.S)
    if not m:
        return []
    return [{"name": str(k), "type": " ".join(t.split()), "doc": ""} for k, t in enumerate(split_top(m.group(1))) if t.strip()]


def split_top(s):
    parts, depth, cur = [], 0, ""
    for ch in s:
        if ch in "([{<":
            depth += 1
        elif ch in ")]}>":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    parts.append(cur)
    return [p.strip() for p in parts]


def doc_above(lines, line):
    """`///` comment text directly above 1-based `line` (skipping attributes)."""
    out = []
    k = line - 2
    while k >= 0:
        s = lines[k].strip()
        if s.startswith("///"):
            out.append(s[3:][1:] if s[3:].startswith(" ") else s[3:])
        elif s.startswith("#[") or s.startswith("#!["):
            pass
        elif s.endswith(")]") and not s.startswith("//"):
            pass
        else:
            break
        k -= 1
    out.reverse()
    return "\n".join(out).strip("\n")


def body_lines(lines, start, end, depth_code):
    pass


def record_frame(f, items, src, code, lines, line_of, col_of, close_idx):
    kind = f["kind"]
    if kind == "fn":
        it = Item(kind="fn", name=f["name"], line=f["line"], end=f["end"], body=True,
                  test=f.get("test", False), parents=f["anc"])
        it.sig = signature(src, lines, f["line"], f["body_open"], col_of)
        items.append(it)
    elif kind in ("struct", "union", "enum"):
        body_start = line_of(f["body_open"])
        it = Item(kind="enum" if kind == "enum" else "struct", name=f["name"], line=f["line"], end=f["end"],
                  parents=f["anc"], tuple=False)
        it.fields = members(src, code, lines, f["body_open"], close_idx, line_of, enum=(kind == "enum"))
        items.append(it)
    elif kind == "trait":
        items.append(Item(kind="trait", name=f["name"], line=f["line"], end=f["end"], parents=f["anc"]))
    elif kind == "macro":
        items.append(Item(kind="macro", name=f["name"], line=f["line"], end=f["end"], parents=f["anc"]))
    elif kind == "mod":
        items.append(Item(kind="mod", name=f["name"], line=f["line"], end=f["end"], parents=f["anc"],
                          cfg_test=f.get("cfg_test", False)))
    elif kind == "impl":
        items.append(Item(kind="impl", name=f["name"], line=f["line"], end=f["end"], parents=f["anc"],
                          title=f["title"], trait=f.get("trait")))


def members(src, code, lines, open_idx, close_idx, line_of, enum):
    body_code = code[open_idx + 1:close_idx]
    body_src = src[open_idx + 1:close_idx]
    first_line = line_of(open_idx)
    # split into top-level segments by commas at depth 0
    segs = []
    depth = 0
    start = 0
    for k, ch in enumerate(body_code):
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "<":
            depth += 1
        elif ch == ">" and k > 0 and body_code[k - 1] != "-":
            depth -= 1
        elif ch == "," and depth == 0:
            segs.append((start, k))
            start = k + 1
    segs.append((start, len(body_code)))
    out = []
    for a, b in segs:
        seg_code = body_code[a:b]
        seg_src = body_src[a:b]
        stripped = seg_code.strip()
        if not stripped:
            continue
        lead = len(seg_code) - len(seg_code.lstrip())
        # skip attributes at the start of the segment
        m_attr = re.match(r'(\s|#\[[^\]]*\])*', seg_code)
        off = m_attr.end() if m_attr else lead
        decl_line = line_of(open_idx + 1 + a + off)
        text = " ".join(seg_code[off:].split())
        text_src = " ".join(seg_src[off:].split())
        doc = doc_above(lines, decl_line)
        if enum:
            nm = re.match(r'([A-Za-z_]\w*)', text)
            if not nm:
                continue
            disp = text_src
            if len(disp) > 90:
                inner = re.findall(r'(?:[{,(]\s*)([a-z_]\w*)\s*:', disp)
                disp = nm.group(1) + (" { " + ", ".join(inner) + " }" if "{" in disp else "(…)")
            out.append({"name": nm.group(1), "type": disp, "doc": doc, "line": decl_line})
        else:
            fm = re.match(r'(?:pub(?:\s*\([^)]*\))?\s+)?([A-Za-z_]\w*)\s*:(.*)$', text_src, re.S)
            if not fm:
                continue
            out.append({"name": fm.group(1), "type": fm.group(2).strip(), "doc": doc, "line": decl_line})
    return out


# ---------------------------------------------------------------- keys

def is_test_mod(m):
    return m["kind"] == "mod" and (m.get("cfg_test") or m["name"] in ("tests",) or m["name"].endswith("_tests"))


def classify(p):
    """Attach key, group, level, role to every item."""
    seen = {}
    for it in p.items:
        anc = [a for a in it.parents if a["kind"] != "block"]
        tmods = [a for a in anc if is_test_mod(a)]
        it.test_mod = tmods[-1]["name"] if tmods else None
        named = [a for a in anc if a["kind"] in ("impl", "trait", "fn", "mod")]
        prefix = []
        for a in named:
            if a["kind"] == "mod" and is_test_mod(a):
                continue
            if a["kind"] == "fn":
                prefix.append(a["name"])
            elif a["kind"] in ("impl", "trait"):
                prefix = [a["name"]]
            elif a["kind"] == "mod":
                prefix.append(a["name"])
        it.nested = any(a["kind"] == "fn" for a in anc)
        grp = None
        for a in reversed(named):
            if a["kind"] in ("impl", "trait"):
                grp = a
                break
            if a["kind"] == "mod" and not is_test_mod(a):
                grp = a
                break
        it.group = grp
        if it.kind == "fn":
            if it.test_mod is not None or getattr(it, "test", False):
                it.role = "test" if it.test else "helper"
            else:
                it.role = "fn"
            base = "::".join(prefix + [it.name]) if it.role == "fn" else it.name
        else:
            it.role = "type"
            base = it.name
        k = (it.role if it.role != "helper" else "test", base)
        seen[k] = seen.get(k, 0) + 1
        it.key = base if seen[k] == 1 else f"{base}#{seen[k]}"


# ---------------------------------------------------------------- prose

def load_prose(rel):
    prose = {"title": "", "intro": "", "type": {}, "fields": {}, "variants": {}, "group": {},
             "fn": {}, "test": {}, "tests": ""}
    base = rel.replace("/", os.sep)
    d = os.path.join(PROSE_DIR, os.path.dirname(base))
    stem = os.path.basename(base)
    if not os.path.isdir(d):
        return prose
    files = sorted(f for f in os.listdir(d) if f.startswith(stem) and f.endswith(".md"))
    for f in files:
        cur = None
        buf = []

        def flush():
            if cur is None:
                return
            kind, arg = cur
            text = "\n".join(buf).strip("\n")
            if kind in ("title",):
                prose["title"] = arg
            elif kind in ("intro", "tests"):
                prose[kind] = (prose[kind] + "\n\n" + text).strip("\n") if prose[kind] else text
            elif kind in ("fields", "variants"):
                tbl = prose[kind].setdefault(arg, {})
                for line in text.split("\n"):
                    if ":" in line:
                        k, v = line.split(":", 1)
                        tbl[k.strip().strip("`")] = v.strip()
            elif kind in prose:
                prose[kind][arg] = text

        for line in open(os.path.join(d, f), encoding="utf-8").read().split("\n"):
            m = re.match(r'^@@(\w+)\s*(.*)$', line)
            if m:
                flush()
                cur = (m.group(1), m.group(2).strip())
                buf = []
            else:
                buf.append(line)
        flush()
    return prose


def clean_doc(doc):
    doc = re.sub(r'\[`([^`]+)`\]\([^)]*\)', r'`\1`', doc)
    doc = re.sub(r'\[`([^`]+)`\]', r'`\1`', doc)
    doc = re.sub(r'\[([^\]`]+)\]\((?:crate|self|super)::[^)]*\)', r'\1', doc)
    # join wrapped lines into paragraphs, keep fenced blocks and list items
    out, para, fence = [], [], False
    for line in doc.split("\n"):
        if line.strip().startswith("```"):
            if para:
                out.append(" ".join(para))
                para = []
            fence = not fence
            out.append(line.replace("```text", "```text").replace("```ignore", "```rust").replace("```no_run", "```rust"))
            continue
        if fence:
            out.append(line)
            continue
        if not line.strip():
            if para:
                out.append(" ".join(para))
                para = []
            out.append("")
            continue
        if re.match(r'^\s*([-*]|\d+\.)\s', line) and para:
            out.append(" ".join(para))
            para = []
        para.append(line.strip())
    if para:
        out.append(" ".join(para))
    text = "\n".join(out)
    return re.sub(r'\n{3,}', "\n\n", text).strip()


def humanize(name):
    s = name.replace("_", " ").strip()
    return s[:1].upper() + s[1:] + "."


def cell(s):
    return s.replace("|", "\\|").replace("\n", " ")


def resolve_keys(p, prose):
    """Map loosely-named prose keys (from harvesting) onto real item keys."""
    fn_keys = [it.key for it in p.items if it.kind == "fn" and it.role == "fn"]
    type_names = {it.name for it in p.items if it.role == "type"}
    test_names = {it.name for it in p.items if it.kind == "fn" and it.role in ("test", "helper")}
    for k in list(prose["fn"].keys()):
        if k in fn_keys:
            continue
        last = k.split("::")[-1]
        cands = [fk for fk in fn_keys if fk.split("#")[0].split("::")[-1] == last]
        if len(cands) == 1 and cands[0] not in prose["fn"]:
            prose["fn"][cands[0]] = prose["fn"].pop(k)
        elif len(cands) > 1 and k.count("::"):
            suff = [fk for fk in cands if fk.split("#")[0].endswith(k)]
            if len(suff) == 1 and suff[0] not in prose["fn"]:
                prose["fn"][suff[0]] = prose["fn"].pop(k)
        elif last in type_names and last not in prose["type"]:
            prose["type"][last] = prose["fn"].pop(k)
        elif last in test_names and last not in prose["test"]:
            prose["test"][last] = prose["fn"].pop(k)


def digest(rel, keys=None):
    p = parse(os.path.join(ROOT, rel))
    classify(p)
    out = []
    if keys is None:
        head = []
        for line in p.lines:
            s = line.strip()
            if s.startswith("//!"):
                head.append(s[3:].strip())
            elif s and not s.startswith("//"):
                break
        if head:
            out.append("//! " + " ".join(head))
    for it in p.items:
        if keys is not None and it.key not in keys and it.name not in keys:
            continue
        if it.kind in ("impl", "mod"):
            if keys is None:
                out.append(f"== {it.line} {getattr(it, 'title', 'mod ' + it.name)}")
            continue
        doc = " ".join(doc_above(p.lines, it.line).split())
        size = (it.end or it.line) - it.line + 1
        if it.kind == "fn":
            if it.role == "test":
                out.append(f"T {it.line} {it.name}" + (f" — {doc[:160]}" if doc else ""))
                continue
            sig = " ".join(it.sig.split())
            out.append(f"\n@ {it.line} {it.key} [{size}L] {sig}")
            if doc:
                out.append(f"  /// {doc}")
            if size <= 10 or (not doc and size <= 30):
                body = p.lines[it.line:it.end]
                out.append("\n".join("  | " + l.strip() for l in body if l.strip()))
        else:
            out.append(f"\n# {it.line} {it.kind} {it.name} [{size}L]" + (f" /// {doc}" if doc else ""))
            if it.kind in ("const", "static", "type"):
                out.append("  | " + " ".join(p.lines[it.line - 1].split()))
            for f in getattr(it, "fields", []) or []:
                fd = " ".join(f["doc"].split())
                out.append(f"  - {f['name']}: {f['type'][:70]}" + (f" /// {fd}" if fd else ""))
    print("\n".join(out))


# ---------------------------------------------------------------- build

def commit():
    try:
        return subprocess.check_output(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:
        return "HEAD"


def module_path(rel):
    p = rel[len("src/"):-len(".rs")]
    if p == "main":
        return "crate"
    if p.endswith("/mod"):
        p = p[: -len("/mod")]
    return "crate::" + p.replace("/", "::")


def build(rel, write=True):
    path = os.path.join(ROOT, rel)
    p = parse(path)
    classify(p)
    prose = load_prose(rel)
    resolve_keys(p, prose)
    doc_rel = "docs/" + rel[:-3] + ".md"
    depth = doc_rel.count("/")
    up = "../" * depth
    docs_up = "../" * (depth - 1)
    link = lambda line: f"[{rel}:{line}]({up}{rel}#L{line})"
    used = set()
    problems = []
    fallback = []

    def get(kind, key):
        tbl = prose[kind]
        if key in tbl:
            used.add((kind, key))
            return tbl[key]
        return None

    out = []
    title = prose["title"] or "TITLE"
    out.append(f"# `{rel}` — {title}\n")
    out.append(f"> **Source:** [{rel}]({up}{rel}) · **Module:** `{module_path(rel)}` · "
               f"**Size:** {len(p.lines)} lines · Line links as of commit `{commit()}`\n")
    intro = prose["intro"] or "## What this file is for\n\nMISSING INTRO\n"
    out.append(intro.strip() + "\n")
    if not prose["intro"]:
        problems.append("intro")

    types = [it for it in p.items if it.role == "type" and it.kind not in ("impl", "mod")
             and not it.test_mod]
    fns = [it for it in p.items if it.kind == "fn" and it.role == "fn"]
    tests = [it for it in p.items if it.kind == "fn" and it.role in ("test", "helper")]
    test_types = [it for it in p.items if it.role == "type" and it.kind not in ("impl", "mod") and it.test_mod]
    impls = {it.line: it for it in p.items if it.kind == "impl"}

    def type_heading(it):
        kw = {"struct": "struct", "enum": "enum", "trait": "trait", "const": "const", "static": "static",
              "type": "type", "macro": "macro_rules!"}[it.kind]
        owner = it.group["name"] + "::" if it.group and it.kind in ("const", "static", "type") else ""
        return f"`{kw} {owner}{it.name}`"

    def emit_type(it, level="###"):
        out.append(f"{level} {type_heading(it)}\n{link(it.line)}\n")
        if it.kind in ("const", "static", "type"):
            end = it.end or it.line
            body = p.lines[it.line - 1:min(end, it.line + 5)]
            ind = len(body[0]) - len(body[0].lstrip())
            text = "\n".join(l[ind:] if l[:ind].strip() == "" else l.strip() for l in body)
            if end > it.line + 5:
                text += "\n// …"
            out.append("```rust\n" + text.rstrip() + "\n```\n")
        txt = get("type", it.name)
        doc = clean_doc(doc_above(p.lines, it.line))
        if txt:
            out.append(txt.strip() + "\n")
        elif doc:
            out.append(doc + "\n")
        else:
            problems.append(f"type {it.name}")
            out.append("MISSING\n")
        flds = getattr(it, "fields", None)
        if flds:
            if it.kind == "struct":
                over = prose["fields"].get(it.name, {})
                if over:
                    used.add(("fields", it.name))
            else:
                over = {**prose["variants"].get(it.name, {}), **prose["fields"].get(it.name, {})}
                for kk in ("variants", "fields"):
                    if it.name in prose[kk]:
                        used.add((kk, it.name))
            if it.kind == "enum":
                out.append("| Variant | Meaning |\n|---|---|")
                for f in flds:
                    meaning = over.get(f["name"]) or clean_doc(f["doc"]) or ""
                    if not meaning:
                        problems.append(f"variant {it.name}::{f['name']}")
                    out.append(f"| `{cell(f['type'])}` | {cell(meaning)} |")
            else:
                out.append("| Field | Type | Meaning |\n|---|---|---|")
                for f in flds:
                    meaning = over.get(f["name"]) or clean_doc(f["doc"]) or ""
                    if not meaning and not getattr(it, "tuple", False):
                        problems.append(f"field {it.name}.{f['name']}")
                    out.append(f"| `{cell(f['name'])}` | `{cell(f['type'])}` | {cell(meaning)} |")
            out.append("")

    if types:
        out.append("## Types\n")
        for it in types:
            emit_type(it)

    # functions, grouped in source order
    if fns:
        cur = "__start__"
        for it in fns:
            g = it.group
            gkey = (g["kind"], g["line"]) if g else None
            if gkey != cur:
                cur = gkey
                if g is None:
                    out.append("## Free functions\n")
                else:
                    if g["kind"] == "impl":
                        title_txt = g.get("title") or f"impl {g['name']}"
                    elif g["kind"] == "trait":
                        title_txt = f"trait {g['name']}"
                    else:
                        title_txt = f"mod {g['name']}"
                    out.append(f"## `{title_txt}`\n")
                    gi = get("group", title_txt)
                    if gi:
                        out.append(gi.strip() + "\n")
            level = "####" if it.nested else "###"
            disp = it.key.split("#")[0]
            out.append(f"{level} `{disp}`\n{link(it.line)}\n")
            out.append("```rust\n" + it.sig + "\n```\n")
            txt = get("fn", it.key)
            doc = clean_doc(doc_above(p.lines, it.line))
            if txt and txt.strip() == "=doc":
                if doc:
                    out.append(doc + "\n")
                else:
                    problems.append(f"fn {it.key} (=doc but no doc comment)")
                    out.append("MISSING\n")
            elif txt:
                out.append(txt.strip() + "\n")
            elif doc:
                fallback.append(it.key)
                out.append(doc + "\n")
            else:
                problems.append(f"fn {it.key}")
                out.append("MISSING\n")

    if tests or test_types:
        out.append("## Tests\n")
        mp = module_path(rel)
        if prose["tests"]:
            out.append(prose["tests"].strip() + "\n")
        mods = []
        for it in tests:
            if it.test_mod not in mods:
                mods.append(it.test_mod)
        for m in mods:
            filt = (mp[len("crate::"):] if mp != "crate" else "") + (("::" + m) if m else "")
            filt = filt.strip(":") or "."
            ts = [t for t in tests if t.test_mod == m and t.role == "test"]
            hs = [t for t in tests if t.test_mod == m and t.role == "helper"]
            title_m = f"`mod {m}`" if m else "Tests in this file"
            out.append(f"### {title_m}\n")
            out.append(f"Run with `cargo test {filt}` · {len(ts)} tests\n")
            if ts:
                out.append("| Test | What it checks |\n|---|---|")
                for t in ts:
                    txt = get("test", t.key) or get("test", t.name)
                    if not txt:
                        d = clean_doc(doc_above(p.lines, t.line))
                        txt = d if d else humanize(t.name)
                    out.append(f"| [`{t.name}`]({up}{rel}#L{t.line}) | {cell(txt)} |")
                out.append("")
            if hs:
                out.append("**Test helpers** (used only by the tests above):\n")
                for h in hs:
                    txt = get("test", h.key) or get("test", h.name) or get("fn", h.name)
                    if not txt:
                        d = clean_doc(doc_above(p.lines, h.line))
                        txt = d.split("\n\n")[0] if d else humanize(h.name)
                    disp = h.key.split("#")[0]
                    out.append(f"- [`{disp}`]({up}{rel}#L{h.line}) — {cell(txt)}")
                out.append("")
            tt = [t for t in test_types if t.test_mod == m]
            if tt:
                out.append("**Test-only types and constants:** " + ", ".join(
                    f"[`{t.name}`]({up}{rel}#L{t.line})" for t in tt) + "\n")

    text = "\n".join(out).rstrip() + "\n"
    text = text.replace("](DOCS/", "](" + docs_up)
    unknown = []
    for kind in ("type", "fn", "test", "fields", "variants", "group"):
        for k in prose[kind]:
            if (kind, k) not in used:
                unknown.append(f"{kind} {k}")
    if write:
        dest = os.path.join(ROOT, doc_rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        open(dest, "w", encoding="utf-8", newline="\n").write(text)
    return p, problems, fallback, unknown, doc_rel


def cmd_keys(rel):
    p = parse(os.path.join(ROOT, rel))
    classify(p)
    for it in p.items:
        if it.kind in ("impl", "mod"):
            print(f"{it.line:>6} [{it.kind}] {getattr(it, 'title', it.name)}")
            continue
        has_doc = "d" if doc_above(p.lines, it.line) else " "
        size = (it.end or it.line) - it.line + 1
        role = it.role
        extra = ""
        if it.kind in ("struct", "enum") and getattr(it, "fields", None):
            missing = [f["name"] for f in it.fields if not f["doc"]]
            extra = f" fields={len(it.fields)} undoc={','.join(missing)}" if missing else f" fields={len(it.fields)}"
        print(f"{it.line:>6} {role:<6} {has_doc} {size:>4}L {it.key}{extra}")


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    mode, rel = sys.argv[1], sys.argv[2].replace("\\", "/")
    if mode == "keys":
        cmd_keys(rel)
    elif mode == "digest":
        digest(rel)
    elif mode == "gaps":
        # digest of only the items the page still lacks prose for
        p, problems, fallback, unknown, doc_rel = build(rel, write=False)
        want = set(fallback)
        for pr in problems:
            parts = pr.split(" ", 1)
            if len(parts) == 2 and parts[0] in ("fn", "type"):
                want.add(parts[1].split(" ")[0])
        print("problems:", "; ".join(problems))
        digest(rel, want)
    elif mode == "build":
        p, problems, fallback, unknown, doc_rel = build(rel)
        print(f"wrote {doc_rel}")
        print(f"problems ({len(problems)}):", "; ".join(problems[:400]))
        print(f"doc-fallback fns ({len(fallback)}):", ", ".join(fallback[:80]))
        if unknown:
            print(f"unused prose keys ({len(unknown)}):", "; ".join(unknown[:60]))
