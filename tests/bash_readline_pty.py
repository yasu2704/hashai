#!/usr/bin/env python3
"""Run an interactive Bash command file through a real PTY with a timeout."""

import os
import fcntl
import pty
import select
import signal
import subprocess
import sys
import termios
import time


PROGRESS_FRAME = "⠋".encode()
ACTION_READY = b"__HASHAI_BASH_READY__"


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: bash_readline_pty.py COMMANDS_FILE")
    command_groups = open(sys.argv[1], "rb").read().split(b"\0")
    master, slave = pty.openpty()
    def controlling_tty() -> None:
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(
        [os.environ.get("HASHAI_BASH_BIN", "bash"), "--noprofile", "--norc", "-i"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        preexec_fn=controlling_tty,
    )
    os.close(slave)
    os.set_blocking(master, False)
    progress = bytearray()

    def release_after_progress(group_index: int) -> None:
        release = os.environ.get("HASHAI_PROGRESS_RELEASE_FILE")
        if release is None or group_index != 1:
            time.sleep(0.5)
            return
        deadline = time.monotonic() + 5
        while PROGRESS_FRAME not in progress:
            if time.monotonic() >= deadline:
                raise RuntimeError("Bash did not display its transient progress frame")
            if not select.select([master], [], [], 0.1)[0]:
                continue
            try:
                output = os.read(master, 4096)
            except BlockingIOError:
                continue
            progress.extend(output)
            sys.stdout.buffer.write(output)
            sys.stdout.buffer.flush()
        if os.environ.get("HASHAI_PROGRESS_CANCEL") == "1":
            while os.tcgetpgrp(master) == process.pid:
                if time.monotonic() >= deadline:
                    raise RuntimeError("Hashai did not acquire the terminal foreground")
                time.sleep(0.01)
            os.write(master, b"\x03")
        else:
            with open(release, "xb"):
                pass
        tail = b""
        while ACTION_READY not in tail:
            if time.monotonic() >= deadline:
                raise RuntimeError("Bash callback did not complete after progress release")
            if not select.select([master], [], [], 0.1)[0]:
                continue
            output = os.read(master, 4096)
            sys.stdout.buffer.write(output)
            sys.stdout.buffer.flush()
            tail = (tail + output)[-4096:]
    try:
        time.sleep(0.1)  # Let Bash enter Readline before sourcing the binding.
        for index, commands in enumerate(command_groups):
            os.write(master, commands)
            if index + 1 < len(command_groups):
                release_after_progress(index)
        deadline = time.monotonic() + 10
        while process.poll() is None:
            if time.monotonic() >= deadline:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise RuntimeError("interactive Bash did not exit within 10 seconds")
            select.select([master], [], [], 0.1)
            try:
                output = os.read(master, 4096)
                sys.stdout.buffer.write(output)
                sys.stdout.buffer.flush()
            except OSError:
                pass
        return process.wait()
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        os.close(master)


if __name__ == "__main__":
    raise SystemExit(main())
