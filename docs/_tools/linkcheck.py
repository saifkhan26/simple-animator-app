"""Check every relative link in docs/: target exists, #L<n> within the file, #anchor matches a heading."""
import os
import re
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))
DOCS = os.path.join(ROOT, "docs")
LINK_RE = re.compile(r"\]\(([^)\s]+)\)")
_slugs = {}
_lines = {}


def slugify(text):
    t = text.strip().lower()
    t = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", t)  # links -> text
    t = t.replace("`", "")
    t = re.sub(r"[^\w\- ]", "", t)  # keep word chars, hyphen, space
    return t.replace(" ", "-")


def slugs(path):
    if path in _slugs:
        return _slugs[path]
    seen = {}
    out = set()
    fence = False
    for line in open(path, encoding="utf-8"):
        if line.strip().startswith("```"):
            fence = not fence
        if fence:
            continue
        m = re.match(r"^(#{1,6})\s+(.*?)\s*$", line)
        if m:
            s = slugify(m.group(2))
            n = seen.get(s, 0)
            out.add(s if n == 0 else f"{s}-{n}")
            seen[s] = n + 1
    _slugs[path] = out
    return out


bad = 0
checked = 0
for d, _, files in os.walk(DOCS):
    for f in files:
        if not f.endswith(".md"):
            continue
        page = os.path.join(d, f)
        rel = os.path.relpath(page, ROOT).replace("\\", "/")
        fence = False
        for ln, line in enumerate(open(page, encoding="utf-8"), 1):
            if line.strip().startswith("```"):
                fence = not fence
            if fence:
                continue
            # inline code spans hold example links, not real ones
            for target in LINK_RE.findall(re.sub(r"`[^`]*`", "", line)):
                if re.match(r"^[a-z]+:", target) or target.startswith("<"):
                    continue
                checked += 1
                path, _, frag = target.partition("#")
                full = os.path.normpath(os.path.join(d, path)) if path else page
                if not os.path.exists(full):
                    bad += 1
                    print(f"MISSING  {rel}:{ln} -> {target}")
                    continue
                if not frag:
                    continue
                m = re.fullmatch(r"L(\d+)", frag)
                if m:
                    if full not in _lines:
                        _lines[full] = open(full, encoding="utf-8", errors="replace").read().count("\n") + 1
                    if int(m.group(1)) > _lines[full]:
                        bad += 1
                        print(f"BADLINE  {rel}:{ln} -> {target}")
                    continue
                if full.endswith(".md") and frag not in slugs(full):
                    bad += 1
                    print(f"ANCHOR   {rel}:{ln} -> {target}")
print(f"checked {checked} links, {bad} problems")
sys.exit(1 if bad else 0)
