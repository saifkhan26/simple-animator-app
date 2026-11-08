import os
import re

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))
DOCS = os.path.join(ROOT, "docs")

HEAD = """# Animator — developer documentation

Documentation for every part of the Animator codebase, written for someone who knows **JavaScript** well and is new
to **Rust**. Every source file has its own page, and every function on it is explained, with links that open the code
at the right line (Ctrl+click in VS Code, or click on GitHub).

## Start here

1. **[getting-started.md](getting-started.md)** — install Rust, build, run, test; what is in the repository; what each
   dependency does.
2. **[rust-primer.md](rust-primer.md)** — the Rust you need, compared with JavaScript (ownership, `Option`/`Result`,
   traits, closures, threads, egui's immediate-mode UI…). Keep it open while reading.
3. **[architecture.md](architecture.md)** — how everything connects: startup, the frame loop, the document model,
   input, tools, rendering, undo, files, the Krita link.
4. **[walkthroughs.md](walkthroughs.md)** — follow one action through the code: drawing a line, undo, save, open,
   play, fill, export, Edit in Krita.
5. **[src/main.md](src/main.md)** then **[src/app.md](src/app.md)** — the entry point and the central `AppState`.
6. **[glossary.md](glossary.md)** — animation, graphics and Krita terms.

How the pages are laid out: [HOW-THESE-DOCS-WORK.md](HOW-THESE-DOCS-WORK.md). After changing code, refresh the
pages' line links and find what needs explaining with `python docs/_tools/refresh.py` then
`python docs/_tools/check.py` ([_tools/README.md](_tools/README.md)).

## Every source file

Each link opens that file's page. The page links to the source itself.
"""

GROUPS = [
    ("Entry point and app state", ["src/main", "src/app", "src/app/"]),
    ("Document model (`src/doc/`)", ["src/doc/"]),
    ("Tools (`src/tools/`)", ["src/tools/"]),
    ("Krita brush engine (`src/tools/krita/`)", ["src/tools/krita/"]),
    ("User interface (`src/ui/`)", ["src/ui/"]),
    ("Input (`src/input/`)", ["src/input/"]),
    ("Files: import, export, formats (`src/io/`)", ["src/io/"]),
    ("Timeline (`src/timeline/`)", ["src/timeline/"]),
    ("Brush library (`src/brush_library/`)", ["src/brush_library/"]),
    ("Krita link", ["src/krita_", "krita-plugin/"]),
    ("Everything else", ["src/"]),
]


def pages():
    out = []
    for d, _, files in os.walk(DOCS):
        for f in files:
            if not f.endswith(".md"):
                continue
            rel = os.path.relpath(os.path.join(d, f), DOCS).replace("\\", "/")
            if rel.startswith(("src/", "krita-plugin/")):
                out.append(rel)
    return sorted(out)


def title_of(rel):
    first = open(os.path.join(DOCS, rel), encoding="utf-8").readline().strip()
    m = re.match(r"#\s+`([^`]+)`\s+—\s+(.*)$", first)
    if m:
        return m.group(1), m.group(2)
    return rel, first.lstrip("# ")


def group_of(rel):
    src = rel[:-3]
    best = None
    for i, (_, prefixes) in enumerate(GROUPS):
        for p in prefixes:
            if src == p or src.startswith(p):
                if p.endswith("/") and "/krita/" in src and "tools/krita" not in p and p == "src/tools/":
                    continue
                cand = (len(p), i)
                if best is None or cand > best:
                    best = cand
    return best[1] if best else len(GROUPS) - 1


def main():
    rows = {i: [] for i in range(len(GROUPS))}
    for rel in pages():
        rows[group_of(rel)].append(rel)
    out = [HEAD]
    total = 0
    for i, (name, _) in enumerate(GROUPS):
        if not rows[i]:
            continue
        out.append(f"\n### {name}\n")
        out.append("| File | What it is |\n|---|---|")
        for rel in rows[i]:
            path, title = title_of(rel)
            out.append(f"| [`{path}`]({rel}) | {title} |")
            total += 1
    out.append(f"\n_{total} pages._\n")
    open(os.path.join(DOCS, "README.md"), "w", encoding="utf-8", newline="\n").write("\n".join(out))
    print("README.md:", total, "pages")


main()
