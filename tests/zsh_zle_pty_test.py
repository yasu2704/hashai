#!/usr/bin/env python3
"""Focused tests for Zsh PTY readiness-marker framing."""

import pathlib
import os
import shutil
import subprocess
import sys
import unittest
import errno
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from zsh_zle_pty import MAX_WRITE_CHUNK, marker_seen, read_until_marker, write_all


class MarkerSeenTests(unittest.TestCase):
    def test_keymap_selection_precedes_artifact_installation(self) -> None:
        harness = (pathlib.Path(__file__).parent / "zsh_zle_integration.sh").read_text()
        self.assertLess(harness.index("bindkey -$keymap"), harness.index("source '$artifact_path'"))

    def test_long_zle_setup_is_sourced_not_streamed_through_the_terminal(self) -> None:
        harness = (pathlib.Path(__file__).parent / "zsh_zle_integration.sh").read_text()
        self.assertIn('cat >"$setup" <<EOF', harness)
        self.assertIn('printf "source \'%s\'\\n%s\\n\\0" "$setup" "$readiness_command"', harness)

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
        zsh = os.environ.get("HASHAI_ZSH_BIN", "zsh")
        if shutil.which(zsh) is None:
            self.skipTest("Zsh is unavailable locally; CI exercises the exact command")
        output = subprocess.run(
            [zsh, "-fc", "print -r -- '__HASHAI_PTY_''READY__'"],
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

    def test_write_backpressure_drains_and_preserves_marker_for_waiter(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        pending = bytearray()
        with (
            patch("zsh_zle_pty.select.select", return_value=([9], [9], [])),
            patch("zsh_zle_pty.os.read", return_value=b"prompt " + marker),
            patch(
                "zsh_zle_pty.os.write",
                side_effect=[BlockingIOError(errno.EAGAIN, "again"), 1, 2],
            ),
        ):
            write_all(9, b"abc", pending)
        self.assertIn(marker, pending)

    def test_pending_marker_split_across_backpressure_reads_is_detected(self) -> None:
        marker = b"__HASHAI_PTY_READY__"
        pending = bytearray(b"prefix __HASHAI_")
        with patch("zsh_zle_pty.select.select", return_value=([], [], [])):
            # The second half came from a later drain; wait consumes the same
            # shared pending stream rather than discarding it between phases.
            pending.extend(b"PTY_READY__\r\n% ")
            read_until_marker(9, marker, 0, pending, bytearray())

    def test_write_chunks_are_bounded_and_keep_order_after_eagain(self) -> None:
        pending = bytearray()
        writes: list[bytes] = []

        def record_write(_: int, data: bytes) -> int:
            writes.append(data)
            if len(writes) == 1:
                raise BlockingIOError(errno.EAGAIN, "again")
            return len(data)

        payload = bytes(range(256)) * 3
        with (
            patch("zsh_zle_pty.select.select", return_value=([], [9], [])),
            patch("zsh_zle_pty.os.write", side_effect=record_write),
        ):
            write_all(9, payload, pending, group_index=7)
        self.assertTrue(all(len(chunk) <= MAX_WRITE_CHUNK for chunk in writes))
        self.assertEqual(b"".join(writes[1:]), payload)
