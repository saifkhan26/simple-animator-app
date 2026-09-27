"""The protocol between Animator and this plugin. Pure Python: no Krita
import, so it can be tested anywhere.

Animator links a document by writing it into a folder named
"animator-krita". Next to each linked file sit two small text files, one
"key=value" per line:

    <file>.to-krita   written by Animator:  seq=N, msg=request | sent
    <file>.to-app     written here:         seq=N, msg=ready | reloaded | error
                                            (saved=0|1 on ready, text=... on error)

One round per "Send to Krita again":
  request -> (save Krita's unsaved edits first if there are any) -> ready
  sent    -> reload the document in place -> reloaded
"""

import os

LINK_DIR = "animator-krita"


def parse(text):
    """`key=value` lines to a dict. Anything else is ignored."""
    out = {}
    for line in text.splitlines():
        key, sep, value = line.partition("=")
        if sep:
            out[key.strip()] = value.strip()
    return out


def format_msg(fields):
    # One line per field; newlines in a value would break the format.
    return "".join("%s=%s\n" % (k, str(v).replace("\n", " ")) for k, v in fields.items())


def read(path):
    """The message in `path`, or None when there is none (yet)."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            return parse(f.read())
    except OSError:
        return None


def write(path, fields):
    """Write atomically: the reader never sees half a message."""
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(format_msg(fields))
    os.replace(tmp, path)


def is_linked(path):
    """Whether a document's file is one Animator linked."""
    if not path:
        return False
    return os.path.basename(os.path.dirname(os.path.normpath(path))) == LINK_DIR


def key(path):
    """One spelling per file, whatever slashes or case Krita hands back."""
    return os.path.normcase(os.path.normpath(path))


def seq_of(msg):
    try:
        return int((msg or {}).get("seq", "0"))
    except ValueError:
        return 0


class Session:
    """Protocol state for one linked file: which messages were handled."""

    def __init__(self, current=None):
        # A message already waiting when the document is first seen belongs to
        # an earlier Krita run; answering it now would reload out of the blue.
        self.handled = seq_of(current)
        self.handled_msg = (current or {}).get("msg")

    def next_action(self, msg, doc_modified):
        """What to do about `msg`: ("ready", seq), ("save", seq),
        ("reload", seq), or None when there is nothing new."""
        if not msg:
            return None
        seq, kind = seq_of(msg), msg.get("msg")
        if (seq, kind) == (self.handled, self.handled_msg) or seq < self.handled:
            return None
        self.handled, self.handled_msg = seq, kind
        if kind == "request":
            return ("save", seq) if doc_modified else ("ready", seq)
        if kind == "sent":
            return ("reload", seq)
        return None
