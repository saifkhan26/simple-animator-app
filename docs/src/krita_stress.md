# `src/krita_stress.rs` — stress tests for the Krita link (test-only)

> **Source:** [src/krita_stress.rs](../../src/krita_stress.rs) · **Module:** `crate::krita_stress` · **Size:** 960 lines · Line links as of commit `1fa5845`

## What this file is for

This whole file is **test code**: `src/main.rs` declares it as `#[cfg(test)] mod krita_stress;`, so it is
compiled only for `cargo test` and never ships in the app.

It hammers the "Edit in Krita" link ([krita_link.rs](krita_link.md)) with **random editing sessions**
in which *both* sides edit: the app side and a simulated Krita side each paint, retime, add/delete/reorder/
rename layers, toggle reference layers, add Krita-only `-x` layers, grow the timeline… then Krita "saves"
(a real `.kra` file is written with the app's own Krita writer) and the app pulls the changes in, sometimes
re-sending. After every step it checks that both sides agree on what each layer shows on each frame. A
seeded random generator makes every failing session reproducible from its seed.

It also has **scale** tests timing send/pull on big projects (a 2160×3840 shot, a 192-frame timeline).

```bash
cargo test krita_stress
```

```bash
STRESS_SEEDS=2000 STRESS_ROUNDS=40 cargo test --release krita_stress::random_sessions
```

(The environment variables scale the random test up.)

## How it connects

- **Uses:** [krita_link.rs](krita_link.md) (`prepare`, `plan`, `present`, baselines, hashes),
  [io/kra/mod.rs](io/kra/mod.md) (reading/writing `.kra`), [doc/project.rs](doc/project.md).
- **Used by:** nobody — tests only.

## Rust concepts in this file

**A hand-written random number generator.** `Rng` is *xorshift64\**: a few shifts and XORs on a `u64` per
call. Deterministic (same seed → same sequence) and dependency-free — what you want in tests. JS
equivalent: a seeded PRNG instead of `Math.random()`.

**`macro_rules!` for assertions with context.** The file defines a `check!` macro: like `assert!`, but on
failure it prints the session's log so the failing sequence of edits can be read back.
More: [Macros](../rust-primer.md#macros)

**Catching panics in a test.** `panic_text` turns a caught panic payload
(`Box<dyn Any + Send>`) into its message, so one failing seed can be reported with its seed number.

## Types

### `struct Rng`
[src/krita_stress.rs:29](../../src/krita_stress.rs#L29)

A tiny deterministic random number generator (xorshift64\*). The single field is its state.

| Field | Type | Meaning |
|---|---|---|
| `0` | `u64` |  |

### `const W`
[src/krita_stress.rs:55](../../src/krita_stress.rs#L55)

```rust
const W: u32 = 16;
```

Test canvas width (16 px) — tiny, so thousands of rounds run fast.

### `const H`
[src/krita_stress.rs:56](../../src/krita_stress.rs#L56)

```rust
const H: u32 = 12;
```

Test canvas height (12 px).

### `const FRAMES`
[src/krita_stress.rs:57](../../src/krita_stress.rs#L57)

```rust
const FRAMES: usize = 8;
```

Starting timeline length for a session (8).

### `struct Krita`
[src/krita_stress.rs:94](../../src/krita_stress.rs#L94)

The simulated Krita side: its document as a `Project`, and the Krita layer uuid of each layer.

| Field | Type | Meaning |
|---|---|---|
| `p` | `Project` | Krita's document, held as a project. |
| `uuids` | `Vec<String>` | Each layer's Krita uuid, in stack order. |

### `struct App`
[src/krita_stress.rs:177](../../src/krita_stress.rs#L177)

The simulated app side: its project and the link state the real app keeps.

| Field | Type | Meaning |
|---|---|---|
| `p` | `Project` | The app's project. |
| `links` | `HashMap<u64, String>` | Which Krita uuid each app layer (by uid) is linked to. |
| `base` | `Baseline` | The baseline: what both sides last agreed on. |
| `path` | `PathBuf` | The linked `.kra` file. |

### `struct Dirty`
[src/krita_stress.rs:220](../../src/krita_stress.rs#L220)

What changed on each side since they last agreed — used to know what the checks should expect.

| Field | Type | Meaning |
|---|---|---|
| `local_content` | `HashSet<u64>` | App layers whose drawings changed. |
| `local_attr` | `HashSet<u64>` | App layers whose name/opacity/flags changed. |
| `local_order` | `bool` | Whether the app reordered layers. |
| `krita_content` | `HashSet<String>` | Krita layers (by uuid) whose drawings changed. |
| `krita_attr` | `HashSet<String>` | Krita layers whose attributes changed. |
| `krita_unsaved` | `bool` | Whether Krita has edits not yet saved to the file. |

### `struct Session`
[src/krita_stress.rs:229](../../src/krita_stress.rs#L229)

One random two-sided editing session.

| Field | Type | Meaning |
|---|---|---|
| `seed` | `u64` | The seed that reproduces it. |
| `round` | `usize` | Current round number. |
| `rng` | `Rng` | Its random generator. |
| `app` | `App` | The app side. |
| `krita` | `Krita` | The Krita side. |
| `d` | `Dirty` | What changed since the last agreement. |
| `stats` | `Stats` | Counters for the summary. |
| `log` | `Vec<String>` | Every step, for a failure report. |

### `struct Stats`
[src/krita_stress.rs:242](../../src/krita_stress.rs#L242)

Counters of what happened across sessions (pulls, no-op pulls, sends, new drawings, unlinks, carried `-x`
layers, refused pulls), printed at the end.

| Field | Type | Meaning |
|---|---|---|
| `pulls` | `usize` | Pulls applied. |
| `noops` | `usize` | Pulls that changed nothing. |
| `sends` | `usize` | Re-sends. |
| `new_cells` | `usize` | New drawings created by pulls. |
| `unlinks` | `usize` | Layers unlinked (deleted on one side). |
| `carried` | `usize` | Krita-only `-x` layers carried through a re-send. |
| `refused` | `usize` | Pulls the link refused (as designed). |

### `macro_rules! check`
[src/krita_stress.rs:252](../../src/krita_stress.rs#L252)

`check!(session, condition, "message", …)` — asserts like `assert!`, but on failure panics with the seed,
round and the whole step log so the failure can be replayed.

## `impl Rng`

### `Rng::new`
[src/krita_stress.rs:32](../../src/krita_stress.rs#L32)

```rust
fn new(seed: u64) -> Self
```

Seeds the generator (scrambling the seed and forcing it odd so it is never zero).

### `Rng::next`
[src/krita_stress.rs:35](../../src/krita_stress.rs#L35)

```rust
fn next(&mut self) -> u64
```

The next pseudo-random 64-bit number (xorshift: three shift-XOR steps, then a multiply).

### `Rng::below`
[src/krita_stress.rs:43](../../src/krita_stress.rs#L43)

```rust
fn below(&mut self, n: usize) -> usize
```

A random number in `0..n` (0 if `n` is 0).

### `Rng::chance`
[src/krita_stress.rs:50](../../src/krita_stress.rs#L50)

```rust
fn chance(&mut self, pct: u64) -> bool
```

True with probability `pct` percent.

## Free functions

### `doodle`
[src/krita_stress.rs:59](../../src/krita_stress.rs#L59)

```rust
fn doodle(rng: &mut Rng) -> Canvas
```

A small random drawing: 1–6 random pixels on a blank 16×12 canvas.

### `scribble`
[src/krita_stress.rs:69](../../src/krita_stress.rs#L69)

```rust
fn scribble(c: &mut Canvas, rng: &mut Rng)
```

Paints one random pixel on `c` (an "edit").

### `blank`
[src/krita_stress.rs:74](../../src/krita_stress.rs#L74)

```rust
fn blank(c: &Canvas) -> bool
```

Whether a canvas is entirely transparent.

### `shows`
[src/krita_stress.rs:79](../../src/krita_stress.rs#L79)

```rust
fn shows(p: &Project, l: &Layer, frames: usize) -> Vec<u64>
```

What a layer shows on each frame, as a content hash per frame (0 for nothing) — the thing the checks
compare between the two sides.

### `attrs`
[src/krita_stress.rs:88](../../src/krita_stress.rs#L88)

```rust
fn attrs(l: &Layer) -> (String, u8, bool, bool)
```

A layer's checked attributes: name, opacity (as Krita's 0–255), visible, locked.

## `impl Krita`

### `Krita::open`
[src/krita_stress.rs:101](../../src/krita_stress.rs#L101)

```rust
fn open(path: &Path) -> Krita
```

What Krita would hold after opening the `.kra` at `path` (read with the app's reader, layer uuids kept).

### `Krita::save`
[src/krita_stress.rs:136](../../src/krita_stress.rs#L136)

```rust
fn save(&self, path: &Path)
```

Writes Krita's document back to the `.kra` file, as a Krita save would.

### `Krita::index_of`
[src/krita_stress.rs:141](../../src/krita_stress.rs#L141)

```rust
fn index_of(&self, uuid: &str) -> Option<usize>
```

The stack index of the layer with this uuid.

### `Krita::stays`
[src/krita_stress.rs:148](../../src/krita_stress.rs#L148)

```rust
fn stays(&self, i: usize) -> bool
```

Whether Krita layer `i` stays only in Krita (a `-x` layer, or inside a `-x` group).

### `Krita::stack`
[src/krita_stress.rs:155](../../src/krita_stress.rs#L155)

```rust
fn stack(&self) -> String
```

The layer stack as a compact string for failure messages (`[` opens a group's children, `]` is its header).

### `Krita::restack`
[src/krita_stress.rs:168](../../src/krita_stress.rs#L168)

```rust
fn restack(&mut self, edit: impl FnOnce(&mut Project))
```

Runs an edit that reorders Krita's stack, carrying each layer's uuid with it; layers the edit creates get
fresh uuids, as Krita would give them.

## `impl App`

### `App::send`
[src/krita_stress.rs:186](../../src/krita_stress.rs#L186)

```rust
fn send(&mut self) -> kl::Sent
```

The app's send (as `AppState::start_send` does, without the worker thread): writes the `.kra` and updates
links and baseline.

### `App::try_plan`
[src/krita_stress.rs:202](../../src/krita_stress.rs#L202)

```rust
fn try_plan(&self) -> anyhow::Result<kl::PullPlan>
```

Reads the `.kra` and computes the pull plan, as the app's worker and UI thread would.

### `App::plan`
[src/krita_stress.rs:209](../../src/krita_stress.rs#L209)

```rust
fn plan(&self) -> kl::PullPlan
```

`try_plan`, panicking on error.

### `App::linked`
[src/krita_stress.rs:213](../../src/krita_stress.rs#L213)

```rust
fn linked(&self, uuid: &str) -> Option<usize>
```

The app layer linked to a Krita uuid, if any.

## `impl Session`

### `Session::new`
[src/krita_stress.rs:269](../../src/krita_stress.rs#L269)

```rust
fn new(seed: u64, dir: &Path) -> Session
```

Starts a session from `seed` in `dir`: a random small project on the app side, sent to Krita, both sides
agreeing.

### `Session::local_edit`
[src/krita_stress.rs:306](../../src/krita_stress.rs#L306)

```rust
fn local_edit(&mut self)
```

One random edit on the app side (paint, retime, new drawing, add/delete/reorder/rename a layer, toggle
reference, grow timeline), recorded in `d` and the log.

### `Session::krita_edit`
[src/krita_stress.rs:389](../../src/krita_stress.rs#L389)

```rust
fn krita_edit(&mut self)
```

One random edit on the Krita side (same kinds, plus `-x` layers), recorded likewise.

### `Session::save_and_pull`
[src/krita_stress.rs:489](../../src/krita_stress.rs#L489)

```rust
fn save_and_pull(&mut self)
```

Krita saves; the app pulls.

### `Session::pull`
[src/krita_stress.rs:496](../../src/krita_stress.rs#L496)

```rust
fn pull(&mut self)
```

The app pulls Krita's save and then checks everything: every linked layer shows on each side what the
rules say it should (Krita's edits win where only Krita changed, the app's where only the app changed,
etc.), new Krita layers arrive, deleted ones unlink, `-x` layers stay in Krita.

### `Session::resend`
[src/krita_stress.rs:678](../../src/krita_stress.rs#L678)

```rust
fn resend(&mut self)
```

The app re-sends; checks that Krita's reopened document matches the app and that Krita-only layers were
carried through.

### `Session::run`
[src/krita_stress.rs:794](../../src/krita_stress.rs#L794)

```rust
fn run(&mut self, rounds: usize)
```

Runs `rounds` rounds of random edits, pulls and re-sends.

## Free functions

### `scratch`
[src/krita_stress.rs:813](../../src/krita_stress.rs#L813)

```rust
fn scratch(tag: &str) -> PathBuf
```

A fresh temporary directory for a test.

### `panic_text`
[src/krita_stress.rs:820](../../src/krita_stress.rs#L820)

```rust
fn panic_text(e: &Box<dyn std::any::Any + Send>) -> String
```

The message text inside a caught panic.

### `blob`
[src/krita_stress.rs:864](../../src/krita_stress.rs#L864)

```rust
fn blob(w: u32, h: u32, seed: u64, frac: f32) -> Canvas
```

A drawing-like test cell: a filled blob with a soft edge covering `frac` of the canvas.

### `scale`
[src/krita_stress.rs:884](../../src/krita_stress.rs#L884)

```rust
fn scale(tag: &str, w: u32, h: u32, frames: usize, drawings: &[usize])
```

A timing test: send, pull unchanged, Krita edits one drawing, pull, re-send carrying a full-size `-x` layer —
each step timed and printed.

## Tests

### Tests in this file

Run with `cargo test krita_stress` · 3 tests

| Test | What it checks |
|---|---|
| [`random_sessions`](../../src/krita_stress.rs#L826) | Seeded two-sided sessions. `STRESS_SEEDS` / `STRESS_ROUNDS` scale it up. |
| [`scale_users_shot`](../../src/krita_stress.rs#L950) | The user's shot: 2160x3840, six layers, one with seven drawings. |
| [`scale_long_timeline`](../../src/krita_stress.rs#L957) | A long timeline at 720p: three layers of 48 drawings over 192 frames. |
