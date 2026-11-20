#!/usr/bin/env python3
"""follow_log.py OUT PIDFILE LOGDIR [FILE...]

Appends to OUT every complete line the app writes to LOGDIR/js-debug-*.log
and to each FILE, in the order it reads them. The js-debug logs that exist
when it starts are read from their size then: their lines belong to earlier
starts. Each FILE is read from its start. It stops, after a last pass, on
SIGTERM, when the pid in PIDFILE is gone, or when PIDFILE has no pid after
NO_PID_SECONDS (the launch failed).
"""
import glob
import os
import signal
import sys
import time

POLL_SECONDS = 0.3
NO_PID_SECONDS = 300


def size_of(path):
    try:
        return os.path.getsize(path)
    except OSError:
        return 0


def read_pid(path):
    try:
        with open(path) as f:
            return int(f.read().strip() or 0)
    except (OSError, ValueError):
        return 0


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


class Follower:
    def __init__(self, out, logdir, files):
        self.out = out
        self.logdir = logdir
        self.files = files
        self.offsets = {path: size_of(path) for path in self.js_logs()}

    def js_logs(self):
        return sorted(glob.glob(os.path.join(self.logdir, "js-debug-*.log")))

    def copy_new(self, path, final):
        offset = self.offsets.get(path, 0)
        size = size_of(path)
        if size < offset:
            offset = 0
        if size == offset:
            return
        with open(path, "rb") as f:
            f.seek(offset)
            data = f.read(size - offset)
        if not final:
            data = data[: data.rfind(b"\n") + 1]
        if not data:
            return
        self.offsets[path] = offset + len(data)
        if not data.endswith(b"\n"):
            data += b"\n"
        with open(self.out, "ab") as f:
            f.write(data)

    def copy_all(self, final=False):
        for path in self.js_logs() + self.files:
            self.copy_new(path, final)


def main(argv):
    if len(argv) < 4:
        sys.exit(__doc__)
    out, pidfile, logdir = argv[1:4]
    follower = Follower(out, logdir, argv[4:])
    stopping = []
    signal.signal(signal.SIGTERM, lambda *_: stopping.append(True))
    started = time.monotonic()
    while not stopping:
        follower.copy_all()
        pid = read_pid(pidfile)
        if pid and not alive(pid):
            break
        if not pid and time.monotonic() - started > NO_PID_SECONDS:
            break
        time.sleep(POLL_SECONDS)
    follower.copy_all(final=True)


if __name__ == "__main__":
    main(sys.argv)
