#!/usr/bin/env python3
"""Run Fish command groups through a real PTY with marker synchronization."""
import errno, fcntl, os, pty, re, select, signal, subprocess, sys, termios, time

locale_name = os.environ.get("LC_ALL") or os.environ.get("LC_CTYPE") or os.environ.get("LANG", "")
TEST_TIMEOUT = float(os.environ.get("HASHAI_FISH_TEST_TIMEOUT", "10"))
PROGRESS_FRAMES = (
    ["⠋ generating…".encode(), "⠙ generating…".encode()]
    if re.search(r"UTF-?8", locale_name, re.IGNORECASE)
    else ["| generating…".encode(), "/ generating…".encode()]
)

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

def write_all(fd, data, responder=None, pending=None):
    deadline = time.monotonic() + TEST_TIMEOUT
    while data:
        try:
            count = os.write(fd, data)
        except InterruptedError:
            continue
        except BlockingIOError as error:
            if error.errno not in (errno.EAGAIN, errno.EWOULDBLOCK): raise
            readable, writable, _ = select.select([fd], [fd], [], .1)
            if readable:
                output = os.read(fd, 4096)
                if pending is not None: pending.extend(output)
                sys.stdout.buffer.write(output); sys.stdout.buffer.flush()
                if responder:
                    for reply in responder.responses(output): write_all(fd, reply, pending=pending)
            if time.monotonic() >= deadline:
                raise RuntimeError(f"PTY did not become writable with {len(data)} bytes pending")
            if not writable:
                continue
            continue
        data = data[count:]

def wait_marker(fd, marker, responder, pending, progress):
    tail = b""; deadline = time.monotonic() + TEST_TIMEOUT
    while time.monotonic() < deadline:
        if pending:
            data = bytes(pending); pending.clear()
            maybe_release_progress(fd, data, progress)
            seen, tail = marker_seen(tail, data, marker)
            if seen:
                wait_until_quiet(fd, responder, pending, progress)
                return
        if select.select([fd], [], [], .1)[0]:
            data = os.read(fd, 4096)
            for reply in responder.responses(data): write_all(fd, reply)
            sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
            maybe_release_progress(fd, data, progress)
            seen, tail = marker_seen(tail, data, marker)
            if seen:
                wait_until_quiet(fd, responder, pending, progress)
                return
    raise RuntimeError("Fish did not emit readiness marker")

def wait_until_quiet(fd, responder, pending, progress):
    """Wait for the completed widget's repaint before sending the next key."""
    quiet_until = time.monotonic() + .1
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        timeout = max(0, quiet_until - time.monotonic())
        if not select.select([fd], [], [], timeout)[0]:
            return
        data = os.read(fd, 4096)
        for reply in responder.responses(data): write_all(fd, reply, pending=pending)
        sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
        maybe_release_progress(fd, data, progress)
        quiet_until = time.monotonic() + .1

def marker_seen(tail, data, marker):
    combined = tail + data
    if marker in combined:
        return True, b""
    return False, combined[-(len(marker)-1):]

def maybe_release_progress(fd, data, progress):
    release = os.environ.get("HASHAI_PROGRESS_RELEASE_FILE")
    if not release or os.path.exists(release): return
    progress.extend(data)
    first = progress.find(PROGRESS_FRAMES[0])
    second = progress.find(PROGRESS_FRAMES[1], first + len(PROGRESS_FRAMES[0])) if first >= 0 else -1
    if second >= 0:
        if os.environ.get("HASHAI_PROGRESS_CANCEL") == "1": os.write(fd, b"\x03")
        with open(release, "xb"): pass

def main():
    groups = open(sys.argv[1], "rb").read().split(b"\0")
    master, slave = pty.openpty()
    responder = ProbeResponder()
    pending = bytearray()
    progress = bytearray()
    def controlling_tty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    proc = subprocess.Popen([os.environ.get("HASHAI_FISH_BIN", "fish"), "--no-config", "--interactive"], stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_tty, close_fds=True)
    os.close(slave); os.set_blocking(master, False)
    try:
        for index, group in enumerate(groups):
            write_all(master, group, responder, pending)
            if index + 1 < len(groups): wait_marker(master, b"__HASHAI_FISH_READY__", responder, pending, progress)
        deadline=time.monotonic()+TEST_TIMEOUT
        retry_capture = time.monotonic() + .5
        while proc.poll() is None:
            if time.monotonic()>deadline: raise RuntimeError("Fish did not exit")
            if os.environ.get("HASHAI_PROGRESS_CANCEL") == "1" and time.monotonic() >= retry_capture:
                os.write(master, b"\x14")
                retry_capture = time.monotonic() + .5
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master,4096)
                    for reply in responder.responses(data): write_all(master, reply)
                    maybe_release_progress(master, data, progress)
                    sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
                except OSError: pass
        return proc.wait()
    finally:
        if proc.poll() is None: os.killpg(proc.pid, signal.SIGKILL); proc.wait()
        os.close(master)
if __name__ == "__main__": raise SystemExit(main())
