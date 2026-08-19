#!/usr/bin/env python3
"""Focused tests for Zsh PTY readiness-marker framing."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from zsh_zle_pty import marker_seen


class MarkerSeenTests(unittest.TestCase):
    def test_setup_input_cannot_echo_the_complete_marker(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        setup = b"print -r -- '__HASHAI_PTY_'\"'READY__'"
        self.assertNotIn(marker, setup)

    def test_detects_marker_followed_by_prompt_bytes(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        seen, tail = marker_seen(b"", marker + b"\r\n% ", marker)
        self.assertTrue(seen)
        self.assertEqual(tail, b"")

    def test_detects_marker_split_across_reads(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        seen, tail = marker_seen(b"prefix __HASHAI_", b"PTY_READY__\r\n", marker)
        self.assertTrue(seen)
        self.assertEqual(tail, b"")
