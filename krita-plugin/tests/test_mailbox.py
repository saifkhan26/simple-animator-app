"""Tests for the plugin's pure protocol module. Run from the repo root:

    python -m unittest discover -s krita-plugin/tests

`mailbox.py` is loaded by path: importing the `animator_link` package would
import Krita, which only exists inside Krita.
"""

import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "mailbox", os.path.join(HERE, "..", "animator_link", "mailbox.py")
)
mailbox = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mailbox)


class Format(unittest.TestCase):
    def test_round_trip(self):
        msg = {"seq": 3, "msg": "error", "text": "a=b c"}
        self.assertEqual(
            mailbox.parse(mailbox.format_msg(msg)),
            {"seq": "3", "msg": "error", "text": "a=b c"},
        )

    def test_newlines_in_values_stay_on_one_line(self):
        self.assertEqual(mailbox.parse(mailbox.format_msg({"text": "x\ny"}))["text"], "x y")

    def test_atomic_write_leaves_no_temp_file(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "a.kra.to-app")
            mailbox.write(p, {"seq": 1, "msg": "ready", "saved": 0})
            self.assertEqual(os.listdir(d), ["a.kra.to-app"])
            self.assertEqual(mailbox.read(p)["saved"], "0")
        self.assertIsNone(mailbox.read(os.path.join(d, "missing")))

    def test_linked_only_inside_the_link_folder(self):
        self.assertTrue(mailbox.is_linked(os.path.join("C:", "Temp", "animator-krita", "shot-1.kra")))
        self.assertFalse(mailbox.is_linked(os.path.join("C:", "Art", "shot.kra")))
        self.assertFalse(mailbox.is_linked(""))


class Rounds(unittest.TestCase):
    def test_clean_document_is_ready_at_once(self):
        s = mailbox.Session(None)
        self.assertEqual(s.next_action({"seq": "1", "msg": "request"}, False), ("ready", 1))

    def test_unsaved_edits_are_saved_first(self):
        s = mailbox.Session(None)
        self.assertEqual(s.next_action({"seq": "1", "msg": "request"}, True), ("save", 1))

    def test_each_message_is_handled_once(self):
        s = mailbox.Session(None)
        req = {"seq": "1", "msg": "request"}
        self.assertIsNotNone(s.next_action(req, False))
        self.assertIsNone(s.next_action(req, False))
        self.assertEqual(s.next_action({"seq": "1", "msg": "sent"}, False), ("reload", 1))
        self.assertIsNone(s.next_action({"seq": "1", "msg": "sent"}, False))
        self.assertEqual(s.next_action({"seq": "2", "msg": "request"}, False), ("ready", 2))

    def test_a_message_from_before_krita_started_is_not_acted_on(self):
        s = mailbox.Session({"seq": "4", "msg": "sent"})
        self.assertIsNone(s.next_action({"seq": "4", "msg": "sent"}, False))
        self.assertEqual(s.next_action({"seq": "5", "msg": "request"}, False), ("ready", 5))

    def test_stale_seq_is_ignored(self):
        s = mailbox.Session({"seq": "4", "msg": "request"})
        self.assertIsNone(s.next_action({"seq": "3", "msg": "sent"}, False))

    def test_nothing_waiting(self):
        self.assertIsNone(mailbox.Session(None).next_action(None, True))


if __name__ == "__main__":
    unittest.main()
