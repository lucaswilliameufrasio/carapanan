"""Exercise carapana tui in a real controlling terminal using only stdlib."""

import fcntl
import json
import os
import pty
import re
import select
import signal
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path


binary = os.path.abspath(sys.argv[1])
database_path = Path(sys.argv[2])
with tempfile.TemporaryDirectory(prefix="carapana-tui-pty-") as temporary:
    root = Path(temporary)
    home = root / "home"
    data_home = root / "xdg"
    home.mkdir()
    data_home.mkdir()
    if sys.platform == "darwin":
        carapana_data = home / "Library" / "Application Support" / "Carapana"
    else:
        carapana_data = data_home / "carapana"
    carapana_data.mkdir(mode=0o700, parents=True)
    shutil.copy2(database_path, carapana_data / "sessions.sqlite3")
    environment = os.environ.copy()
    environment.update(
        HOME=str(home),
        XDG_DATA_HOME=str(data_home),
        TERM="xterm-256color",
    )

    daemon = subprocess.Popen(
        [binary, "daemon"],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    tui = None
    master = slave = None
    socket_path = carapana_data / "daemon.sock"
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not socket_path.exists():
            if daemon.poll() is not None:
                raise AssertionError(
                    f"daemon exited before binding socket: "
                    f"{daemon.stderr.read().decode(errors='replace')}"
                )
            time.sleep(0.02)
        assert socket_path.exists(), "daemon did not create its private socket"

        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        original_terminal = termios.tcgetattr(slave)

        def acquire_controlling_terminal():
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        tui = subprocess.Popen(
            [binary, "tui"],
            env=environment,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
            pass_fds=(slave,),
            preexec_fn=acquire_controlling_terminal,
        )

        output = bytearray()

        def visible_text(data):
            without_controls = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b" ", data)
            return b" ".join(without_controls.split())

        def read_until(predicate, timeout=5):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                ready, _, _ = select.select([master], [], [], 0.05)
                if ready:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
                    if predicate(output):
                        return
                if tui.poll() is not None:
                    break
            raise AssertionError(
                f"TUI did not render expected state; output={bytes(output)!r}"
            )

        read_until(lambda data: b"pty-session" in visible_text(data))
        assert b"Read-only" in visible_text(output)
        os.write(master, b"\r")
        read_until(lambda data: b"Status: Paused" in visible_text(data))
        os.write(master, b"e")
        read_until(lambda data: b"Event history" in visible_text(data))
        assert b"Created" in visible_text(output)
        os.write(master, b"q")

        deadline = time.monotonic() + 5
        while tui.poll() is None and time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        assert tui.poll() is not None, "q did not exit the TUI"
        assert tui.returncode == 0, f"TUI exited with {tui.returncode}"
        assert b"\x1b[?1049h" in output, "TUI did not enter alternate screen"
        assert b"\x1b[?1049l" in output, "TUI did not restore alternate screen"
        assert termios.tcgetattr(slave) == original_terminal, "TUI did not restore terminal modes"

        sessions = subprocess.run(
            [binary, "sessions", "--json"],
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        )
        listed_sessions = json.loads(sessions.stdout)
        assert len(listed_sessions) == 1
        assert listed_sessions[0]["session_id"] == "pty-session"
        assert listed_sessions[0]["status"] == "paused"
        assert listed_sessions[0]["queued_count"] == 1
        assert listed_sessions[0]["attached_clients"] == 0
        snapshot = subprocess.run(
            [binary, "show", "pty-session", "--json"],
            env=environment,
            check=True,
            capture_output=True,
            text=True,
        )
        queued = json.loads(snapshot.stdout)["snapshot"]["queued_messages"]
        assert len(queued) == 1
        assert queued[0]["id"] == "pty-queued-1"
        assert queued[0]["text"] == "must remain queued"
        assert daemon.poll() is None, "read-only TUI unexpectedly stopped the daemon"
        print("real PTY startup, read-only view, q exit and terminal restoration: passed")
    finally:
        if tui is not None and tui.poll() is None:
            tui.terminate()
            tui.wait(timeout=5)
        if daemon.poll() is None:
            daemon.send_signal(signal.SIGTERM)
            daemon.wait(timeout=5)
        assert not socket_path.exists(), "daemon socket remained after shutdown"
        if master is not None:
            os.close(master)
        if slave is not None:
            os.close(slave)
