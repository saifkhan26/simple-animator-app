# Rust primer for JavaScript developers

This page teaches the Rust you need to read this codebase, by comparing each idea to
JavaScript/TypeScript. You do not need to read it top to bottom before anything else —
every file page links straight to the section it relies on. Examples are taken from (or
simplified from) this project's real code, with a link to where they come from.

If you only read three sections, read [Ownership and borrowing](#ownership-and-borrowing),
[Option and Result](#option-and-result) and [egui and immediate mode UI](#egui-and-immediate-mode-ui).

**Contents**

1. [Reading a Rust file](#reading-a-rust-file)
2. [Variables and mutability](#variables-and-mutability)
3. [Numbers and casts](#numbers-and-casts)
4. [Strings and slices](#strings-and-slices)
5. [Structs](#structs)
6. [Enums and match](#enums-and-match)
7. [Option and Result](#option-and-result)
8. [Impl blocks and methods](#impl-blocks-and-methods)
9. [Ownership and borrowing](#ownership-and-borrowing)
10. [Lifetimes](#lifetimes)
11. [Traits and derive](#traits-and-derive)
12. [Generics](#generics)
13. [Closures](#closures)
14. [Iterators](#iterators)
15. [Collections](#collections)
16. [Modules and visibility](#modules-and-visibility)
17. [Smart pointers](#smart-pointers)
18. [Threads and channels](#threads-and-channels)
19. [Macros](#macros)
20. [Attributes and cfg](#attributes-and-cfg)
21. [Constants and statics](#constants-and-statics)
22. [Tests](#tests)
23. [Unsafe and FFI](#unsafe-and-ffi)
24. [Serde and file formats](#serde-and-file-formats)
25. [egui and immediate mode UI](#egui-and-immediate-mode-ui)

---

## Reading a Rust file

A Rust source file is a list of *items*: `use` imports, `fn` functions, `struct`/`enum`
types, `impl` blocks (methods), `const`s, and `mod` (sub-module) declarations.

```rust
use std::sync::Arc;                 // import, like `import { Arc } from "std/sync"`
use crate::doc::canvas::Canvas;     // `crate::` = "from the root of this project"

/// Doc comment: three slashes. Shows up in IDE hovers and `cargo doc`.
/// Like a JSDoc `/** ... */` block.
pub fn area(w: u32, h: u32) -> u32 {   // params have types; `-> u32` is the return type
    // ordinary comment
    let pixels = w * h;               // `let` = `const` in JS (immutable by default)
    pixels                             // last expression WITHOUT `;` is the return value
}
```

```js
// JavaScript equivalent
export function area(w, h) {
  const pixels = w * h;
  return pixels;
}
```

Key differences to keep in mind:

- **The last line without a semicolon is returned.** `pixels` (no `;`) returns it.
  `pixels;` (with `;`) would return nothing. You can still write `return x;` to leave early.
- **Blocks are expressions.** `if`, `match` and `{ ... }` produce values:
  ```rust
  let label = if playing { "Pause" } else { "Play" };   // like a ternary
  ```
- **`//!` comments** at the top of a file document the file (module) itself.
- **Types after names:** `w: u32`, like TypeScript's `w: number`.
- **`fn` without `pub` is private** to its module (file). See [Modules and visibility](#modules-and-visibility).
- **`::` is path separator** (`std::sync::Arc`, `Canvas::new`), `.` is field/method access.

## Variables and mutability

```rust
let x = 5;          // immutable — like `const`
let mut y = 5;      // mutable — like `let`
y += 1;             // fine
// x += 1;          // compile error: x is not `mut`

let z: f32 = 0.5;   // explicit type annotation (usually inferred)

let w = "10";       // shadowing: a NEW variable with the same name
let w: u32 = w.parse().unwrap();  // replaces the old `w` from here on
```

Rust's `mut` goes further than JS `const`: an immutable binding also means you cannot change
the *contents* (fields of a struct, items of a `Vec`) through it. In JS a `const arr` can still
be pushed to; in Rust a non-`mut` `Vec` cannot.

`_` as a name means "I don't use this value": `let _ = tx.send(x);` deliberately ignores a result.

## Numbers and casts

JavaScript has one `number` type. Rust has many, and never converts between them silently.

| Rust type | Meaning | Typical use in this app |
|---|---|---|
| `u8` | 0..=255 | one colour channel of a pixel |
| `u16`, `u32`, `u64` | unsigned (never negative) integers | sizes, ids, counts |
| `i32`, `i64` | signed integers | pixel offsets that may go negative |
| `usize` | unsigned, pointer-sized (64-bit here) | indexes into arrays/`Vec`s, lengths |
| `f32` | 32-bit float | positions, pressures, opacities |
| `f64` | 64-bit float (= JS `number`) | precise maths, timing |
| `bool` | `true` / `false` | flags |

```rust
let w: u32 = 1920;
let half = w / 2;              // integer division: 960 (decimals dropped)
let x: f32 = w as f32 * 0.5;   // `as` converts. No implicit u32 → f32.
let idx = (y * w + x0) as usize;   // array indexes must be `usize`
let byte = (0.73_f32 * 255.0).round() as u8;  // f32 → u8 (186)
```

- **`as`** is the cast operator. Float → int truncates toward zero and *saturates*
  (`300.0 as u8` is `255`, `-1.0 as u8` is `0`).
- **Overflow:** in debug builds `255u8 + 1` panics (crashes with a message); in release it wraps.
  Code that wants a specific behaviour says so:
  `a.saturating_add(b)` (stops at max), `a.wrapping_add(b)` (wraps around),
  `a.checked_add(b)` (returns `None` on overflow).
- **`x.clamp(lo, hi)`**, `x.min(y)`, `x.max(y)`, `x.abs()`, `x.sqrt()`, `x.round()`,
  `x.floor()` are methods, not `Math.*` functions.
- Number literals can carry a type suffix and underscores: `1_000_000u64`, `0.5f32`.

## Strings and slices

Rust has two main string types:

| Type | What it is | JS analogy |
|---|---|---|
| `String` | an owned, growable string (heap) | a normal JS string you can build up |
| `&str` | a borrowed view into some string data | a read-only "window" onto a string |

```rust
let name: String = String::from("Layer 1");  // owned
let view: &str = &name;                      // borrowed view, no copy
let lit: &'static str = "Play";              // string literals are &'static str
let msg = format!("Frame {} of {}", 3, 24);  // like a template literal `Frame ${3} of ${24}`
```

Functions usually take `&str` (accepts both) and return `String` (caller owns the result).

**Slices** are the same idea for arrays: `&[T]` is a read-only view into a run of elements;
`&mut [T]` is a writable view.

```rust
fn sum(values: &[f32]) -> f32 { values.iter().sum() }

let v = vec![1.0, 2.0, 3.0, 4.0];
sum(&v);          // whole Vec as a slice
sum(&v[1..3]);    // elements 1 and 2 — ranges are `start..end` (end excluded)
```

Ranges: `a..b` is `a` up to but not including `b`; `a..=b` includes `b`; `..b`, `a..`, `..`
leave an end open. They're used both for slicing and for loops (`for i in 0..10`).

Pixel buffers in this app are `Vec<u8>` or `&[u8]`: 4 bytes (R, G, B, A) per pixel, row after
row — exactly like `ImageData.data` in a browser canvas. Pixel `(x, y)` starts at byte index
`(y * width + x) * 4`.

## Structs

A `struct` is a named bundle of fields — like a TypeScript `interface` plus the object itself.

```rust
// simplified from src/doc/layer.rs
pub struct Layer {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub exposures: Vec<Option<CellId>>,
}

let layer = Layer {
    name: "Ink".to_string(),
    opacity: 1.0,
    visible: true,
    exposures: Vec::new(),
};
println!("{}", layer.name);
```

```ts
interface Layer { name: string; opacity: number; visible: boolean; exposures: (number | null)[] }
const layer: Layer = { name: "Ink", opacity: 1, visible: true, exposures: [] };
```

Useful forms:

- **Field init shorthand:** `Layer { name, opacity, .. }` when variables have the same names
  (like `{ name, opacity }` in JS).
- **Struct update syntax:** `Foo { x: 1, ..Default::default() }` — "these fields, and defaults
  for the rest". Like `{ ...defaults, x: 1 }` in JS.
- **Tuple structs:** `struct LayerDrag(usize);` — a struct with unnamed fields, accessed as
  `drag.0`. Often used to give a plain value its own type ("newtype").
- **Unit structs:** `struct Marker;` — no fields at all.
- **Type aliases:** `pub type CellId = usize;` just gives a type another name (TS `type CellId = number`).

## Enums and match

Rust enums are much richer than TS enums: each variant can carry its own data. They're like
TypeScript *discriminated unions*.

```rust
// shaped like the app's background jobs (src/app.rs)
enum Job {
    Idle,
    Saving { path: String },
    Exporting { done: usize, total: usize },
}
```

```ts
type Job =
  | { kind: "Idle" }
  | { kind: "Saving"; path: string }
  | { kind: "Exporting"; done: number; total: number };
```

**`match`** is `switch` that must handle every case and can pull data out:

```rust
let text = match job {
    Job::Idle => "idle".to_string(),
    Job::Saving { path } => format!("saving {path}"),
    Job::Exporting { done, total } if total > 0 => format!("{}%", done * 100 / total),
    Job::Exporting { .. } => "exporting".to_string(),   // `..` = ignore the other fields
};
```

The compiler refuses to build if a variant is missing — add a variant and every `match` that
forgot it becomes an error. `_ => ...` is the "default" arm.

Shorter forms:

```rust
if let Job::Saving { path } = &job { println!("{path}"); }  // match one variant only

let Job::Saving { path } = job else { return; };   // "let-else": bind or bail out early

if matches!(job, Job::Idle | Job::Saving { .. }) { }  // true/false test, `|` = "or"
```

## Option and Result

Rust has **no `null` and no exceptions**. Instead it has two enums from the standard library:

```rust
enum Option<T> { Some(T), None }        // a value or nothing
enum Result<T, E> { Ok(T), Err(E) }     // success or an error
```

```rust
// src/doc/layer.rs: each frame slot of a layer may or may not show a drawing
pub exposures: Vec<Option<CellId>>,

match layer.exposures[frame] {
    Some(cell) => draw(cell),
    None => {}                          // empty frame
}
```

```ts
exposures: (number | null)[];
const cell = layer.exposures[frame];
if (cell !== null) draw(cell);
```

The point: you *cannot* forget the `None`/`Err` case — the type forces you to deal with it.

### Common Option/Result helpers

| Code | Meaning | JS-ish equivalent |
|---|---|---|
| `opt.unwrap()` | take the value, **crash** if `None` | `x!` (but panics) |
| `opt.expect("msg")` | same, with a message | |
| `opt.unwrap_or(d)` | value or default | `x ?? d` |
| `opt.unwrap_or_default()` | value or the type's default (0, "", empty) | `x ?? 0` |
| `opt.map(f)` | transform the inner value if present | `x == null ? null : f(x)` |
| `opt.and_then(f)` | like map where `f` also returns an Option | `x?.f()` |
| `opt.is_some()` / `is_none()` | test | `x != null` |
| `opt.ok_or(err)` | turn `None` into an `Err` | |
| `res.ok()` | turn a `Result` into an `Option`, dropping the error | |
| `opt.take()` | move the value out, leaving `None` behind | `const v = o.x; o.x = null;` |

### The `?` operator

`?` means "if this is an error (or `None`), return it from the current function right now;
otherwise give me the value". It replaces most `try/catch`.

```rust
use anyhow::Result;   // anyhow::Result<T> = Result<T, anyhow::Error> (any error type)

fn load(path: &Path) -> Result<Project> {
    let bytes = std::fs::read(path)?;        // I/O error? return it.
    let project = decode(&bytes)?;           // decode error? return it.
    Ok(project)                              // success: wrap the value in Ok
}
```

```js
async function load(path) {
  const bytes = await fs.readFile(path);   // throws on error
  const project = decode(bytes);           // throws on error
  return project;
}
```

The [`anyhow`](https://docs.rs/anyhow) crate is used throughout the I/O code: `anyhow::Result`
accepts any error type, `bail!("msg")` returns an error early, and `.context("msg")` adds a
description to an error on its way up.

## Impl blocks and methods

Methods are not written inside the struct. They go in a separate `impl` block:

```rust
pub struct Canvas { pub width: u32, pub height: u32, /* … */ }

impl Canvas {
    // No `self` → an "associated function", called as Canvas::new(…). Like a static method.
    pub fn new(width: u32, height: u32) -> Self {   // `Self` = the type being implemented (Canvas)
        Canvas { width, height /* … */ }
    }

    // `&self` → reads the canvas. Like a normal method that doesn't modify `this`.
    pub fn area(&self) -> u32 { self.width * self.height }

    // `&mut self` → may modify the canvas.
    pub fn clear(&mut self) { /* … */ }

    // `self` (no &) → consumes the value; the caller can't use it afterwards.
    pub fn into_pixels(self) -> Vec<u8> { /* … */ }
}

let mut c = Canvas::new(640, 480);
c.clear();
let a = c.area();
```

```js
class Canvas {
  static new(width, height) { return new Canvas(width, height); }
  area() { return this.width * this.height; }
  clear() { /* … */ }
}
```

- `self` is Rust's `this`, but it is written explicitly as the first parameter, and *how* it is
  taken (`&self`, `&mut self`, `self`) tells you whether the method reads, modifies or consumes.
- `new` is just a naming convention — Rust has no constructors.
- **One type can have many `impl` blocks, even in different files of the same crate.** This
  project uses that heavily: `AppState` is defined in `src/app.rs`, and `src/app/select.rs`,
  `src/app/liquify.rs`, etc. each add more methods with their own `impl AppState { … }`.
  In JS a class body must be in one place; in Rust it can be spread out.

## Ownership and borrowing

This is the idea that makes Rust different. In JS every object is shared by reference and the
garbage collector frees it when nobody points at it. Rust has no garbage collector; instead
**every value has exactly one owner**, and the value is freed when its owner goes away.

### Move

```rust
let a = vec![1, 2, 3];
let b = a;          // ownership MOVES to b. `a` can no longer be used.
// println!("{:?}", a);  // compile error: value moved
```

In JS `const b = a` makes two names for one array. In Rust it hands the array over.
Passing a value to a function also moves it:

```rust
fn consume(v: Vec<u8>) { /* v is freed at the end of this function */ }
consume(b);   // b is gone now
```

### Borrow with `&` (read-only) and `&mut` (read-write)

Usually you don't want to give a value away, just let someone look at it or change it.
That's a **borrow**:

```rust
fn total(v: &Vec<u8>) -> u32 { v.iter().map(|&x| x as u32).sum() }   // looks
fn bump(v: &mut Vec<u8>) { v.push(0); }                                // changes

let mut data = vec![1, 2, 3];
total(&data);        // lend it read-only
bump(&mut data);     // lend it writable
println!("{:?}", data);  // still ours
```

**The borrowing rule** (checked at compile time by the "borrow checker"): at any moment you may
have **either** any number of `&` (shared, read-only) borrows **or** exactly one `&mut`
(exclusive, writable) borrow — never both. This prevents a whole class of bugs (changing a list
while looping over it, two threads writing the same memory) without any runtime cost.

You'll see code shaped around this rule, e.g. copying a value out of `self` into a local
variable before calling another `&mut self` method, or `std::mem::take(&mut self.x)` to
temporarily move a field out.

### Clone and Copy

- `.clone()` makes an explicit deep copy (like `structuredClone(x)`). It's visible in the code
  so you can see where copies happen.
- Small plain types (`u32`, `f32`, `bool`, `char`, and structs marked `#[derive(Copy)]` like a
  2-D point) are **`Copy`**: assigning them copies them automatically, no move.

## Lifetimes

A reference (`&T`) must never outlive the value it points to. Most of the time the compiler
works this out by itself. When a *struct* holds a reference, or a function returns one derived
from its arguments, you sometimes see a **lifetime parameter** like `'a`:

```rust
// a struct that borrows something for as long as it exists
struct RowMenu<'a> {
    name: &'a str,        // "this &str lives at least as long as the RowMenu"
}

fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { if x.len() > y.len() { x } else { y } }
```

Read `'a` as a label tying references together: "the returned reference is valid as long as
both inputs are". It does not change what the code does at run time. `'static` means "valid for
the whole program" — string literals are `&'static str`.

## Traits and derive

A **trait** is a set of methods a type promises to have — like a TypeScript `interface`, except
that you can implement it for a type *after* the type is defined, in a separate block.

```rust
trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Layer {
    fn describe(&self) -> String { format!("layer {}", self.name) }
}
```

```ts
interface Describe { describe(): string }
class Layer implements Describe { describe() { return `layer ${this.name}`; } }
```

Standard traits you'll see everywhere:

| Trait | Gives you | JS analogy |
|---|---|---|
| `Clone` | `.clone()` | `structuredClone` |
| `Copy` | implicit copying for small values | primitives |
| `Debug` | `{:?}` formatting for logs | `console.log` dump |
| `Default` | `Type::default()` — a "zero" value | default parameter object |
| `PartialEq` / `Eq` | `==` | deep equality |
| `PartialOrd` / `Ord` | `<`, `>`, sorting | comparator |
| `Hash` | usable as a `HashMap` key | |
| `From<T>` / `Into<T>` | conversions: `Foo::from(t)`, `t.into()` | |
| `Display` | `{}` formatting (user-facing text) | `toString()` |
| `Drop` | code that runs when a value is freed | a destructor / `finally` |
| `Serialize` / `Deserialize` | saving/loading (see [Serde](#serde-and-file-formats)) | `JSON.stringify` |

**`#[derive(...)]`** asks the compiler to write the obvious implementation for you:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View { pub zoom: f32, pub pan_x: f32, pub pan_y: f32 }
```

`impl Default for X { fn default() -> Self { … } }` written by hand means the default isn't
just "all zeros" — read it to learn the real starting values.

## Generics

Generics are the same idea as in TypeScript: code that works for many types.

```rust
fn first<T: Clone>(items: &[T]) -> Option<T> {   // T must implement Clone ("trait bound")
    items.first().cloned()
}
```

```ts
function first<T>(items: T[]): T | undefined { return items[0]; }
```

Bounds can also be written with `where`:

```rust
fn paint<F>(f: F) where F: Fn(u32, u32) -> u8 { /* … */ }
```

`impl Trait` in argument position is shorthand: `fn paint(f: impl Fn(u32) -> u8)` means
"any type that is a function from u32 to u8". In return position, `-> impl Iterator<Item = u32>`
means "some iterator; the exact type is hidden".

`dyn Trait` (usually as `Box<dyn Trait>` or `&dyn Trait`) means "some value implementing the
trait, decided at run time" — closest to how JS treats every object.

## Closures

Closures are Rust's arrow functions.

```rust
let double = |x: f32| x * 2.0;                 // (x) => x * 2
let add = |a, b| { let s = a + b; s };         // block body
layers.iter().filter(|l| l.visible).count();   // passed inline, like JS
```

- A closure can use variables from its surroundings, like in JS. Normally it *borrows* them.
- **`move |…| …`** makes the closure take ownership of what it uses. Required when the closure
  outlives the current function — e.g. it runs on another thread:
  ```rust
  thread::spawn(move || { let _ = tx.send(probe(&probe_path)); });
  ```
- Function parameters that accept closures are typed by trait: `Fn` (only reads captured
  values), `FnMut` (may modify them), `FnOnce` (may consume them; called at most once).
  `fn(&Path) -> Result<Vec<Canvas>>` (lower-case `fn`) is a plain function pointer.

## Iterators

Rust's iterator methods look like JS array methods, but they are **lazy**: nothing runs until
something consumes the iterator (`collect`, `sum`, `count`, a `for` loop…).

```rust
let visible_names: Vec<String> = layers
    .iter()                       // borrow each element: yields &Layer
    .filter(|l| l.visible)        // like .filter
    .map(|l| l.name.clone())      // like .map
    .collect();                   // build the Vec (JS returns arrays directly)
```

```js
const visibleNames = layers.filter(l => l.visible).map(l => l.name);
```

| Rust | JS |
|---|---|
| `.iter()` / `.iter_mut()` / `.into_iter()` | iterate by `&`, by `&mut`, by value (consumes) |
| `.map(f)`, `.filter(f)` | same |
| `.enumerate()` | yields `(index, item)` — like `.forEach((item, i) => …)` |
| `.zip(other)` | pairs items from two iterators |
| `.find(f)`, `.position(f)` | `.find`, `.findIndex` (return `Option`) |
| `.any(f)`, `.all(f)` | `.some`, `.every` |
| `.sum()`, `.count()`, `.min()`, `.max()` | reductions |
| `.fold(init, f)` | `.reduce(f, init)` |
| `.rev()` | iterate backwards |
| `.chunks(n)` / `.chunks_exact(4)` | groups of n (e.g. 4 bytes = one pixel) |
| `.collect::<Vec<_>>()` | `Array.from(...)` |

`for x in &v { … }` loops over borrowed items; `for (i, x) in v.iter().enumerate()` gets indexes;
`for i in 0..n` counts.

## Collections

| Rust | JS | Notes |
|---|---|---|
| `Vec<T>` | `Array` | growable list. `vec![1, 2, 3]`, `v.push(x)`, `v.len()`, `v[i]` |
| `[T; N]` | fixed-length array | e.g. `[u8; 4]` for one RGBA pixel; size known at compile time |
| `HashMap<K, V>` | `Map` | `m.insert(k, v)`, `m.get(&k)` → `Option<&V>` |
| `HashSet<T>` | `Set` | |
| `BTreeMap<K, V>` | `Map` kept sorted by key | |
| `VecDeque<T>` | array used as a queue | fast push/pop at both ends |

Indexing with `v[i]` **panics** (crashes) if `i` is out of range; `v.get(i)` returns an `Option`
instead. Code that might be out of range uses `get`.

## Modules and visibility

Each file is a **module**. The tree is declared with `mod` lines, starting from `src/main.rs`:

```rust
// src/main.rs
mod app;     // loads src/app.rs           (module crate::app)
mod tools;   // loads src/tools/mod.rs     (module crate::tools)

// src/tools/mod.rs
pub mod fill;   // loads src/tools/fill.rs (module crate::tools::fill)
```

So `src/tools/krita/tip/auto.rs` is the module `crate::tools::krita::tip::auto`.
`mod.rs` is the folder's "index file" (like `index.js`). A file `src/app.rs` can also have
children in a folder `src/app/` — that's how `src/app/select.rs` becomes `crate::app::select`.

Paths:

- `crate::x::y` — from the project root (like an absolute import).
- `super::x` — from the parent module (like `../x`).
- `self::x` — from the current module.

Visibility (everything is private by default — the opposite of JS objects):

| Keyword | Visible to |
|---|---|
| *(nothing)* | this module and its children only |
| `pub` | everyone |
| `pub(crate)` | anywhere in this project, but not to outside crates |
| `pub(super)` | the parent module |

`use some::path::Thing;` brings a name into scope (like `import`); `pub use` re-exports it.

A **crate** is a package (an npm package). This project is one crate; its dependencies (egui,
image, serde…) are crates listed in `Cargo.toml` (≈ `package.json`) and downloaded by `cargo`
(≈ `npm`).

## Smart pointers

Because Rust has no garbage collector, sharing data needs explicit "smart pointer" types.

| Type | What it does | When you see it here |
|---|---|---|
| `Box<T>` | puts a value on the heap, single owner | `Box<dyn App>`, big or recursive values |
| `Rc<T>` | shared ownership via reference count (one thread) | rare |
| `Arc<T>` | shared ownership, thread-safe ("atomic" Rc) | cells: `Vec<Arc<Canvas>>` |
| `RefCell<T>` | lets you mutate through a shared reference, checked at run time | caches |
| `Mutex<T>` | lock-protected value shared between threads | background job state |
| `Cow<T>` | "clone on write": borrowed until you need to modify it | avoiding copies |

**The `Arc` + copy-on-write pattern** is central to this app's document model:

```rust
// src/doc/project.rs
/// Shared, copy-on-write: cloning the project (a save, an export, a
/// Krita send) costs a reference per cell, and only a cell edited while
/// such a copy is alive gets duplicated — on its first write.
pub cells: Vec<Arc<Canvas>>,
```

Cloning an `Arc` doesn't copy the canvas, it just adds another owner (bumps a counter). When
code needs to *write* to a cell it calls `Arc::make_mut(&mut arc)`: if nobody else holds the
canvas it's modified in place; if someone does (say a background save), the canvas is copied
first so the other holder keeps seeing the old pixels. See it in
[src/app/liquify.rs:320](../src/app/liquify.rs#L320):

```rust
let Some(c) = self.project.cells.get_mut(cell).map(Arc::make_mut) else { return };
```

## Threads and channels

The UI runs on one thread. Slow jobs (saving, exporting, decoding a video) run on worker
threads so the window stays responsive — similar to Web Workers, but sharing memory safely.

```rust
// src/app.rs — File → Import video
let (tx, rx) = mpsc::channel();          // a one-way pipe: tx sends, rx receives
let probe_path = path.clone();
thread::spawn(move || {                  // start a thread; `move` gives it probe_path and tx
    let _ = tx.send(crate::io::video_import::probe(&probe_path));
});
self.bg_job = Some(BgJob::VideoProbe { rx, path });   // keep rx; poll it every frame
```

```js
const worker = new Worker("probe.js");
worker.postMessage(path);
worker.onmessage = (e) => { /* result arrives later */ };
```

Every frame the UI calls `rx.try_recv()` (non-blocking): `Ok(result)` means the job finished.

Other tools you'll meet:

- `std::thread::scope(|s| { s.spawn(…); … })` — start threads that may borrow local data, all
  joined before `scope` returns. Used to split pixel work across CPU cores.
- `AtomicU64`, `AtomicBool` — numbers/flags that several threads can read and write safely
  without a lock (e.g. "cancel requested", "progress so far").
- `Send` / `Sync` — marker traits the compiler uses to check a type is safe to move to / share
  with another thread. You rarely write them; errors mention them.

## Macros

Names ending in `!` are **macros**: code that writes code at compile time. Common ones:

| Macro | Does |
|---|---|
| `println!("x = {}", x)` | print a line (`{}` = Display, `{:?}` = Debug, `{x}` = inline variable) |
| `format!(…)` | same formatting, returns a `String` (like a template literal) |
| `vec![a, b, c]` / `vec![0u8; n]` | make a `Vec` (the second makes `n` zeros) |
| `assert!(cond)`, `assert_eq!(a, b)` | panic if false / not equal (used in tests) |
| `panic!("msg")` | crash with a message (an unrecoverable error) |
| `unreachable!()` | "this can't happen" — panics if it does |
| `matches!(v, Pattern)` | true if `v` matches the pattern |
| `log::info!`, `log::warn!` | logging (shown in the terminal, filtered by `RUST_LOG`) |
| `include_bytes!("file")` | embeds a file's bytes into the program at compile time |

`macro_rules! name { … }` defines a project-specific macro. Treat a call to it like a function
call that the file page explains.

## Attributes and cfg

`#[...]` lines are **attributes**: annotations on the item that follows (like TS decorators).
`#![...]` (with `!`) applies to the whole enclosing file/crate.

| Attribute | Meaning |
|---|---|
| `#[derive(Clone, Debug, …)]` | auto-implement traits (see [Traits and derive](#traits-and-derive)) |
| `#[cfg(test)]` | compile this item only when running `cargo test` |
| `#[cfg(target_os = "windows")]` | compile only on Windows |
| `#[cfg(feature = "exact-rcp")]` | compile only when that Cargo feature is on |
| `#[test]` | this function is a test |
| `#[inline]` / `#[inline(always)]` | performance hint: paste the function body at call sites |
| `#[allow(dead_code)]` etc. | silence a compiler warning |
| `#[serde(default)]` | serde: if the field is missing in a file, use its default |
| `#[repr(C)]` | lay out the struct like C does (needed when talking to C / Windows APIs) |
| `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` | in release builds on Windows, don't open a console window |

`cfg!(…)` (with `!`) is the same test as an ordinary `true`/`false` expression in code.

## Constants and statics

```rust
const TEXTURE_BUDGET: usize = 512 * 1024 * 1024;   // compile-time constant, inlined where used
static NAME: &str = "Animator";                     // one fixed memory location for the program
```

- `const` — like a top-level JS `const` with a literal value; SCREAMING_CASE by convention.
- `static` — a global. Mutable globals need thread-safe wrappers.
- `OnceLock<T>` / `LazyLock<T>` — a global initialised the first time it's used (lazy singleton).
- `thread_local! { static X: RefCell<…> = … }` — a separate global per thread.

## Tests

Tests live in the same file as the code, in a module that only compiles for `cargo test`:

```rust
#[cfg(test)]
mod tests {
    use super::*;            // bring the parent module's items in scope

    #[test]
    fn area_multiplies() {
        assert_eq!(area(3, 4), 12);
    }
}
```

```js
// Jest equivalent
describe("area", () => {
  test("multiplies", () => { expect(area(3, 4)).toBe(12); });
});
```

Run them from the repo root:

```bash
cargo test                       # everything
```

```bash
cargo test tools::fill           # only tests whose path contains "tools::fill"
```

```bash
cargo test --release some_name   # optimised build (some heavy tests are faster this way)
```

A test passes if it returns without panicking. Each file page lists its tests in a table.

## Unsafe and FFI

Rust normally guarantees memory safety. **`unsafe { … }`** marks a block where the programmer
promises to uphold the rules the compiler can't check — almost always because the code is
calling into C or the operating system (**FFI**, "foreign function interface"), or reading raw
pointers.

```rust
#[repr(C)]                     // same memory layout as the C struct the driver expects
struct Packet { x: i32, y: i32, pressure: u32 }

unsafe {
    // call a function from a DLL loaded at run time (libloading crate)
    let n = (api.get_packets)(context, 64, buf.as_mut_ptr());
}
```

In this app, `unsafe` is concentrated in Windows-specific code: the pen-tablet driver
(`src/input/tablet.rs`), raw window messages (`src/input/msg_hook.rs`), window styling
(`src/platform.rs`) and some low-level pixel code. Think of it as the Rust version of calling
a native Node addon: it's where the safety net has a deliberate, small hole.

`*const T` / `*mut T` are raw pointers (like C pointers); `.as_ptr()` / `.as_mut_ptr()` get one
from a slice.

## Serde and file formats

[Serde](https://serde.rs) is the standard (de)serialisation library. You derive two traits and
can then convert a struct to and from many formats:

```rust
#[derive(Serialize, Deserialize)]
struct UiPrefs {
    theme: String,
    #[serde(default)]          // missing in an old file? use the default instead of failing
    brush_size: f32,
}
```

```js
const text = JSON.stringify(prefs);   // serialize
const prefs2 = JSON.parse(text);      // deserialize
```

Formats used in this project:

| Format | Crate | Used for |
|---|---|---|
| **postcard** | `postcard` | compact binary encoding inside `.anim` project files |
| **TOML** | `toml` | `shortcuts.toml`, the user's key bindings (human-editable) |
| **RON** | (via eframe) | `ui.ron`, saved window/panel layout and UI preferences |
| **XML** | `roxmltree` (read) | inside Krita `.kra` and `.kpp` files |
| **zip / deflate** | `zip`, `flate2` | `.kra` containers, compressing `.anim` payloads |

## egui and immediate mode UI

The user interface uses [egui](https://www.egui.rs/), an **immediate-mode** GUI library. This is
the biggest mental shift for a web developer, and it explains the shape of
[src/ui/shell.rs](../src/ui/shell.rs) and much of [src/app.rs](../src/app.rs).

**Retained mode (the DOM):** you create elements once, keep references to them, and update
them when state changes. React hides this behind a virtual DOM, but the DOM still exists.

**Immediate mode (egui):** there are no persistent widgets. **Every frame** (60+ times per
second while anything moves) the app calls a function that describes the whole UI from
scratch, and each widget call both *draws* the widget and *returns* what the user did to it
this frame.

```rust
// egui: called every frame
fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    ui.label(format!("Frame {}", state.project.current_frame + 1));
    if ui.button("Play").clicked() {          // draws the button AND reports a click
        state.playback.playing = true;        // just change state; next frame shows it
    }
    ui.add(egui::Slider::new(&mut state.opacity, 0.0..=1.0));  // edits the value in place
}
```

```jsx
// React: similar "describe the UI from state" idea…
function Draw({ state, setState }) {
  return (<>
    <span>Frame {state.frame + 1}</span>
    <button onClick={() => setState({ ...state, playing: true })}>Play</button>
  </>);
}
// …but React keeps a DOM and calls you back on events; egui calls YOU every frame
// and hands you the events as return values.
```

Things that follow from this:

- **No event handlers or callbacks** — `if ui.button(…).clicked() { … }` *is* the handler.
- **State lives in your own structs** (`AppState`), not in widgets. Widgets borrow it
  (`&mut state.opacity`) for the duration of the call.
- **Layout is code order**: `ui.horizontal(|ui| { … })` lays the closure's widgets out in a row,
  `ui.vertical(…)` in a column, `egui::Window::new("Title").show(ctx, |ui| { … })` makes a
  floating window. The closures are how egui nests layouts — like JSX children.
- A **`Response`** (returned by most widgets) carries `.clicked()`, `.hovered()`, `.dragged()`,
  `.drag_delta()`, `.on_hover_text("tooltip")`…
- `egui::Context` (`ctx`) is the global UI handle: input state (`ctx.input(|i| …)`), repaint
  requests (`ctx.request_repaint()`), textures, memory for panel positions.
- **`Painter`** draws shapes directly (lines, rects, images) — like a 2-D canvas context — used
  for the drawing canvas, onion skins, grids and cursors.

**eframe** is the small framework that opens the native window, runs the frame loop and calls
your app. The app implements eframe's `App` trait; its `update(&mut self, ctx, frame)` method is
the function called every frame — effectively the app's main loop. See
[architecture.md](architecture.md) for what `AppState::update` does each frame.

**wgpu** is the GPU layer underneath (a Rust implementation of the WebGPU API). egui renders
through it, and this app adds its own wgpu drawing for the canvas
([src/ui/canvas_gpu.rs](../src/ui/canvas_gpu.rs)).
