#!/usr/bin/env python3
import errno
import os
import pathlib
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from fish_pty import ProbeResponder, marker_seen, terminal_responses, write_all

class MarkerTests(unittest.TestCase):
    marker = b"__HASHAI_FISH_READY__"

    def test_marker_followed_by_prompt_is_detected(self):
        self.assertTrue(marker_seen(b"", self.marker + b"\r\n> ", self.marker)[0])

    def test_split_marker_is_detected(self):
        seen, tail = marker_seen(b"prefix __HASHAI_FISH_", b"READY__", self.marker)
        self.assertTrue(seen); self.assertEqual(tail, b"")

    def test_setup_echo_cannot_contain_complete_marker(self):
        setup = b"echo '__HASHAI_FISH_'READY__"
        self.assertNotIn(self.marker, setup)

    def test_split_marker_command_emits_exact_marker(self):
        import subprocess
        command = "echo '__HASHAI_FISH_'READY__"
        output = subprocess.run(
            [os.environ["HASHAI_FISH_BIN"], "--no-config", "-c", command],
            check=True, capture_output=True, text=True,
        )
        self.assertEqual(output.stdout, "__HASHAI_FISH_READY__\n")

    def test_selected_fish_binary_is_used(self):
        fish = os.environ["HASHAI_FISH_BIN"]
        self.assertIn("fish", pathlib.Path(fish).name)

    def test_terminal_probe_replies_are_deterministic(self):
        stream = b"\x1b[0c\x1b[?u\x1b]11;?\x1b\\\x1b[>0q\x1bP+q696e646e\x1b\\"
        replies = terminal_responses(stream)
        self.assertEqual(replies, [b"\x1b[?1;2c", b"\x1b[?0u", b"\x1b]11;rgb:0000/0000/0000\x1b\\", b"\x1bP>|hashai\x1b\\", b"\x1bP0+r696e646e\x1b\\"])

    def test_split_probe_replies_once(self):
        stream = b"\x1b[0c\x1bP+q696e646e\x1b\\"
        for point in range(len(stream) + 1):
            responder = ProbeResponder()
            actual = responder.responses(stream[:point]) + responder.responses(stream[point:])
            self.assertEqual(actual, [b"\x1b[?1;2c", b"\x1bP0+r696e646e\x1b\\"])

    def test_second_probe_wave_and_cpr_reply(self):
        responder = ProbeResponder()
        first = responder.responses(b"\x1b[0c\x1b]11;?")
        second = responder.responses(b"\x1b]11;?\x1b[6n\x1b[0c")
        self.assertEqual(first, [b"\x1b[?1;2c", b"\x1b]11;rgb:0000/0000/0000\x1b\\"])
        self.assertEqual(second, [b"\x1b[?1;2c", b"\x1b]11;rgb:0000/0000/0000\x1b\\", b"\x1b[1;1R"])

    def test_write_all_retries_eagain_and_partial_write(self):
        writes = [BlockingIOError(errno.EAGAIN, "again"), 1, 2]
        with patch("fish_pty.os.write", side_effect=writes), patch("fish_pty.select.select", return_value=([], [9], [])):
            write_all(9, b"abc")

    def test_terminal_replies_do_not_satisfy_marker(self):
        data = b"".join(terminal_responses(b"\x1b[c"))
        self.assertFalse(marker_seen(b"", data, self.marker)[0])
