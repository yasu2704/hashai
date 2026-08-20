#!/usr/bin/env python3
"""Run an interactive Bash command file through a real PTY with a timeout."""

import os
import pty
import select
import signal
import subprocess
import sys
import time


PROGRESS_FRAMES = ["⠋".encode(), "⠙".encode()]


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: bash_readline_pty.py COMMANDS_FILE")
    command_groups = open(sys.argv[1], "rb").read().split(b"\0")
    master, slave = pty.openpty()
    process = subprocess.Popen(
        [os.environ.get("HASHAI_BASH_BIN", "bash"), "--noprofile", "--norc", "-i"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        preexec_fn=os.setsid,
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
        frame_index = 0
        while frame_index < len(PROGRESS_FRAMES):
            if time.monotonic() >= deadline:
                raise RuntimeError("Bash did not display two ordered progress frames")
            if not select.select([master], [], [], 0.1)[0]:
                continue
            try:
                output = os.read(master, 4096)
            except BlockingIOError:
                continue
            progress.extend(output)
            sys.stdout.buffer.write(output)
            sys.stdout.buffer.flush()
            while frame_index < len(PROGRESS_FRAMES):
                position = progress.find(PROGRESS_FRAMES[frame_index])
                if position < 0:
                    break
                del progress[: position + len(PROGRESS_FRAMES[frame_index])]
                frame_index += 1
        if os.environ.get("HASHAI_PROGRESS_CANCEL") == "1":
            os.write(master, b"\x03")
        else:
            with open(release, "xb"):
                pass
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
