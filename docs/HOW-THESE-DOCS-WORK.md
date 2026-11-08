# How these docs work

This page explains how the documentation is laid out, how to read a file page,
and the exact rules every page follows. If you add or change a source file,
use the template at the bottom to keep its page in the same shape.

## Layout

```text
docs/
├── README.md                  ← start here: reading order + index of every file
├── getting-started.md         ← build, run, test; what is in the repo
├── rust-primer.md             ← Rust explained for JavaScript developers
├── architecture.md            ← how all the parts connect, from main() onward
├── walkthroughs.md            ← end-to-end traces ("what happens when I draw a line")
├── glossary.md                ← animation / graphics / Krita words used in the code
├── HOW-THESE-DOCS-WORK.md     ← this page
├── src/                       ← ONE page per Rust source file, same folder shape as src/
│   ├── main.md                ← docs for src/main.rs
│   ├── app.md                 ← docs for src/app.rs
│   ├── app/select.md          ← docs for src/app/select.rs
│   └── …
└── krita-plugin/              ← one page per file of the Python Krita plugin
```

**The rule:** source file `src/<path>.rs` is documented in `docs/src/<path>.md`.
`src/tools/krita/tip/auto.rs` → `docs/src/tools/krita/tip/auto.md`.
A `mod.rs` file gets `mod.md` in the same folder.

## Clickable source links

Every page links back to the code. Links are ordinary relative Markdown links,
so in VS Code (editor or preview), RustRover and on GitHub you can
**Ctrl+click** (Cmd+click on macOS) them:

- `[src/app/select.rs](../../../src/app/select.rs)` opens the file.
- `[src/app/select.rs:120](../../../src/app/select.rs#L120)` opens it at line 120.

Each page's header says which commit its line numbers were taken at (`4c83de8` or
`1fa5845`; files that didn't change between the two have identical lines). When code
moves, a line link may be off by a few lines — the function name in the heading is the
reliable anchor; search for it with Ctrl+F / Ctrl+Shift+F.

## How to read a file page

Each page has the same sections, in this order:

1. **Header** — the source path (clickable), its Rust module path (`crate::app::select`), size.
2. **What this file is for** — plain English, no Rust knowledge needed.
3. **How it connects** — which files use this one and which ones it uses, as links.
4. **Rust concepts in this file** — the Rust features this file leans on, each with a tiny
   snippet and the closest JavaScript equivalent. Longer explanations live in the
   [Rust primer](rust-primer.md).
5. **Types** — every `struct`, `enum`, `trait`, `type` alias, `const` and `static`, with a table
   of fields / variants.
6. **Functions** — every function and method, one `###` heading each, in source order
   (grouped under the `impl` block they belong to).
7. **Tests** — a table with one line per `#[test]` function saying what behaviour it protects,
   plus any test helper functions.

## Keeping the docs up to date

The file pages are partly generated. After changing code, run from the repository root:

```bash
python docs/_tools/refresh.py
```

```bash
python docs/_tools/check.py
```

`refresh.py` regenerates each page's headings, line links, signatures and tables from the current source while
keeping every explanation; new functions appear with their doc comment or `MISSING`, removed ones are reported.
Then write the missing explanations straight into the pages. Details: [_tools/README.md](_tools/README.md).

---

# Authoring rules (for whoever writes or updates a page)

## Audience

Write for a developer who knows **JavaScript/TypeScript well and Rust barely at all**.
That means:

- Explain *what* and *why* in plain words before *how*.
- When a Rust feature appears that a JS developer would not know (ownership, `&mut`,
  `Option`, `match`, traits, lifetimes, `impl`, `?`, `as` casts, `Arc`, macros…), explain it
  the first time it matters on that page, with a 2–6 line snippet and a JS comparison, and
  link the primer section for more.
- Domain words (cell, onion skin, dab, premultiplied alpha, x-sheet…) get a one-line
  explanation on first use, and a link to [glossary.md](glossary.md).
- Never invent behaviour. Every statement must come from reading the code (the `///` doc
  comments in the source are very good — paraphrase them in simpler words). If something is
  genuinely unclear after reading, say what the code does literally.

## Paths and links

- Source link prefix depends on how deep the page is. From `docs/src/app.md` the repo root is
  `../../`; from `docs/src/app/select.md` it is `../../../`; one more `../` per extra folder.
- Link to another doc page relatively: from `docs/src/app/select.md` to the page for
  `src/tools/selection.rs` write `[tools/selection.rs](../tools/selection.md)`.
- Link the primer as `rust-primer.md#<section>` with the right `../` prefix. The primer's
  section anchors are fixed:

