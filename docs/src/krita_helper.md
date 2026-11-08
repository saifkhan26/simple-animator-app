# `src/krita_helper.rs` — installing the Krita plugin and the mailbox files

> **Source:** [src/krita_helper.rs](../../src/krita_helper.rs) · **Module:** `crate::krita_helper` · **Size:** 192 lines · Line links as of commit `67a7cb9`

## What this file is for

Krita can't reload a file that changed on disk. So for "Edit in Krita" round trips, the app ships a tiny
**Python plugin for Krita** ("Animator Link", in [`krita-plugin/`](../krita-plugin/animator_link/extension.md)).
This file:

1. **Installs** that plugin into Krita's user plugin folder (`pykrita`) — the plugin's files are embedded
   into the app binary at compile time with `include_str!`.
2. Implements the **mailbox**: the two programs talk through small text files next to the linked `.kra`:
   `name.kra.to-krita` (the app writes, the plugin reads) and `name.kra.to-app` (the plugin writes, the
   app reads). Each message is a few `key=value` lines, written atomically (to a `.tmp` file, then
   renamed) so neither side ever reads half a message.

```text
Animator                                   Krita plugin
  tell(seq, "request")  ─► foo.kra.to-krita ─►  save Krita's edits if any
                        ◄─ foo.kra.to-app   ◄─  "ready" (saved=1/0)
  rewrite foo.kra
  tell(seq, "sent")     ─► foo.kra.to-krita ─►  reload foo.kra in place
                        ◄─ foo.kra.to-app   ◄─  "reloaded"   (or "error")
```

`seq` numbers each round so stale replies are ignored.

## How it connects

- **Used by:** [krita_link.rs](krita_link.md) (sends `tell`, reads `reply` during re-send rounds),
  [app.rs](app.md) (`install_krita_helper`, menu checks `installed`/`outdated`).
- **Mirrors:** [krita-plugin/animator_link/mailbox.py](../krita-plugin/animator_link/mailbox.md) — the
  Python side of the same file protocol.

## Rust concepts in this file

