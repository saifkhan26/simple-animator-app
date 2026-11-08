# `src/brush_library/import.rs` — importing presets from an installed Krita

> **Source:** [src/brush_library/import.rs](../../../src/brush_library/import.rs) · **Module:** `crate::brush_library::import` · **Size:** 843 lines · Line links as of commit `67a7cb9`

## What this file is for

This file is the logic behind the **Import Krita brushes** window (the window itself is drawn
by [ui/krita_import.rs](../ui/krita_import.md)). It finds the brush presets in the user's
installed Krita, says for each one whether this app can paint it (and if not, why), and copies
the ones the user picks into the app's own brushes folder, where the
[brush library](mod.md) reads them.

**Where Krita keeps presets.** A preset is a `.kpp` file (a PNG with the brush settings inside
it as XML). Krita keeps them in two kinds of place:

- loose `.kpp` files in a `paintoppresets` folder — both in the user's resource folder and in
  Krita's own install;
- `.bundle` files, which are zip archives with `paintoppresets/` and `brushes/` folders inside.

A preset names its **tip** (the stamp image) by file name only, and Krita finds that file
wherever it is installed. So a preset in one bundle may use a tip from another bundle or from a
`brushes` folder. This file copies that search order.

**Where things are looked for on disk.**

| Place | How it is found |
|---|---|
| The user's resource folder | `ResourceDirectory=` in Krita's `kritarc` settings file (in the OS "local config" folder, e.g. `%LOCALAPPDATA%` on Windows, or the macOS preferences folder); otherwise `<OS data folder>/krita` (e.g. `%APPDATA%\krita`). Its `paintoppresets/` folder and the `.bundle` files directly in it are listed. |
| Krita's install | `share/krita` two folders up from the Krita executable (`…/bin/krita.exe` → `…/share/krita`). Its `paintoppresets/` folder and the bundles in `bundles/` are listed. |
| Files the user picks | `.bundle` files, or the folder of any picked `.kpp`. |

**What an import writes.** Importing copies the `.kpp` and the tip files it actually used into a
new folder `<brushes>/<name>/` (the library's brushes folder, `%APPDATA%\animator-app\brushes\`
on Windows), with a `source.toml` noting where it came from. Nothing is ever read live from
Krita afterwards. The folder is built under a hidden `.import-<process id>` name and renamed into
place at the end, so the library never sees half an import.

