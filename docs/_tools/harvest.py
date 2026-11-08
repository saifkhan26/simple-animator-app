"""Turn an existing doc page back into prose directives for gen.py.

  python docs/_tools/harvest.py src/x.rs [page.md ...]

With no page given, reads docs/src/x.md. Writes $DOCS_PROSE/<src path>.0-harvest.md.

Everything that is not generated from the source is kept: the title, the intro sections,
each entry's explanation, field/variant meanings, test descriptions, the Tests intro and
any text under an impl heading. Line links into src/ are moved from the commit named in
the page header to HEAD (via `git diff`), so explanations that point at a line keep
pointing at the same code.
"""
import os
import re
import subprocess
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))
PROSE_DIR = os.environ.get("DOCS_PROSE") or os.path.join(ROOT, "target", "docs-prose")

TYPE_KW = ("struct", "enum", "const", "static", "type", "trait", "macro_rules!", "union")
_maps = {}


def head_commit():
    try:
        return subprocess.check_output(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:
        return None


def line_map(old, rel):
    """old line number -> HEAD line number for src file `rel`, from `git diff old HEAD`."""
    key = (old, rel)
    if key in _maps:
        return _maps[key]
    hunks = []
    try:
        diff = subprocess.check_output(["git", "-C", ROOT, "diff", "-U0", old, "HEAD", "--", rel], text=True,
                                       encoding="utf-8", errors="replace", stderr=subprocess.DEVNULL)
        for m in re.finditer(r'^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', diff, re.M):
            hunks.append((int(m.group(1)), int(m.group(2) or 1), int(m.group(3)), int(m.group(4) or 1)))
    except Exception:
        pass

    def f(x):
        off = 0
        for a, b, c, d in hunks:
            if b == 0:  # d lines inserted after old line a
                if x > a:
                    off += d
                    continue
                break
            if x < a:
                break
            if x < a + b:  # inside a changed stretch
                return c + min(x - a, max(d - 1, 0))
            off += d - b
        return x + off

    _maps[key] = f
    return f


def remap_links(text, old):
    if not old:
        return text

    def rep(m):
        label, path, num = m.group(1), m.group(2), int(m.group(3))
        rel = "src/" + path.split("src/", 1)[1]
        new = line_map(old, rel)(num)
        if new == num:
            return m.group(0)
        return f"[{label.replace(str(num), str(new))}]({path}#L{new})"

    return re.sub(r'\[([^\]]*)\]\(([^)#\s]*?src/[^)#\s]+\.rs)#L(\d+)\)', rep, text)


def targets(heading):
    spans = re.findall(r'`([^`]+)`', heading)
    out = []
    for s in spans:
        s = s.strip()
        words = s.split()
        if words and words[0] in TYPE_KW and len(words) > 1:
            out.append(("type", words[-1].split("::")[-1].split("<")[0]))
        else:
            s = re.sub(r'^(pub(\([^)]*\))?\s+)?fn\s+', "", s)
            s = re.sub(r'\(.*$', "", s).strip()
            s = re.sub(r'<.*$', "", s).strip()
            if re.match(r'^[A-Za-z_][\w:]*$', s):
                out.append(("fn", s))
        if out and heading.count("`") <= 2:
            break
    return out


def split_cells(row):
    """Cells of a markdown table row, honouring escaped pipes."""
    body = row.strip()[1:-1].replace("\\|", "\x00")
    return [c.strip().replace("\x00", "|") for c in body.split("|")]


def number(seen, key):
    seen[key] = seen.get(key, 0) + 1
    return key if seen[key] == 1 else f"{key}#{seen[key]}"


def harvest(pages):
    lines = []
    for p in pages:
        lines += open(p, encoding="utf-8").read().split("\n") + [""]
    title = ""
    old = None
    m = re.match(r'#\s+`[^`]+`\s+—\s+(.*)$', lines[0]) if lines else None
    if m:
        title = m.group(1).strip()
    for line in lines[:8]:
        cm = re.search(r'Line links as of commit `([0-9a-f]+)`', line)
        if cm:
            old = cm.group(1)
            break

    intro, entries, tests_intro = [], [], []
    tests, fields, variants, groups = [], {}, {}, []
    mode = "head"
    cur = None
    buf = []
    in_fence = False
    group_title = None
    group_buf = []
    test_seen = {}
    seen_body = False

    def close():
        nonlocal cur, buf
        if cur is not None:
            entries.append((cur, buf))
        cur, buf = None, []

    def close_group():
        nonlocal group_title, group_buf
        if group_title:
            text = "\n".join(group_buf).strip()
            if text:
                groups.append((group_title, text))
        group_title, group_buf = None, []

    for line in lines:
        if line.strip().startswith("```"):
            in_fence = not in_fence
            fence_line = True
        else:
            fence_line = False
        h = None if (in_fence and not fence_line) or fence_line else re.match(r'^(#{1,4})\s+(.*)$', line)
        if h and len(h.group(1)) == 1:
            continue
        if h and len(h.group(1)) == 2:
            close()
            close_group()
            t = h.group(2)
            body_heading = t.startswith(("Types", "Free functions", "Functions", "`"))
            if t.startswith("Tests"):
                mode = "tests_intro"
                seen_body = True
            elif not body_heading:
                # any other section is the page's own writing: it goes with the intro
                mode = "intro"
                intro.append(line)
            else:
                mode = "body"
                seen_body = True
                if "`" in t:
                    group_title = t.replace("`", "").strip()
            continue
        if h and (mode in ("body", "tests", "tests_intro") or
                  (mode == "intro" and targets(h.group(2)) and not re.match(r'Rust concepts', h.group(2)))):
            if mode == "intro":
                mode = "body"
            if mode == "tests_intro":
                mode = "tests"
            close()
            close_group()
            t = h.group(2)
            if re.match(r'Rust concepts', t):
                cur = ("concepts", t)
                continue
            tg = targets(t) if mode == "body" else []
            # a body heading naming no item is the page's own writing: keep it with the intro
            cur = ("entry", tg) if tg else (("extra", t) if mode == "body" else ("skip", t))
            continue
        if mode == "intro":
            intro.append(line)
        elif mode == "tests_intro":
            tests_intro.append(line)
        elif mode in ("body", "tests"):
            if group_title is not None and cur is None:
                group_buf.append(line)
                continue
            if cur is None:
                cur = ("skip", "")
            buf.append(line)
            if mode == "tests":
                mt = re.match(r'^\|\s*\[`([A-Za-z_]\w*)`\]\([^)]*\)\s*\|', line)
                if mt:
                    cells = split_cells(line)
                    tests.append((number(test_seen, mt.group(1)), cells[-1] if len(cells) > 1 else ""))
                mh = re.match(r'^\s*[-*]\s*\[`([A-Za-z_][\w:]*)`\]\([^)]*\)\s*[—-]+\s*(.*)$', line)
                if mh:
                    tests.append((number(test_seen, mh.group(1).split("::")[-1]), mh.group(2)))
    close()
    close_group()

    out = []
    if title and title != "TITLE":
        out.append(f"@@title {title}")
    concepts_extra = [f"### {tg}\n" + "\n".join(body).strip() for (kind, tg), body in entries
                      if kind in ("concepts", "extra") and "\n".join(body).strip()]
    intro_txt = "\n".join(intro).strip()
    if intro_txt and "MISSING INTRO" not in intro_txt:
        if concepts_extra:
            intro_txt += "\n\n" + "\n\n".join(concepts_extra)
        out.append("@@intro\n" + intro_txt)
    for gt, text in groups:
        out.append(f"@@group {gt}\n{text}")
    fn_seen = {}
    for (kind, tg), body in entries:
        if kind != "entry":
            continue
        b = list(body)
        while b and not b[0].strip():
            b.pop(0)
        if b and re.match(r'^\[src/[^\]]*\]\(', b[0].strip()):
            b.pop(0)
        while b and not b[0].strip():
            b.pop(0)
        if b and b[0].strip().startswith("```"):
            k = 1
            while k < len(b) and not b[k].strip().startswith("```"):
                k += 1
            b = b[k + 1:]
        # Field / variant tables are regenerated from the source (their meanings are kept);
        # any other table is part of the explanation and stays.
        text_lines = []
        table = None  # None, "members" or "prose"
        for line in b:
            if not line.startswith("|"):
                table = None
                text_lines.append(line)
                continue
            if table is None:
                head = [c.lower().strip("* ") for c in split_cells(line)]
                is_type = any(k2 == "type" for k2, _ in tg)
                table = "members" if is_type and head and head[0] in ("field", "variant") else "prose"
            if table == "prose":
                text_lines.append(line)
                continue
            row = re.match(r'^\|\s*`([^`]+)`\s*\|', line)
            if row:
                cells = split_cells(line)
                nm = re.match(r'[A-Za-z_0-9]\w*', row.group(1).strip())
                name = nm.group(0) if nm else row.group(1).strip()
                meaning = cells[-1] if len(cells) >= 2 else ""
                for k2, t in tg:
                    if k2 == "type" and meaning:
                        (fields if len(cells) >= 3 else variants).setdefault(t, {})[name] = meaning
        text = re.sub(r'\n{3,}', "\n\n", "\n".join(text_lines).strip())
        for k2, t in tg:
            key = number(fn_seen, t) if k2 == "fn" else t
            if text and text != "MISSING":
                out.append(f"@@{k2} {key}\n{text}")
    for t, rows in fields.items():
        out.append(f"@@fields {t}\n" + "\n".join(f"{k}: {v}" for k, v in rows.items()))
    for t, rows in variants.items():
        out.append(f"@@variants {t}\n" + "\n".join(f"{k}: {v}" for k, v in rows.items()))
    tests_txt = "\n".join(tests_intro).strip()
    if tests_txt:
        out.append("@@tests\n" + tests_txt)
    for t, v in tests:
        if v:
            out.append(f"@@test {t}\n{v}")
    return remap_links("\n\n".join(out) + "\n", old if old != head_commit() else None)


def page_for(rel):
    return os.path.join(ROOT, "docs", rel[: -len(".rs")] + ".md")


def write_prose(rel, text):
    dest = os.path.join(PROSE_DIR, rel.replace("/", os.sep) + ".0-harvest.md")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "w", encoding="utf-8", newline="\n").write(text)
    return dest


if __name__ == "__main__":
    rel = sys.argv[1].replace("\\", "/")
    pages = sys.argv[2:] or [page_for(rel)]
    dest = write_prose(rel, harvest(pages))
    print("wrote", dest)