**`include_str!` embeds files at compile time.**
```rust
const DESKTOP: &str = include_str!("../krita-plugin/animator_link.desktop");
```
The file's text becomes a string constant inside the executable, so installing the plugin needs no
separate files. More: [Macros](../rust-primer.md#macros)

**`anyhow::Context`.** `.with_context(|| format!("creating {}", dir.display()))?` adds a human-readable
description to an I/O error before returning it, so the error says *what* failed. More:
[Option and Result](../rust-primer.md#option-and-result)

**`OsString` for paths.** File names on Windows aren't guaranteed valid UTF-8, so path pieces are built with
`OsString` rather than `String`.

## Types

### `const VERSION`
[src/krita_helper.rs:17](../../src/krita_helper.rs#L17)

```rust
pub const VERSION: u32 = 1;
```

Bumped with any change to the plugin files; the installed `.desktop` carries it, so the menu can offer a reinstall.

### `const DESKTOP`
[src/krita_helper.rs:19](../../src/krita_helper.rs#L19)

```rust
const DESKTOP: &str = include_str!("../krita-plugin/animator_link.desktop");
```

The plugin's `.desktop` file text (Krita reads it to list the plugin), embedded with `include_str!`.

### `const FILES`
[src/krita_helper.rs:20](../../src/krita_helper.rs#L20)

```rust
const FILES: &[(&str, &str)] = &[
    ("animator_link/__init__.py", include_str!("../krita-plugin/animator_link/__init__.py")),
    ("animator_link/extension.py", include_str!("../krita-plugin/animator_link/extension.py")),
    ("animator_link/mailbox.py", include_str!("../krita-plugin/animator_link/mailbox.py")),
];
```

The plugin's Python files, each as (path inside the plugin folder, embedded text).

### `enum Reply`
[src/krita_helper.rs:112](../../src/krita_helper.rs#L112)

What the plugin answered.

| Variant | Meaning |
|---|---|
| `Ready { seq: u64, saved: bool }` | Krita is ready for the new file; `saved` = it saved edits first. |
| `Reloaded { seq: u64 }` | Krita reloaded the file for round `seq`. |
| `Error { seq: u64, text: String }` | The plugin hit an error in round `seq`; `text` says what. |

## Free functions

### `plugin_dir`
[src/krita_helper.rs:29](../../src/krita_helper.rs#L29)

```rust
pub fn plugin_dir() -> Option<PathBuf>
```

Krita's user plugin folder: `<data dir>/krita/pykrita` (`%APPDATA%\krita\pykrita` on Windows).

### `installed_version`
[src/krita_helper.rs:34](../../src/krita_helper.rs#L34)

```rust
pub fn installed_version(dir: &Path) -> Option<u32>
```

The plugin version installed in `dir`, read from the `X-Animator-Version=` line of its `.desktop` file.

### `installed`
[src/krita_helper.rs:43](../../src/krita_helper.rs#L43)

```rust
pub fn installed() -> bool
```

Whether any version of the plugin is installed (whether it is *enabled* in Krita only shows when it
answers).

### `outdated`
[src/krita_helper.rs:48](../../src/krita_helper.rs#L48)

```rust
pub fn outdated() -> bool
```

Whether the installed plugin is older than the one this app carries (the menu then offers a reinstall).

### `install_into`
[src/krita_helper.rs:53](../../src/krita_helper.rs#L53)

```rust
pub fn install_into(dir: &Path) -> Result<()>
```

Writes the plugin's `.desktop` file and Python files into `dir`, creating folders as needed.

### `install`
[src/krita_helper.rs:63](../../src/krita_helper.rs#L63)

```rust
pub fn install() -> Result<PathBuf>
```

Installs into Krita's user plugin folder and returns the folder.

### `sibling`
[src/krita_helper.rs:71](../../src/krita_helper.rs#L71)

```rust
fn sibling(kra: &Path, suffix: &str) -> PathBuf
```

The path `kra` with `suffix` appended to its full name (`foo.kra` + `.to-app` → `foo.kra.to-app`).

### `to_krita`
[src/krita_helper.rs:78](../../src/krita_helper.rs#L78)

```rust
pub fn to_krita(kra: &Path) -> PathBuf
```

The mailbox file this app writes for the plugin.

### `to_app`
[src/krita_helper.rs:83](../../src/krita_helper.rs#L83)

```rust
pub fn to_app(kra: &Path) -> PathBuf
```

The mailbox file the plugin writes for this app.

### `write_msg`
[src/krita_helper.rs:88](../../src/krita_helper.rs#L88)

```rust
pub fn write_msg(path: &Path, fields: &[(&str, String)]) -> Result<()>
```

Writes `key=value` lines to `path` atomically: write a `.tmp` sibling, then rename it over the target
(renames are atomic, so the reader sees the old or the new file, never half).

### `read_msg`
[src/krita_helper.rs:95](../../src/krita_helper.rs#L95)

```rust
pub fn read_msg(path: &Path) -> Option<HashMap<String, String>>
```

Reads a mailbox file into a map of keys to values; `None` if it can't be read.

### `tell`
[src/krita_helper.rs:107](../../src/krita_helper.rs#L107)

```rust
pub fn tell(kra: &Path, seq: u64, msg: &str) -> Result<()>
```

Sends the plugin a message for round `seq`: `"request"` (get ready, save your edits) or `"sent"` (the new
file is on disk — reload it).

### `reply`
[src/krita_helper.rs:120](../../src/krita_helper.rs#L120)

```rust
pub fn reply(kra: &Path) -> Option<Reply>
```

Reads the plugin's latest reply about `kra` as a `Reply`. Anything unreadable or unknown is `None` ("no
answer"), never a wrong answer.

## Tests

### `mod tests`

Run with `cargo test krita_helper::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`installs_every_file_with_its_version`](../../src/krita_helper.rs#L149) | Installs every file with its version. |
| [`junk_replies_read_as_no_answer`](../../src/krita_helper.rs#L163) | Whatever ends up in the plugin's reply file — junk, half a message, an unknown kind — reads as "no answer", never as a wrong one. |
| [`mailbox_round_trip_and_no_temp_left`](../../src/krita_helper.rs#L174) | Mailbox round trip and no temp left. |

**Test helpers** (used only by the tests above):

- [`tmp_dir`](../../src/krita_helper.rs#L135) — Tmp dir.
- [`plugin_says`](../../src/krita_helper.rs#L143) — Stand in for the plugin: write its reply.