| Anchor | Covers |
|---|---|
| `#reading-a-rust-file` | `use`, `fn`, `let`, `;` vs no-`;` return, comments, `///` docs |
| `#variables-and-mutability` | `let`, `let mut`, shadowing, type annotations |
| `#numbers-and-casts` | `u8`/`i32`/`u32`/`usize`/`f32`/`f64`, `as`, overflow, `saturating_*`, `clamp` |
| `#strings-and-slices` | `String` vs `&str`, `&[T]`, `&mut [T]`, indexing, ranges `a..b` |
| `#structs` | structs, field init shorthand, tuple structs, `..Default::default()` |
| `#enums-and-match` | enums with data, `match`, `if let`, `let else`, `matches!` |
| `#option-and-result` | `Option`, `Result`, `?`, `unwrap`, `map`, `ok_or`, `anyhow` |
| `#impl-blocks-and-methods` | `impl`, `self` / `&self` / `&mut self`, `Self`, `new()` |
| `#ownership-and-borrowing` | move, `&` borrow, `&mut` borrow, `clone()`, `Copy` |
| `#lifetimes` | `'a`, why some structs hold references |
| `#traits-and-derive` | traits, `impl Trait for T`, `#[derive(...)]`, `Default`, `From` |
| `#generics` | `<T>`, trait bounds, `impl Trait` arguments |
| `#closures` | `|x| x + 1`, `move`, `Fn`/`FnMut`/`FnOnce` |
| `#iterators` | `.iter()`, `.map()`, `.filter()`, `.collect()`, `for` loops |
| `#collections` | `Vec`, arrays `[T; N]`, `HashMap`, `HashSet`, `VecDeque`, `BTreeMap` |
| `#modules-and-visibility` | `mod`, `pub`, `pub(crate)`, `pub(super)`, `crate::`, `super::` |
| `#smart-pointers` | `Box`, `Rc`, `Arc`, `RefCell`, `Mutex`, `Cow` |
| `#threads-and-channels` | `std::thread`, scoped threads, `mpsc` channels, atomics |
| `#macros` | `println!`, `vec!`, `format!`, `assert!`, `macro_rules!` |
| `#attributes-and-cfg` | `#[derive]`, `#[cfg(test)]`, `#[cfg(target_os)]`, `#[inline]`, `#[allow]` |
| `#constants-and-statics` | `const`, `static`, `thread_local!`, `OnceLock` |
| `#tests` | `#[test]`, `cargo test`, `assert_eq!`, test modules |
| `#unsafe-and-ffi` | `unsafe`, raw pointers, calling C / Windows APIs |
| `#serde-and-file-formats` | `Serialize` / `Deserialize`, RON, TOML, postcard |
| `#egui-and-immediate-mode-ui` | how egui draws UI every frame (vs. React/DOM) |

## Page template

Copy this skeleton. Replace everything in `<…>`. Keep the section order and headings.

````markdown
# `src/<path>.rs` — <short title in plain words>

> **Source:** [src/<path>.rs](<prefix>src/<path>.rs) · **Module:** `crate::<path with ::>` ·
> **Size:** <N> lines · Line links as of commit `4c83de8`

## What this file is for

<1–4 paragraphs. What job this file does in the app, in plain words. Where it sits
(e.g. "part of the document model", "drawn every frame by the UI"). An analogy to a JS /
web concept is welcome when it fits.>

## How it connects

- **Used by:** <links to pages of files that call into this one, with a few words on why>
- **Uses:** <links to pages of files this one calls>

<Optional small ASCII diagram in a ```text block if the flow is not obvious.>

## Rust concepts in this file

<2–6 short items, chosen from what this file actually uses. Each: a heading-less bold
lead, a tiny snippet taken from (or simplified from) this file, the JS equivalent,
one or two sentences, and a primer link.>

**`Option<T>` instead of `null`.** …
```rust
let hit: Option<usize> = layers.iter().position(|l| l.uid == uid);
```
```js
const hit = layers.findIndex(l => l.uid === uid); // -1 when missing
```
More: [Option and Result](<docs-root prefix>rust-primer.md#option-and-result)

## Types

### `struct <Name>`
[src/<path>.rs:<line>](<prefix>src/<path>.rs#L<line>)

<What one value of this type represents.>

| Field | Type | Meaning |
|---|---|---|
| `field` | `u32` | … |

### `enum <Name>`
…variants table (Variant | Meaning)…

### `const <NAME>`
…

## Functions

<If the file has impl blocks, add a `## impl <Type>` heading line per block (or
"Functions on <Type>"), then the method entries under it. Free functions under
"## Free functions". Keep source order.>

### `<name>` (free function) or `<Type>::<name>` (method)
[src/<path>.rs:<line>](<prefix>src/<path>.rs#L<line>)

```rust
<the signature, copied from the source; long ones may be wrapped>
```

**What it does.** <plain-English purpose, 1–4 sentences>

**Parameters.** <bullet per parameter — skip for none / only `self`>

**Returns.** <what comes back; skip for `()`>

**How it works.** <numbered steps — only for non-trivial functions>

**Example.** <only where the concept is hard: a small usage example or a JS
re-implementation of the core idea>

**Used by:** <callers you found with search; omit rather than guess>

## Tests

Run with `cargo test <module path>` (e.g. `cargo test app::select`).

| Test | What it checks |
|---|---|
| [`name_of_test`](<prefix>src/<path>.rs#L<line>) | <one sentence> |

**Test helpers:** `helper_name` — <one line>.
````

## How much to write per function

- **Every** function gets its own `###` entry: public, private, methods, trait methods
  (`fn default`, `fn fmt`, `fn drop`, `fn from`…), and functions nested inside other functions
  (document those under their parent with a `####` heading). `#[test]` functions go in the
  Tests table instead.
- **Tiny** functions (getters, one-liners, ≤ ~5 lines): signature + 1–3 sentences.
- **Medium** functions: what it does, parameters, returns, used by.
- **Big or tricky** functions (long, algorithmic, maths, threading, unsafe, GPU): add
  "How it works" steps; add an **Example** when the idea is hard to see from the prose.
- `macro_rules!` macros: document like functions.
- Large `struct`s (dozens of fields): table every field; group rows with bold sub-headers
  in the table if it helps.
