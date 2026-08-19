#!/usr/bin/env python3
"""Run an interactive Zsh command file through a real PTY with a timeout."""

import os
import pty
import select
import signal
import subprocess
import sys
import time


def write_all(fd: int, data: bytes) -> None:
    """Write one keystroke group completely; PTYs may accept partial writes."""
    while data:
        written = os.write(fd, data)
        data = data[written:]


def read_until_marker(fd: int, marker: bytes, deadline: float) -> None:
    """Forward PTY output until setup has returned to an interactive prompt."""
    received = b""
    while marker not in received:
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Zsh did not emit PTY readiness marker {marker!r}")
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            output = os.read(fd, 4096)
        except OSError:
            continue
        received = (received + output)[-len(marker) :]
        sys.stdout.buffer.write(output)
        sys.stdout.buffer.flush()


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: zsh_zle_pty.py COMMANDS_FILE")
    with open(sys.argv[1], "rb") as commands_file:
        command_groups = commands_file.read().split(b"\0")
    master, slave = pty.openpty()
    process = subprocess.Popen(
        [os.environ.get("HASHAI_ZSH_BIN", "zsh"), "-f", "-i"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        preexec_fn=os.setsid,
    )
    os.close(slave)
    os.set_blocking(master, False)
    try:
        for index, commands in enumerate(command_groups):
            write_all(master, commands)
            if index + 1 < len(command_groups):
                read_until_marker(master, b"__HASHAI_PTY_READY__", time.monotonic() + 10)
        deadline = time.monotonic() + 10
        while process.poll() is None:
            if time.monotonic() >= deadline:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise RuntimeError("interactive Zsh did not exit within 10 seconds")
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
