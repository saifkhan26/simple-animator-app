# docs/_tools — keeping the docs in step with the code

Small Python 3 scripts (standard library only, plus `git` on the PATH). Run them from the repository root.

## The two commands you need

```bash
python docs/_tools/refresh.py
```

```bash
python docs/_tools/check.py
```

**`refresh.py`** rebuilds every page under `docs/src/` from the current source. For each `src/x.rs` it reads
`docs/src/x.md` back into its explanations, then writes the page again from the code:

- headings, `src/x.rs:N` line links, signatures, field/variant tables and test tables are regenerated, so line
  numbers are always current;
- explanations you (or anyone) wrote are kept — and line links *inside* explanations are moved to where that code
  is now (`git diff` from the commit in the page header);
- a **new function** gets a heading, with its `///` doc comment as a stand-in explanation, or the word `MISSING`;
- a **removed or renamed** function's entry disappears from the page, and its text is saved to
  `target/docs-dropped/src/x.rs.md` (reported as `DROPPED`) so you can paste it under the new name;
- a **new source file** gets a new page with `MISSING` placeholders;
- `docs/README.md`'s index is regenerated.

Pass file paths to refresh only those: `python docs/_tools/refresh.py src/app.rs src/undo.rs`.
Running it twice in a row changes nothing.

**`check.py`** fails if a source file has no page, a function has no heading, a test isn't listed, or any link
(file, `#L` line, `#heading`) is broken.

## Writing explanations

Edit the pages directly — they are the source. Write under a function's heading, after its signature block;
`refresh.py` keeps whatever is there. To find gaps:

```bash
grep -rn "^MISSING" docs/src
```

The page format, audience and "how much to write" rules are in [HOW-THESE-DOCS-WORK.md](../HOW-THESE-DOCS-WORK.md).
Do not edit the generated parts (headings, links, signatures, tables' first columns): they are rewritten on every
refresh. A field's or variant's *meaning* cell is kept.

## The scripts

| Script | What it does |
|---|---|
| `refresh.py` | harvest every page, rebuild it, rebuild the index; reports MISSING / DROPPED / NEW PAGE / ORPHAN |
| `check.py` | runs `coverage.py` and `linkcheck.py` |
| `gen.py` | the generator: parses a `.rs` file (fns, impls, types, fields, tests) and writes its page from prose. Also `keys`, `digest` (signatures + doc comments, handy when writing explanations) and `gaps` subcommands |
| `harvest.py` | turns a page back into prose directives for `gen.py` |
| `coverage.py` | page/function/test coverage |
| `linkcheck.py` | every relative link, line and anchor |
| `make_index.py` | writes `docs/README.md` (run by `refresh.py`) |

The Python Krita plugin's pages (`docs/krita-plugin/`) and the overview pages (`architecture.md`, `walkthroughs.md`,
`rust-primer.md`, …) are written by hand and never touched by these scripts.
