# `krita-plugin/animator_link/mailbox.py` — the file protocol, Krita side

> **Source:** [krita-plugin/animator_link/mailbox.py](../../../krita-plugin/animator_link/mailbox.py) ·
> **Python module:** `animator_link.mailbox` · **Size:** 95 lines

## What this file is for

The Krita plugin and the app talk through two tiny text files beside each linked `.kra` (the app's
side is [src/krita_helper.rs](../../src/krita_helper.md)):

```text
<file>.to-krita   written by Animator:  seq=N, msg=request | sent
<file>.to-app     written by plugin:    seq=N, msg=ready | reloaded | error
                                        (saved=0|1 on ready, text=... on error)
```

One round per "Edit in Krita" re-send:

1. Animator writes `request` → the plugin saves Krita's unsaved edits (if any) → replies `ready`
   (`saved=1` if it saved).
2. Animator reads those edits, rewrites the file, writes `sent` → the plugin reloads the document in
   place → replies `reloaded`.

This module is **pure Python with no Krita import**, so its logic can be unit-tested anywhere
([tests/test_mailbox.py](../tests/test_mailbox.md)). [extension.py](extension.md) does the Krita parts.

## How it connects

- **Used by:** [extension.py](extension.md), [tests/test_mailbox.py](../tests/test_mailbox.md).
- **Mirrors:** [src/krita_helper.rs](../../src/krita_helper.md) (`write_msg`, `read_msg`, `tell`,
  `reply`) — the same format from the Rust side.

## Python concepts in this file

**`str.partition`.** `key, sep, value = line.partition("=")` splits at the first `=` into three parts;
`sep` is empty if there was no `=`. Like `const i = line.indexOf("=")` then slicing.

**`os.replace` for atomic writes.** Write to `path + ".tmp"`, then `os.replace(tmp, path)` — a rename that
replaces the target in one step, so a reader never sees half a file.

**Classes.** `class Session:` with `__init__(self, …)` (the constructor) and methods taking `self` — like a
JS `class` with `constructor` and explicit `this`.

## Constants

### `LINK_DIR`
[krita-plugin/animator_link/mailbox.py:19](../../../krita-plugin/animator_link/mailbox.py#L19)

```python
LINK_DIR = "animator-krita"
```

Linked documents are written into a folder with this name; that is how the plugin recognises them.

## Functions

### `parse`
[krita-plugin/animator_link/mailbox.py:22](../../../krita-plugin/animator_link/mailbox.py#L22)

```python
def parse(text)
```

Turns `key=value` lines into a dict (`{"seq": "3", "msg": "ready"}`); lines without `=` are ignored. Values
stay strings.

### `format_msg`
[krita-plugin/animator_link/mailbox.py:32](../../../krita-plugin/animator_link/mailbox.py#L32)

```python
def format_msg(fields)
```

The reverse: a dict to `key=value\n` lines. Newlines inside a value are replaced by spaces so a value can
never break the one-line-per-field format.

### `read`
[krita-plugin/animator_link/mailbox.py:37](../../../krita-plugin/animator_link/mailbox.py#L37)

```python
def read(path)
```

The message in `path` as a dict, or `None` if the file doesn't exist (yet) or can't be read (`OSError`).

### `write`
[krita-plugin/animator_link/mailbox.py:46](../../../krita-plugin/animator_link/mailbox.py#L46)

```python
def write(path, fields)
```

Writes a message atomically: to `path.tmp`, then `os.replace` onto `path`.

### `is_linked`
[krita-plugin/animator_link/mailbox.py:54](../../../krita-plugin/animator_link/mailbox.py#L54)

```python
def is_linked(path)
```

Whether a document's file was linked by Animator: its parent folder is named `animator-krita`. Empty paths
(unsaved documents) are not.

### `key`
[krita-plugin/animator_link/mailbox.py:61](../../../krita-plugin/animator_link/mailbox.py#L61)

```python
def key(path)
```

One canonical spelling per file (normalised slashes and, on Windows, case), so the same document is
recognised however Krita reports its path.

### `seq_of`
[krita-plugin/animator_link/mailbox.py:66](../../../krita-plugin/animator_link/mailbox.py#L66)

```python
def seq_of(msg)
```

The message's round number `seq` as an int; 0 when missing or not a number. `(msg or {})` treats `None` as
an empty dict.

## `class Session`
[krita-plugin/animator_link/mailbox.py:73](../../../krita-plugin/animator_link/mailbox.py#L73)

Protocol state for one linked file: which message was last handled, so each is acted on exactly once.

### `Session.__init__`
[krita-plugin/animator_link/mailbox.py:76](../../../krita-plugin/animator_link/mailbox.py#L76)

```python
def __init__(self, current=None)
```

Starts a session for a document just noticed. A message already waiting at that moment came from an
earlier Krita run; it is marked as handled so the plugin doesn't suddenly reload the document.

### `Session.next_action`
[krita-plugin/animator_link/mailbox.py:82](../../../krita-plugin/animator_link/mailbox.py#L82)

```python
def next_action(self, msg, doc_modified)
```

Decides what to do about message `msg`:

- `None` — nothing new (no message, the same one as last time, or an older `seq`);
- `("save", seq)` — a `request` while the document has unsaved edits;
- `("ready", seq)` — a `request` with nothing to save;
- `("reload", seq)` — a `sent`.

It records the message as handled before returning.

## Tests

See [tests/test_mailbox.py](../tests/test_mailbox.md).
