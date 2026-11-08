# `krita-plugin/tests/test_mailbox.py` — tests for the plugin's protocol

> **Source:** [krita-plugin/tests/test_mailbox.py](../../../krita-plugin/tests/test_mailbox.py) ·
> **Size:** 79 lines

## What this file is for

Unit tests (Python's built-in `unittest`, comparable to Jest) for [mailbox.py](../animator_link/mailbox.md).
Run from the repository root:

```bash
python -m unittest discover -s krita-plugin/tests
```

The test loads `mailbox.py` **by file path** (`importlib.util.spec_from_file_location`) instead of
`import animator_link.mailbox`, because importing the package would run `__init__.py`, which imports
`krita` — a module that only exists inside Krita.

## Python concepts in this file

**`unittest.TestCase`.** Each class groups tests (like `describe`); each method starting with `test_` is one
test (like `test(...)`); `self.assertEqual(a, b)` is `expect(a).toEqual(b)`.

**`tempfile.TemporaryDirectory()`** in a `with` block creates a folder that is deleted when the block ends.

## Tests

### `class Format`

| Test | What it checks |
|---|---|
| [`test_round_trip`](../../../krita-plugin/tests/test_mailbox.py#L23) | `format_msg` then `parse` gives back the fields (as strings), even when a value contains `=` and spaces. |
| [`test_newlines_in_values_stay_on_one_line`](../../../krita-plugin/tests/test_mailbox.py#L30) | A newline inside a value is turned into a space, so it can't break the format. |
| [`test_atomic_write_leaves_no_temp_file`](../../../krita-plugin/tests/test_mailbox.py#L33) | `write` leaves only the target file (no `.tmp`), `read` reads it back, and reading a missing file gives `None`. |
| [`test_linked_only_inside_the_link_folder`](../../../krita-plugin/tests/test_mailbox.py#L41) | Only files inside an `animator-krita` folder count as linked; an empty path doesn't. |

### `class Rounds`

| Test | What it checks |
|---|---|
| [`test_clean_document_is_ready_at_once`](../../../krita-plugin/tests/test_mailbox.py#L48) | A `request` for a document with no unsaved edits answers `ready` straight away. |
| [`test_unsaved_edits_are_saved_first`](../../../krita-plugin/tests/test_mailbox.py#L52) | A `request` for a modified document asks for a `save` first. |
| [`test_each_message_is_handled_once`](../../../krita-plugin/tests/test_mailbox.py#L56) | Seeing the same message again does nothing; `sent` after `request` reloads once; the next round's `request` is handled. |
| [`test_a_message_from_before_krita_started_is_not_acted_on`](../../../krita-plugin/tests/test_mailbox.py#L65) | A message already waiting when the document is first seen is ignored; the next new one is handled. |
| [`test_stale_seq_is_ignored`](../../../krita-plugin/tests/test_mailbox.py#L70) | A message with an older round number than one already handled is ignored. |
| [`test_nothing_waiting`](../../../krita-plugin/tests/test_mailbox.py#L74) | No message means no action. |