**Threads.** Listing means opening every preset and trying to load it, which is slow, so
[`scan`](#scan) does it on a background thread named `krita-import` and sends each preset back
as it is checked. The window calls [`KritaImport::poll`](#kritaimportpoll) each frame to collect
them. Adding files starts another scan beside the first.

## How it connects

- **Used by:**
  - [ui/krita_import.rs](../ui/krita_import.md) — the window: shows `KritaImport`'s groups and
    candidates, calls `poll`, `scanning`, `visible`, `import`, `add_files`.
  - [app.rs](../app.md) — `AppState::open_krita_import` calls `KritaPlaces::find` and
    `KritaImport::start`; `AppState` holds the `Option<KritaImport>`.
  - [brush_library/mod.rs](mod.md) — `save_as_new` uses `install`, `folder_name`, `hash` and
    `SourceInfo` to save an edited brush the same way an import is saved.
  - Tests in [ui/shell.rs](../ui/shell.md) and [ui/sweep_tests.rs](../ui/sweep_tests.md) use
    `KritaImport` and the `fake_krita`/`tmp` helpers.
- **Uses:**
  - [brush_library/mod.rs](mod.md) — `display_name`, `kpp_thumbnail`, `FolderTips`.
  - [tools/krita/mod.rs](../tools/krita/mod.md) — `PixelBrush::load` (does this preset paint?)
    and the `Resources` trait (where tips come from).
  - [tools/krita/defaults.rs](../tools/krita/defaults.md) — the shipped presets, to recognise
    ones that are already built in.
  - Crates: `zip` (reading bundles), `dirs` (OS folders), `toml` + `serde` (`source.toml`),
    `anyhow` (errors).

```text
 KritaImport::start(places, root)
   └─ plan(places) → groups (folders + bundles) ─▶ add_groups ─▶ scan(...) ──spawn──▶ "krita-import"
                                                                     per preset: read bytes, hash,
                                                                     skip duplicates, try PixelBrush::load
                                                                     with a Recording tip source,
                                                                     tx.send(Found(Candidate))
                                                                     ... tx.send(Done)
 window, each frame: poll() ◀───────────────── channel ──────────────┘
 user clicks Import: KritaImport::import(i) → import() → install() → <brushes>/<name>/
```

## Rust concepts in this file

**`Result<(), String>` as a plain value.** A `Result` does not have to be returned with `?`; it
can be stored. `Candidate::status` is `Ok(())` ("paints") or `Err(reason)`.
```rust
let status = PixelBrush::load(&bytes, &rec).map(|_| ()).map_err(|e| format!("{e:#}"));
```
```js
let status; try { loadBrush(bytes); status = { ok: true }; } catch (e) { status = { ok: false, why: String(e) }; }
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**An immediately-called closure to catch errors.** `install` wraps several `?` steps in a
closure that it calls at once, so any failure lands in one place where the temporary folder is
cleaned up — exactly a JS IIFE inside `try/catch`.
```rust
let written = (|| -> Result<()> {
    for (file, data) in &files { std::fs::write(tmp.join(file), data)?; }
    Ok(())
})();
if let Err(e) = written { std::fs::remove_dir_all(&tmp).ok(); return Err(e); }
```
```js
try { (() => { for (const [f, d] of files) fs.writeFileSync(path.join(tmp, f), d); })(); }
catch (e) { fs.rmSync(tmp, { recursive: true }); throw e; }
```
More: [Closures](../../rust-primer.md#closures)

**`RefCell` around a zip reader.** Reading a file out of a `ZipArchive` needs `&mut` (it moves
the file position), but the `Pool` of bundles is shared by `&` reference. Wrapping the archive
in `RefCell` lets `bundle_file(&self, …)` still call `borrow_mut()`. The `Recording` tip source
uses a `RefCell<Vec<…>>` the same way, to note which tips were used from inside a `&self`
method. More: [Smart pointers](../../rust-primer.md#smart-pointers)

**Threads and channels.** `scan` returns the `Receiver` end of a channel and moves everything
the thread needs into it with `move ||`. More:
[Threads and channels](../../rust-primer.md#threads-and-channels)

**`anyhow` errors.** `bail!("…")` returns an error at once; `.context("…")` /
`.with_context(|| …)` adds a sentence in front of an error ("the copy does not load: …").
More: [Option and Result](../../rust-primer.md#option-and-result)

**Saving a struct as TOML with serde.** `#[derive(serde::Serialize, serde::Deserialize)]` on
`SourceInfo` lets `toml::to_string(&info)` write it and `toml::from_str` read it back — like
`JSON.stringify` / `JSON.parse` for a fixed shape.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

## Types

### `struct KritaPlaces`
[src/brush_library/import.rs:30](../../../src/brush_library/import.rs#L30)

Where Krita keeps its resources on this machine. Derives `Default` (both `None`), which the
tests and `sweep_tests` use for "no Krita found".

| Field | Type | Meaning |
|---|---|---|
| `user` | `Option<PathBuf>` | The user's resource folder: kritarc's `ResourceDirectory`, else `<data>/krita`. |
| `install` | `Option<PathBuf>` | `share/krita` beside the Krita executable. |

### `enum Container`
[src/brush_library/import.rs:73](../../../src/brush_library/import.rs#L73)

Something that holds presets.

| Variant | Meaning |
|---|---|
| `Folder(PathBuf)` | A folder of `.kpp` files. |
| `Bundle(usize)` | A `.bundle`, by its index in the importer's bundle list. |

### `struct SourceGroup`
[src/brush_library/import.rs:82](../../../src/brush_library/import.rs#L82)

A heading in the import list: one folder or one bundle.

| Field | Type | Meaning |
|---|---|---|
| `label` | `String` | Heading text, e.g. "Your Krita presets", "Krita's install", or a bundle's name. |
| `path` | `PathBuf` | The folder or bundle file (shown on hover, recorded in `source.toml`). |
| `container` | `Container` | How to read presets out of it. |

### `enum Known`
[src/brush_library/import.rs:90](../../../src/brush_library/import.rs#L90)

Whether a preset is already in the app.

| Variant | Meaning |
|---|---|
| `New` | Not here yet: it can be imported. |
| `Imported` | A byte-identical `.kpp` is already in the brushes folder. |
| `BuiltIn` | It is one of the presets the app ships. |

### `struct Candidate`
[src/brush_library/import.rs:100](../../../src/brush_library/import.rs#L100)

One preset found in Krita: one row in the import window.

| Field | Type | Meaning |
|---|---|---|
| `group` | `usize` | Index of its `SourceGroup`. |
| `file` | `String` | The file name in a folder, or the entry path inside a bundle (`paintoppresets/x.kpp`). |
| `name` | `String` | Display name (see `display_name` in [mod.rs](mod.md)). |
| `status` | `Result<(), String>` | `Ok(())` if this app paints it, else the reason. |
| `hash` | `u64` | FNV-1a hash of the `.kpp` bytes; identical files are the same preset. |
| `known` | `Known` | New, imported, or built in. |
| `id` | `Option<String>` | The library entry it already is (`krita/…` or `user/…`), if built in or imported. |
| `thumb` | `Option<egui::ColorImage>` | The preset's own picture, scaled down. |

### `struct Bundle`
[src/brush_library/import.rs:121](../../../src/brush_library/import.rs#L121)

One opened `.bundle`.

| Field | Type | Meaning |
|---|---|---|
| `archive` | `RefCell<ZipArchive<File>>` | The open zip. |
| `tips` | `HashMap<String, String>` | Tip file name → its entry name in the zip (`gradient.png` → `brushes/gradient.png`). |

### `struct Pool`
[src/brush_library/import.rs:128](../../../src/brush_library/import.rs#L128)

Every place a tip can come from, opened once.

| Field | Type | Meaning |
|---|---|---|
| `folders` | `Vec<PathBuf>` | The `brushes` folders that exist: the user's, then the install's. |
| `bundles` | `Vec<Option<Bundle>>` | Each bundle in the importer's list, `None` for one that would not open. Indexed by `Container::Bundle`. |

### `struct Recording`
[src/brush_library/import.rs:215](../../../src/brush_library/import.rs#L215)

A tip source (`Resources`) that reads from a `Pool` and remembers every tip it handed out, so an
import can copy exactly those files.

| Field | Type | Meaning |
|---|---|---|
| `pool` | `&'a Pool` | Where tips come from (borrowed). |
| `own` | `Option<usize>` | The preset's own bundle, searched first. |
| `used` | `RefCell<Vec<(String, Vec<u8>)>>` | `(tip file name, bytes)` of every tip read. |

### `enum ScanMsg`
[src/brush_library/import.rs:285](../../../src/brush_library/import.rs#L285)

What the scan thread sends.

| Variant | Meaning |
|---|---|
| `Found(Candidate)` | One more preset was checked. |
| `Done` | The scan has finished. |

### `struct SourceInfo`
[src/brush_library/import.rs:411](../../../src/brush_library/import.rs#L411)

Where an imported preset came from; written as `source.toml` beside it. `pub(super)` means
only the parent module ([mod.rs](mod.md)) can see it.

| Field | Type | Meaning |
|---|---|---|
| `origin` | `String` | The folder or bundle path it was imported from, or `edited from <id>` for Save as new. |
| `file` | `String` | Its file name or bundle entry name there. |
| `hash` | `String` | The FNV hash as 16 hex digits. |

### `struct KritaImport`
[src/brush_library/import.rs:477](../../../src/brush_library/import.rs#L477)

The import window's state: what was found and the scans still running. Lives in
`AppState::krita_import` while the window is open.

| Field | Type | Meaning |
|---|---|---|
| `places` | `KritaPlaces` | Krita's folders. |
| `groups` | `Vec<SourceGroup>` | Every heading, in order. |
| `bundles` | `Vec<PathBuf>` | Paths of every bundle; `Container::Bundle(i)` indexes this. |
| `candidates` | `Vec<Candidate>` | Every preset found so far. |
| `scans` | `Vec<Receiver<ScanMsg>>` | Channels of scans still running. |
| `root` | `Option<PathBuf>` | The brushes folder imports go into. |
| `pool` | `Option<Pool>` | Tip sources for importing, opened on the first import and dropped when bundles are added. |
| `search` | `String` | The window's search text. |
| `show_unsupported` | `bool` | Also list presets that cannot paint. |
| `notice` | `Option<String>` | The last import's outcome, shown under the list. |
| `thumbs` | `HashMap<usize, egui::TextureHandle>` | Candidates' thumbnails as GPU textures, by candidate index, uploaded by the window as it draws them. |

## `impl KritaPlaces`

### `KritaPlaces::find`
[src/brush_library/import.rs:40](../../../src/brush_library/import.rs#L40)

```rust
pub fn find(krita_exe: Option<&Path>) -> Self
```

Finds Krita's two resource folders: the user's (from `kritarc`, else the OS data folder + `krita`)
and the install's (`<exe>/../../share/krita`). Each is kept only if the folder exists
(`.filter(|d| d.is_dir())`).

## Free functions

### `kritarc_resource_dir`
[src/brush_library/import.rs:54](../../../src/brush_library/import.rs#L54)

```rust
fn kritarc_resource_dir() -> Option<PathBuf>
```

Reads Krita's settings file `kritarc` (in the local config or preferences folder) and returns the
`ResourceDirectory` it names, if any. `[a, b].into_iter().flatten()` skips folders that are `None`;
`find_map` returns the first successful result.

### `resource_dir_in`
[src/brush_library/import.rs:62](../../../src/brush_library/import.rs#L62)

```rust
fn resource_dir_in(kritarc: &str) -> Option<PathBuf>
```

Pulls the `ResourceDirectory=` value out of kritarc's text (first matching line, trimmed, non-empty).

### `hash`
[src/brush_library/import.rs:115](../../../src/brush_library/import.rs#L115)

```rust
pub(super) fn hash(bytes: &[u8]) -> u64
```

A fast 64-bit fingerprint (FNV-1a) of a preset's bytes, so identical files found in two places count
as the same preset. `fold` is `reduce` with a starting value: for each byte, XOR it in then multiply
by the FNV prime (`wrapping_mul` lets the multiply overflow silently, as the algorithm requires).

## `impl Pool`

### `Pool::open`
[src/brush_library/import.rs:136](../../../src/brush_library/import.rs#L136)

```rust
pub fn open(places: &KritaPlaces, bundles: &[PathBuf]) -> Self
```

Opens every place presets and brush tips can come from: the `brushes` folders of the user and
install locations, and each `.bundle` file (a zip) opened once as a `ZipArchive`, with an index of
the brush-tip files it contains. A bundle that can't be opened becomes `None` in the list. The
archive is wrapped in a `RefCell` so reading from it (which needs `&mut`) works through a shared
`&Pool`.

### `Pool::bundle_file`
[src/brush_library/import.rs:161](../../../src/brush_library/import.rs#L161)

```rust
fn bundle_file(&self, b: usize, entry: &str) -> Option<Vec<u8>>
```

Reads one file (`entry`) out of bundle number `b` into a byte vector, or `None` if it isn't there.
`borrow_mut()` gets temporary write access through the `RefCell`.

### `Pool::bundle_presets`
[src/brush_library/import.rs:172](../../../src/brush_library/import.rs#L172)

```rust
fn bundle_presets(&self, b: usize) -> Vec<String>
```

The names of the `.kpp` preset files inside a bundle, sorted.

### `Pool::tip`
[src/brush_library/import.rs:189](../../../src/brush_library/import.rs#L189)

```rust
fn tip(&self, own: Option<usize>, filename: &str) -> Option<Vec<u8>>
```

Finds a brush-tip image the way Krita would: first in the preset's own bundle, then the `brushes`
folders, then any other bundle.

### `Pool::kpp`
[src/brush_library/import.rs:205](../../../src/brush_library/import.rs#L205)

```rust
fn kpp(&self, group: &SourceGroup, file: &str) -> Option<Vec<u8>>
```

Reads a candidate preset's `.kpp` bytes from its folder or its bundle.

## `impl Resources for Recording<'_>`

### `Recording::tip`
[src/brush_library/import.rs:222](../../../src/brush_library/import.rs#L222)

```rust
fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>
```

A tip lookup that also *records* what it returned (base file name + bytes) in `used`, so the import
knows exactly which tip files the preset needed and copies them along. It returns `Cow::Owned`
("clone on write" holding owned data) because the trait it implements allows borrowed or owned bytes.

## Free functions

### `own_bundle`
[src/brush_library/import.rs:230](../../../src/brush_library/import.rs#L230)

```rust
fn own_bundle(group: &SourceGroup) -> Option<usize>
```

The bundle a source group lives in, if it is a bundle (not a folder).

### `bundle_label`
[src/brush_library/import.rs:238](../../../src/brush_library/import.rs#L238)

```rust
fn bundle_label(path: &Path) -> String
```

A display name for a bundle file: its file name without extension, underscores turned into spaces.

### `plan`
[src/brush_library/import.rs:244](../../../src/brush_library/import.rs#L244)

```rust
pub fn plan(places: &KritaPlaces) -> (Vec<SourceGroup>, Vec<PathBuf>)
```

Lists what a scan will read: Krita's folders and bundles, the user's first; a bundle with the same
file name in both places is listed once. Returns the source groups and the bundle paths.

### `known_hashes`
[src/brush_library/import.rs:292](../../../src/brush_library/import.rs#L292)

```rust
fn known_hashes(root: Option<&Path>) -> (HashMap<u64, String>, HashMap<u64, String>)
```

Hashes of the presets the app ships with and of those already imported into `root`, mapped to their
library ids — so the import window can mark presets "already have".

### `scan`
[src/brush_library/import.rs:317](../../../src/brush_library/import.rs#L317)

```rust
pub fn scan(
    places: KritaPlaces,
    groups: Vec<SourceGroup>,
    first: usize,
    bundles: Vec<PathBuf>,
    root: Option<PathBuf>,
    mut seen: HashSet<u64>,
) -> Receiver<ScanMsg>
```

Starts a background thread that lists the presets in `groups[first..]` (reading each `.kpp`,
hashing it, checking whether it can be painted, making a thumbnail) and sends results back over a
channel. It returns the `Receiver` end; `KritaImport::poll` reads it each frame. Presets identical to
ones already listed are skipped.

### `folder_name`
[src/brush_library/import.rs:388](../../../src/brush_library/import.rs#L388)

```rust
pub(super) fn folder_name(name: &str) -> String
```

Turns a preset name into a safe folder name: no path separators or characters Windows forbids, and
not a reserved device name like `CON` or `NUL`.

### `import`
[src/brush_library/import.rs:419](../../../src/brush_library/import.rs#L419)

```rust
pub fn import(pool: &Pool, group: &SourceGroup, c: &Candidate, root: &Path) -> Result<String>
```

Imports one preset: reads its `.kpp`, loads it once to discover which tip files it uses, and installs
the preset plus those tips into a new folder under `root`. Returns the new library id.

### `install`
[src/brush_library/import.rs:442](../../../src/brush_library/import.rs#L442)

```rust
pub(super) fn install(root: &Path, name: &str, files: Vec<(String, Vec<u8>)>, info: SourceInfo) -> Result<String>
```

Writes a preset's files (the `.kpp` first) and a `source.toml` describing where it came from into a
new folder under `root`. The folder is built under a temporary name and renamed into place at the
end, so the library never sees half a preset, and it must load successfully before the rename.

## `impl KritaImport`

### `KritaImport::start`
[src/brush_library/import.rs:495](../../../src/brush_library/import.rs#L495)

```rust
pub fn start(places: KritaPlaces, root: Option<PathBuf>) -> Self
```

Opens the import window's state and starts the first background scan of Krita's presets. Imports go
into `root` (the app's brushes folder).

### `KritaImport::add_groups`
[src/brush_library/import.rs:514](../../../src/brush_library/import.rs#L514)

```rust
fn add_groups(&mut self, groups: Vec<SourceGroup>, bundles: Vec<PathBuf>)
```

Adds more source groups (folders/bundles) to the import window and starts a scan of just the new ones.
Bundle indexes in the new groups are shifted by how many bundles were already known, so they keep
pointing at the right file; already-listed hashes are passed in so duplicates are skipped.

### `KritaImport::add_files`
[src/brush_library/import.rs:537](../../../src/brush_library/import.rs#L537)

```rust
pub fn add_files(&mut self, paths: &[PathBuf])
```

Adds `.kpp` files and `.bundle` files the user picked with "Add files…".

### `KritaImport::poll`
[src/brush_library/import.rs:570](../../../src/brush_library/import.rs#L570)

```rust
pub fn poll(&mut self) -> bool
```

Called every frame while the window is open: takes in whatever the background scans have found so
far (non-blocking). Returns whether anything changed so the UI can repaint.

### `KritaImport::scanning`
[src/brush_library/import.rs:588](../../../src/brush_library/import.rs#L588)

```rust
pub fn scanning(&self) -> bool
```

Whether any scan is still running.

### `KritaImport::import`
[src/brush_library/import.rs:593](../../../src/brush_library/import.rs#L593)

```rust
pub fn import(&mut self, i: usize) -> Result<String>
```

Imports candidate `i` (opening the `Pool` on first use with `get_or_insert_with`), marks it imported
and returns its new library id. Errors (`anyhow!`) if there is no brushes folder or no such candidate.

### `KritaImport::visible`
[src/brush_library/import.rs:604](../../../src/brush_library/import.rs#L604)

```rust
pub fn visible(&self, group: usize) -> Vec<usize>
```

The candidates group `group` should show under the current search text and filter.

## Tests

### `mod tests`

Run with `cargo test brush_library::import::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`kritarc_names_the_resource_folder`](../../../src/brush_library/import.rs#L694) | Kritarc names the resource folder. |
| [`folder_names_are_safe`](../../../src/brush_library/import.rs#L701) | Folder names are safe. |
| [`the_scan_lists_folders_and_bundles_and_says_what_paints`](../../../src/brush_library/import.rs#L711) | The scan lists folders and bundles and says what paints. |
| [`importing_copies_the_preset_and_its_tips`](../../../src/brush_library/import.rs#L739) | Importing copies the preset and its tips. |
| [`picked_files_join_the_list`](../../../src/brush_library/import.rs#L791) | Picked files join the list. |
| [`krita_import_survey`](../../../src/brush_library/import.rs#L812) | What this machine's Krita offers, per folder or bundle.  ```text cargo test krita_import_survey -- --ignored --nocapture ``` |

**Test helpers** (used only by the tests above):

- [`tmp`](../../../src/brush_library/import.rs#L622) — Tmp.
- [`bundle`](../../../src/brush_library/import.rs#L629) — Bundle.
- [`edited`](../../../src/brush_library/import.rs#L639) — A shipped preset, changed so it is not the shipped one.
- [`fake_krita`](../../../src/brush_library/import.rs#L649) — A Krita resource folder with: - `paintoppresets/`: the shipped Basic-2 (built in), a hairy brush (refused), and My Rake, whose tip is in `brushes/`; - `Pencils.bundle`: My Grain, whose `.gih` tip is in the bundle, and the shipped Pencil-5.
- [`settle`](../../../src/brush_library/import.rs#L677) — Settle.
- [`find`](../../../src/brush_library/import.rs#L685) — Find.
