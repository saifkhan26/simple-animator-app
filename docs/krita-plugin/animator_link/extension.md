# `krita-plugin/animator_link/extension.py` — the plugin running inside Krita

> **Source:** [krita-plugin/animator_link/extension.py](../../../krita-plugin/animator_link/extension.py) ·
> **Python module:** `animator_link.extension` · **Size:** 105 lines

## What this file is for

This is the part of the plugin that runs inside Krita. Every 400 ms it checks each open document that
Animator linked, reads Animator's mailbox message for it, and acts:

- **request** → if the document has unsaved edits, save it (so Animator can pull them), then reply
  `ready`;
- **sent** → Animator rewrote the file: **reload it in place** — open the new version in the same window,
  restore the frame, active layer, zoom, rotation and mirror, then close the old one — and reply
  `reloaded`.

Krita has no "reload from disk" command, which is why this plugin exists. Its Python API can't set the
pan, so the view re-centres after a reload. Any exception is reported back as an `error` reply rather than
leaving Animator waiting.

The protocol logic (what to do for which message) is in [mailbox.py](mailbox.md); this file does the Krita
calls.

## How it connects

- **Registered by:** [__init__.py](__init__.md).
- **Uses:** [mailbox.py](mailbox.md); Krita's Python API (`Krita`, `Extension`, documents, views,
  canvases); PyQt5 (`QTimer`, `QIcon`).
- **Talks to:** the app's [src/krita_link.rs](../../src/krita_link.md) via
  [src/krita_helper.rs](../../src/krita_helper.md)'s mailbox files.

## Python concepts in this file

**Subclassing.** `class AnimatorLink(Extension):` inherits Krita's `Extension` base class;
`super().__init__(parent)` calls the parent constructor — like `class AnimatorLink extends Extension {
constructor(parent) { super(parent); } }`.

**Qt timers and signals.** `self.timer.timeout.connect(self.tick)` subscribes `tick` to the timer's
`timeout` signal — like `setInterval(this.tick, 400)`.

**Tuples unpacked from a function.** `kind, seq = action` splits `("reload", 3)` into two variables.

## Constants and module functions

### `TICK_MS`
[krita-plugin/animator_link/extension.py:13](../../../krita-plugin/animator_link/extension.py#L13)

How often (milliseconds) the plugin checks the mailboxes: 400.

### `reply`
[krita-plugin/animator_link/extension.py:16](../../../krita-plugin/animator_link/extension.py#L16)

```python
def reply(path, fields)
```

Writes a reply for the document at `path` into `path + ".to-app"`.

## `class AnimatorLink(Extension)`
[krita-plugin/animator_link/extension.py:20](../../../krita-plugin/animator_link/extension.py#L20)

### `AnimatorLink.__init__`
[krita-plugin/animator_link/extension.py:21](../../../krita-plugin/animator_link/extension.py#L21)

```python
def __init__(self, parent)
```

Creates the extension with an empty map of sessions (one `mailbox.Session` per linked file, keyed by
`mailbox.key(path)`) and no timer yet.

### `AnimatorLink.setup`
[krita-plugin/animator_link/extension.py:26](../../../krita-plugin/animator_link/extension.py#L26)

```python
def setup(self)
```

Called by Krita once at startup: starts a repeating `QTimer` calling `tick` every `TICK_MS`.

### `AnimatorLink.createActions`
[krita-plugin/animator_link/extension.py:31](../../../krita-plugin/animator_link/extension.py#L31)

```python
def createActions(self, window)
```

Required by Krita's `Extension` interface (to add menu actions per window); this plugin adds none.

### `AnimatorLink.tick`
[krita-plugin/animator_link/extension.py:34](../../../krita-plugin/animator_link/extension.py#L34)

```python
def tick(self)
```

The polling loop body. For each open document whose file is linked:

1. Reads its `.to-krita` message. The first time a document is seen it only starts a `Session` (ignoring
   an old waiting message).
2. Asks the session for the next action, given whether the document is modified.
3. `ready` → reply ready (`saved=0`); `save` → `doc.save()` synchronously (the file is complete when it
   returns) and reply ready (`saved=1`); `reload` → `self.reload(doc)` and reply reloaded.
4. On any exception, replies `error` with the message.

It handles at most one document per tick, because a reload changes Krita's document list.

### `AnimatorLink.view_of`
[krita-plugin/animator_link/extension.py:66](../../../krita-plugin/animator_link/extension.py#L66)

```python
def view_of(self, doc)
```

Finds the window and view showing `doc` (comparing canonical paths); falls back to the active window and
no view.

### `AnimatorLink.reload`
[krita-plugin/animator_link/extension.py:75](../../../krita-plugin/animator_link/extension.py#L75)

```python
def reload(self, doc)
```

Reloads `doc` from disk in place:

1. Remembers the current frame, the active layer's unique id, and the canvas zoom, rotation and mirror.
2. Opens the file again as a new document; adds and shows a view of it in the same window *before* closing
   the old one, so the window never goes empty.
3. Restores the frame, active layer (found by unique id) and canvas pose.
4. Marks the old document unmodified (its edits already went to Animator) and closes it.
5. Shows a floating "Updated from Animator" message for 2 seconds.

Raises `RuntimeError` if Krita can't reopen the file (reported to Animator as an error).
