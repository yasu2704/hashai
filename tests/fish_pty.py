#!/usr/bin/env python3
"""Run Fish command groups through a real PTY with marker synchronization."""
import fcntl, os, pty, re, select, signal, subprocess, sys, termios, time

class ProbeResponder:
    """Return deterministic replies for Fish's startup terminal probes."""
    def __init__(self): self.tail = b""
    def responses(self, data):
        combined = self.tail + data; replies = []
        def reply(value): replies.append(value)
        consumed = 0
        da = max(combined.rfind(b"\x1b[c"), combined.rfind(b"\x1b[0c"))
        if da >= 0:
            reply(b"\x1b[?1;2c")
            consumed = max(consumed, da + (3 if combined[da:da+3] == b"\x1b[c" else 4))
        kitty = combined.rfind(b"\x1b[?u")
        if kitty >= 0:
            reply(b"\x1b[?0u")
            consumed = max(consumed, kitty + 4)
        osc = combined.rfind(b"\x1b]11;?")
        if osc >= 0:
            reply(b"\x1b]11;rgb:0000/0000/0000\x1b\\")
            consumed = max(consumed, osc + 6)
        version = combined.rfind(b"\x1b[>0q")
        if version >= 0:
            reply(b"\x1bP>|hashai\x1b\\")
            consumed = max(consumed, version + 5)
        dsr = combined.rfind(b"\x1b[6n")
        if dsr >= 0:
            reply(b"\x1b[1;1R")
            consumed = max(consumed, dsr + 4)
        for payload in re.findall(rb"\x1bP\+q(.*?)\x1b\\", combined):
            reply(b"\x1bP0+r" + payload + b"\x1b\\")
        matches = list(re.finditer(rb"\x1bP\+q(.*?)\x1b\\", combined))
        if matches: consumed = max(consumed, matches[-1].end())
        # A complete probe was consumed; retaining it would make the next
        # read reply twice. Otherwise retain only a possible split prefix.
        self.tail = combined[consumed:] if consumed else combined[-256:]
        return replies

def terminal_responses(data):
    """Compatibility helper for unit callers with one complete chunk."""
    return ProbeResponder().responses(data)

def write_all(fd, data):
    while data:
        count = os.write(fd, data)
        data = data[count:]

def wait_marker(fd, marker, responder):
    tail = b""; deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .1)[0]:
            data = os.read(fd, 4096)
            for reply in responder.responses(data): write_all(fd, reply)
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
    responder = ProbeResponder()
    def controlling_tty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    proc = subprocess.Popen([os.environ.get("HASHAI_FISH_BIN", "fish"), "--no-config", "--interactive"], stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty, close_fds=True)
    os.close(slave); os.set_blocking(master, False)
    try:
        for index, group in enumerate(groups):
            write_all(master, group)
            if index + 1 < len(groups): wait_marker(master, b"__HASHAI_FISH_READY__", responder)
        deadline=time.monotonic()+10
        while proc.poll() is None:
            if time.monotonic()>deadline: raise RuntimeError("Fish did not exit")
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master,4096)
                    for reply in responder.responses(data): write_all(master, reply)
                    sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
                except OSError: pass
        return proc.wait()
    finally:
        if proc.poll() is None: os.killpg(proc.pid, signal.SIGKILL); proc.wait()
        os.close(master)
if __name__ == "__main__": raise SystemExit(main())
