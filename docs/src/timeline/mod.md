# `src/timeline/mod.rs` — the timeline folder

> **Source:** [src/timeline/mod.rs](../../../src/timeline/mod.rs) · **Module:** `crate::timeline` · **Size:** 5 lines · Line links as of commit `67a7cb9`

## What this file is for

`src/timeline/mod.rs` is the folder's index file (like an `index.js` that only re-exports). It
declares the two modules in the folder and nothing else:

```rust
pub mod onion;     // src/timeline/onion.rs
pub mod playback;  // src/timeline/playback.rs
```

`pub mod` makes them reachable from the rest of the app as `crate::timeline::onion` and
`crate::timeline::playback`.

| Page | What it covers |
|---|---|
| [playback.md](playback.md) | The playback clock: which frame shows when you press Play, looping, camera moves that last several loops |
| [onion.md](onion.md) | Onion-skin settings: which neighbouring drawings show as tinted ghosts, how strong, pinned frames |

The timeline *UI* (frame ruler, layer tracks) is drawn elsewhere — see
[ui/tracks.rs](../ui/tracks.md) and [ui/shell.rs](../ui/shell.md). The timeline *data* (which drawing
is exposed on which frame) lives in the document model — see [doc/layer.rs](../doc/layer.md) and
[doc/project.rs](../doc/project.md).

## How it connects

- **Used by:** [app.rs](../app.md) (`AppState::playback`, `AppState::onion`), [ui/shell.rs](../ui/shell.md).

## Rust concepts in this file

**A `mod.rs` file is a folder's root module.** Declaring `pub mod onion;` here is what makes the
compiler read `onion.rs` at all. More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)
