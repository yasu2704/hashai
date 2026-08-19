#!/usr/bin/env python3
"""Run Fish command groups through a real PTY with marker synchronization."""
import fcntl, os, pty, select, signal, subprocess, sys, termios, time

def terminal_responses(data):
    """Return deterministic replies for Fish's startup terminal probes."""
    replies = []
    if b"\x1b[c" in data:
        replies.append(b"\x1b[?1;2c")
    if b"\x1b[?u" in data:
        replies.append(b"\x1b[?0u")
    if b"\x1b]11;?" in data:
        replies.append(b"\x1b]11;rgb:0000/0000/0000\x1b\\")
    if b"\x1b[>q" in data:
        replies.append(b"\x1b[>0;0;0c")
    # XTGETTCAP requests carry a DCS payload; explicit failure is stable and
    # lets Fish fall back without waiting for a terminal response.
    if b"\x1bP+q" in data:
        replies.append(b"\x1bP0+r\x1b\\")
    return replies

def write_all(fd, data):
    while data:
        count = os.write(fd, data)
        data = data[count:]

def wait_marker(fd, marker):
    tail = b""; deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .1)[0]:
            data = os.read(fd, 4096)
            for reply in terminal_responses(data): write_all(fd, reply)
            sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
            seen, tail = marker_seen(tail, data, marker)
            if seen: return
    raise RuntimeError("Fish did not emit readiness marker")

def marker_seen(tail, data, marker):
    combined = tail + data
    if marker in combined:
        return True, b""
    return False, combined[-(len(marker)-1):]

def main():
    groups = open(sys.argv[1], "rb").read().split(b"\0")
    master, slave = pty.openpty()
    def controlling_tty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    proc = subprocess.Popen([os.environ.get("HASHAI_FISH_BIN", "fish"), "--no-config", "--interactive"], stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty, close_fds=True)
    os.close(slave); os.set_blocking(master, False)
    try:
        for index, group in enumerate(groups):
            write_all(master, group)
            if index + 1 < len(groups): wait_marker(master, b"__HASHAI_FISH_READY__")
        deadline=time.monotonic()+10
        while proc.poll() is None:
            if time.monotonic()>deadline: raise RuntimeError("Fish did not exit")
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master,4096)
                    for reply in terminal_responses(data): write_all(master, reply)
                    sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
                except OSError: pass
        return proc.wait()
    finally:
        if proc.poll() is None: os.killpg(proc.pid, signal.SIGKILL); proc.wait()
        os.close(master)
if __name__ == "__main__": raise SystemExit(main())
