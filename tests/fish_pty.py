#!/usr/bin/env python3
"""Run Fish command groups through a real PTY with marker synchronization."""
import fcntl, os, pty, select, signal, subprocess, sys, termios, time

def write_all(fd, data):
    while data:
        count = os.write(fd, data)
        data = data[count:]

def wait_marker(fd, marker):
    tail = b""; deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if select.select([fd], [], [], .1)[0]:
            data = os.read(fd, 4096); sys.stdout.buffer.write(data); sys.stdout.buffer.flush()
            combined = tail + data
            if marker in combined: return
            tail = combined[-(len(marker)-1):]
    raise RuntimeError("Fish did not emit readiness marker")

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
                try: sys.stdout.buffer.write(os.read(master,4096)); sys.stdout.buffer.flush()
                except OSError: pass
        return proc.wait()
    finally:
        if proc.poll() is None: os.killpg(proc.pid, signal.SIGKILL); proc.wait()
        os.close(master)
if __name__ == "__main__": raise SystemExit(main())
