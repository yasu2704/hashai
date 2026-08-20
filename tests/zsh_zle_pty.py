#!/usr/bin/env python3
"""Run an interactive Zsh command file through a real PTY with a timeout."""

import errno
import os
import pty
import select
import signal
import subprocess
import sys
import time


MAX_WRITE_CHUNK = 256
PROGRESS_FRAMES = ["⠋".encode(), "⠙".encode()]


def drain(fd: int, pending: bytearray) -> None:
    """Drain terminal output without losing readiness bytes during a write."""
    try:
        output = os.read(fd, 4096)
    except (BlockingIOError, OSError):
        return
    pending.extend(output)
    sys.stdout.buffer.write(output)
    sys.stdout.buffer.flush()


def write_all(fd: int, data: bytes, pending: bytearray, group_index: int | None = None) -> None:
    """Duplex PTY write with bounded EAGAIN handling and preserved output."""
    deadline = time.monotonic() + 10
    group = f"group {group_index}" if group_index is not None else "write"
    while data:
        if time.monotonic() >= deadline:
            raise RuntimeError(f"PTY {group} did not become writable with {len(data)} bytes pending")
        readable, writable, _ = select.select([fd], [fd], [], 0.1)
        if readable:
            drain(fd, pending)
        if not writable:
            continue
        try:
            written = os.write(fd, data[:MAX_WRITE_CHUNK])
        except InterruptedError:
            continue
        except BlockingIOError as error:
            if error.errno not in (errno.EAGAIN, errno.EWOULDBLOCK):
                raise
            continue
        if written == 0:
            raise RuntimeError("PTY write returned zero bytes")
        data = data[written:]


def marker_seen(tail: bytes, output: bytes, marker: bytes) -> tuple[bool, bytes]:
    """Detect a readiness marker without losing it to a prompt suffix."""
    combined = tail + output
    if marker in combined:
        return True, b""
    return False, combined[-(len(marker) - 1) :]


def read_until_marker(fd: int, marker: bytes, deadline: float, pending: bytearray, progress: bytearray) -> None:
    """Forward PTY output until setup has returned to an interactive prompt."""
    received = b""
    while marker not in received:
        if pending:
            output = bytes(pending)
            pending.clear()
            maybe_release_progress(fd, output, progress)
            seen, received = marker_seen(received, output, marker)
            if seen:
                return
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Zsh did not emit PTY readiness marker {marker!r}")
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            output = os.read(fd, 4096)
        except (BlockingIOError, OSError):
            continue
        seen, received = marker_seen(received, output, marker)
        maybe_release_progress(fd, output, progress)
        sys.stdout.buffer.write(output)
        sys.stdout.buffer.flush()
        if seen:
            return


def maybe_release_progress(fd: int, output: bytes, progress: bytearray) -> None:
    release = os.environ.get("HASHAI_PROGRESS_RELEASE_FILE")
    if release is None or os.path.exists(release):
        return
    progress.extend(output)
    first = progress.find(PROGRESS_FRAMES[0])
    second = progress.find(PROGRESS_FRAMES[1], first + len(PROGRESS_FRAMES[0])) if first >= 0 else -1
    if second >= 0:
        if os.environ.get("HASHAI_PROGRESS_CANCEL") == "1":
            os.write(fd, b"\x03")
            with open(release, "xb"):
                pass
        else:
            with open(release, "xb"):
                pass


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
    pending = bytearray()
    progress = bytearray()
    try:
        for index, commands in enumerate(command_groups):
            write_all(master, commands, pending, index)
            if index + 1 < len(command_groups):
                read_until_marker(master, b"__HASHAI_PTY_READY__", time.monotonic() + 10, pending, progress)
        deadline = time.monotonic() + 10
        while process.poll() is None:
            if time.monotonic() >= deadline:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise RuntimeError("interactive Zsh did not exit within 10 seconds")
            select.select([master], [], [], 0.1)
            try:
                drain(master, pending)
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
