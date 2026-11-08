"""Docs coverage: every source file has a page, every non-test function a heading naming it
(as many headings as definitions), every #[test] and test helper a mention.

  python docs/_tools/coverage.py [--only substr]
"""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from collections import Counter

root = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith('--') else os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
only = None
if "--only" in sys.argv:
    only = sys.argv[sys.argv.index("--only") + 1]

FN_RE = re.compile(
    r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:unsafe\s+)?(?:async\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)'
)
DEF_RE = re.compile(r'^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)')
MACRO_RE = re.compile(r'^\s*macro_rules!\s*([A-Za-z_][A-Za-z0-9_]*)')


def sources():
    out = []
    for base in ("src", "krita-plugin"):
        for d, _, files in os.walk(os.path.join(root, base)):
            for f in files:
                p = os.path.relpath(os.path.join(d, f), root).replace("\\", "/")
                if p.endswith(".rs") or p.endswith(".py") or p.endswith(".desktop"):
                    out.append(p)
    return sorted(out)


def doc_for(src):
    if src.startswith("src/"):
        return "docs/" + src[: -len(".rs")] + ".md"
    if src.endswith(".py"):
        return "docs/" + src[: -len(".py")] + ".md"
    return "docs/" + src + ".md"


def parse_rust(text):
    lines = text.split("\n")
    fns = []  # (name, line, is_test)
    for i, line in enumerate(lines):
        m = FN_RE.match(line)
        name = None
        if m:
            name = m.group(1)
        else:
            m2 = MACRO_RE.match(line)
            if m2:
                name = m2.group(1)
        if not name:
            continue
        # comment lines are not definitions
        if line.lstrip().startswith("//"):
            continue
        is_test = False
        j = i - 1
        while j >= 0 and (lines[j].strip().startswith("#[") or lines[j].strip().startswith("///") or lines[j].strip().startswith("//")):
            if re.match(r"\s*#\[(tokio::)?test\b", lines[j]):
                is_test = True
            j -= 1
        fns.append((name, i + 1, is_test))
    return fns


def parse_py(text):
    fns = []
    for i, line in enumerate(text.split("\n")):
        m = DEF_RE.match(line)
        if m:
            name = m.group(1)
            fns.append((name, i + 1, name.startswith("test_")))
    return fns


def page_text(doc):
    p = os.path.join(root, doc)
    if os.path.exists(p):
        return open(p, encoding="utf-8").read()
    # big pages are still in parts
    stem = os.path.basename(doc)[: -len(".md")]
    parts_dir = os.path.join(root, "docs", "_parts")
    if os.path.isdir(parts_dir):
        parts = sorted(f for f in os.listdir(parts_dir) if f.startswith(stem + ".part"))
        if parts:
            return "\n".join(open(os.path.join(parts_dir, f), encoding="utf-8").read() for f in parts)
    return None


import gen as _gen
total_missing = 0
pages_missing = []
for src in sources():
    if only and only not in src:
        continue
    doc = doc_for(src)
    text = page_text(doc)
    if text is None:
        pages_missing.append(doc)
        continue
    if not src.endswith(".rs"):
        continue
    p = _gen.parse(os.path.join(root, src))
    _gen.classify(p)
    headings = [l for l in text.splitlines() if l.startswith("#")]
    need = Counter(it.name for it in p.items if it.kind == "fn" and it.role == "fn")
    missing = []
    for name, count in need.items():
        pat = re.compile(r"(?<![A-Za-z0-9_])" + re.escape(name) + r"(?![A-Za-z0-9_])")
        have = sum(1 for h in headings if pat.search(h))
        if have < count:
            missing.append(f"{name} (defs {count}, headings {have})")
    tests_missing = [f"{it.name}@{it.line}" for it in p.items if it.kind == "fn" and it.role in ("test", "helper")
                     and not re.search(r"(?<![A-Za-z0-9_])" + re.escape(it.name) + r"(?![A-Za-z0-9_])", text)]
    if "MISSING" in text:
        missing.append("page contains MISSING")
    if missing or tests_missing:
        total_missing += len(missing) + len(tests_missing)
        print(f"== {src} -> {doc}")
        for m in missing:
            print("   fn  ", m)
        if tests_missing:
            print("   tests:", ", ".join(tests_missing))

print()
print("pages missing:", len(pages_missing))
for p in pages_missing:
    print("   ", p)
print("functions/tests missing:", total_missing)
exit_code = 1 if (total_missing or pages_missing) else 0

sys.exit(exit_code)
