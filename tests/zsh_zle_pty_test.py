#!/usr/bin/env python3
"""Focused tests for Zsh PTY readiness-marker framing."""

import pathlib
import os
import subprocess
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from zsh_zle_pty import marker_seen


class MarkerSeenTests(unittest.TestCase):
    def test_keymap_selection_precedes_artifact_installation(self) -> None:
        harness = (pathlib.Path(__file__).parent / "zsh_zle_integration.sh").read_text()
        self.assertLess(harness.index("bindkey -$keymap"), harness.index("source '$artifact_path'"))

    def test_widget_uses_the_install_time_tty_gate(self) -> None:
        artifact = (pathlib.Path(__file__).parent.parent / "shell" / "hashai.zsh").read_text()
        widget = artifact.split("__hashai_zsh_replace_buffer()", 1)[1].split(
            "__hashai_zsh_install_binding()", 1
        )[0]
        installer = artifact.split("__hashai_zsh_install_binding()", 1)[1]
        self.assertIn("[[ ${__hashai_zsh_zle_enabled:-} == 1 ]] || return 0", widget)
        self.assertLess(
            installer.index("[[ -o interactive && -t 0 && -t 1 && -t 2 ]] || return 0"),
            installer.index("typeset -g __hashai_zsh_zle_enabled=1"),
        )

    def test_setup_input_cannot_echo_the_complete_marker(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        setup = b"print -r -- '__HASHAI_PTY_''READY__'"
        self.assertNotIn(marker, setup)

    def test_setup_command_emits_the_exact_marker(self) -> None:
        marker = "__HASHAI_PTY_READY__"
        output = subprocess.run(
            [os.environ["HASHAI_ZSH_BIN"], "-fc", "print -r -- '__HASHAI_PTY_''READY__'"],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(output.stdout, f"{marker}\n")

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
