"""Bring the generated doc pages up to date with the code.

  python docs/_tools/refresh.py              every src/**/*.rs
  python docs/_tools/refresh.py src/app.rs   just these files

For each source file: read its existing page back into prose (harvest.py), then rebuild the
page from the current source (gen.py). Line links, signatures, field/variant tables and test
tables are regenerated; explanations are kept. A new function gets a heading with its doc
comment as a stand-in, or MISSING when it has none; a removed function's entry disappears.
A source file with no page yet gets a new page with MISSING placeholders.

Then docs/README.md's index is rebuilt. Pages for the Python plugin (docs/krita-plugin/)
are hand-written and not touched. Run check.py afterwards.
"""
import os
import shutil
import sys
import tempfile

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))

# A fresh prose folder for this run, set before gen/harvest read it.
PROSE = tempfile.mkdtemp(prefix="docs-prose-")
os.environ["DOCS_PROSE"] = PROSE
sys.path.insert(0, TOOLS)
sys.stdout.reconfigure(encoding="utf-8")

import gen  # noqa: E402
import harvest  # noqa: E402


def all_sources():
    out = []
    for d, _, files in os.walk(os.path.join(ROOT, "src")):
        for f in files:
            if f.endswith(".rs"):
                out.append(os.path.relpath(os.path.join(d, f), ROOT).replace("\\", "/"))
    return sorted(out)


def orphans(sources):
    want = {"docs/" + s[:-3] + ".md" for s in sources}
    out = []
    for d, _, files in os.walk(os.path.join(ROOT, "docs", "src")):
        for f in files:
            rel = os.path.relpath(os.path.join(d, f), ROOT).replace("\\", "/")
            if f.endswith(".md") and rel not in want:
                out.append(rel)
    return out


def save_dropped(rel, unknown):
    """Explanations whose item is gone (removed or renamed): written to target/docs-dropped/
    so they can be moved under the new name instead of being lost."""
    prose = gen.load_prose(rel)
    p = gen.parse(os.path.join(ROOT, rel))
    gen.classify(p)
    gen.resolve_keys(p, prose)
    out = []
    for entry in unknown:
        kind, key = entry.split(" ", 1)
        val = prose.get(kind, {}).get(key)
        if isinstance(val, dict):
            val = "\n".join(f"{k}: {v}" for k, v in val.items())
        if val:
            out.append(f"## {kind} {key}\n\n{val}\n")
    dest = os.path.join(ROOT, "target", "docs-dropped", rel.replace("/", os.sep) + ".md")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    open(dest, "w", encoding="utf-8", newline="\n").write("\n".join(out))
    return unknown, os.path.relpath(dest, ROOT)


def main(args):
    sources = [a.replace("\\", "/") for a in args] or all_sources()
    new_pages, missing, fallback, dropped = [], {}, {}, {}
    try:
        for rel in sources:
            page = harvest.page_for(rel)
            if os.path.exists(page):
                harvest.write_prose(rel, harvest.harvest([page]))
            else:
                new_pages.append(rel)
            _, problems, fb, unknown, _ = gen.build(rel)
            if problems:
                missing[rel] = problems
            if fb:
                fallback[rel] = fb
            if unknown:
                dropped[rel] = save_dropped(rel, unknown)
    finally:
        shutil.rmtree(PROSE, ignore_errors=True)

    import make_index  # noqa: F401  (rebuilds docs/README.md on import)

    print(f"refreshed {len(sources)} page(s)")
    for rel in new_pages:
        print(f"  NEW PAGE  docs/{rel[:-3]}.md — write its title and intro")
    for rel, probs in missing.items():
        print(f"  MISSING   {rel}: {'; '.join(probs[:12])}{' …' if len(probs) > 12 else ''}")
    for rel, (items, where) in dropped.items():
        print(f"  DROPPED   {rel}: {', '.join(items[:8])} — no such item now; their text is in {where}")
    for rel, fb in fallback.items():
        print(f"  doc-only  {rel}: {', '.join(fb[:12])}{' …' if len(fb) > 12 else ''}")
    if not args:
        for o in orphans(sources):
            print(f"  ORPHAN    {o} — its source file is gone; delete the page if so")
    if missing or new_pages:
        print("Search the pages for MISSING and write the explanations (see HOW-THESE-DOCS-WORK.md).")


if __name__ == "__main__":
    main(sys.argv[1:])
