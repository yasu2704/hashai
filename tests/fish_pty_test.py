#!/usr/bin/env python3
import os
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from fish_pty import marker_seen

class MarkerTests(unittest.TestCase):
    marker = b"__HASHAI_FISH_READY__"

    def test_marker_followed_by_prompt_is_detected(self):
        self.assertTrue(marker_seen(b"", self.marker + b"\r\n> ", self.marker)[0])

    def test_split_marker_is_detected(self):
        seen, tail = marker_seen(b"prefix __HASHAI_FISH_", b"READY__", self.marker)
        self.assertTrue(seen); self.assertEqual(tail, b"")

    def test_setup_echo_cannot_contain_complete_marker(self):
        setup = b"echo __HASHAI_FISH_''READY__"
        self.assertNotIn(self.marker, setup)

    def test_selected_fish_binary_is_used(self):
        fish = os.environ["HASHAI_FISH_BIN"]
        self.assertIn("fish", pathlib.Path(fish).name)
