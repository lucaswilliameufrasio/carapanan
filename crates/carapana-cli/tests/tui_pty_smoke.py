"""Exercise carapanan tui in a real controlling terminal using only stdlib."""

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


if len(sys.argv) == 4 and sys.argv[1] == "__pty_session__":
    terminal_fd = int(sys.argv[2])
    tui_result = subprocess.run(
        [sys.argv[3], "tui"],
        stdin=terminal_fd,
        stdout=terminal_fd,
        stderr=terminal_fd,
        check=False,
    )
    print(tui_result.returncode, flush=True)
    sys.stdin.readline()
    raise SystemExit(tui_result.returncode)


binary = os.path.abspath(sys.argv[1])
database_path = Path(sys.argv[2])
temporary_root = os.path.realpath("/tmp" if sys.platform == "darwin" else tempfile.gettempdir())
with tempfile.TemporaryDirectory(prefix="c-", dir=temporary_root) as temporary:
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

    socket_path = carapana_data / "daemon.sock"
    if sys.platform == "darwin" and len(os.fsencode(socket_path)) >= 104:
        raise AssertionError("daemon socket path exceeds the Darwin SUN_LEN limit")

    daemon = subprocess.Popen(
        [binary, "daemon"],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    terminal_session = None
    master = slave = None
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

        terminal_session = subprocess.Popen(
            [
                sys.executable,
                os.path.abspath(__file__),
                "__pty_session__",
                str(slave),
                binary,
            ],
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
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
                descriptors = [master]
                if terminal_session.stdout is not None:
                    descriptors.append(terminal_session.stdout)
                ready, _, _ = select.select(descriptors, [], [], 0.05)
                if terminal_session.stdout in ready:
                    exit_status = terminal_session.stdout.readline().strip()
                    raise AssertionError(
                        f"TUI exited before rendering the expected state: {exit_status}"
                    )
                if master in ready:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
                    if predicate(output):
                        return
                if terminal_session.poll() is not None:
                    break
            raise AssertionError(
                f"TUI did not render expected state; output={bytes(output)!r}"
            )

        read_until(lambda data: b"pty-session" in visible_text(data))
        assert b"Local control" in visible_text(output)
        os.write(master, b"\r")
        read_until(lambda data: b"Status: Paused" in visible_text(data))
        os.write(master, b"e")
        read_until(lambda data: b"Event history" in visible_text(data))
        assert b"Created" in visible_text(output)
        os.write(master, b"q")

        deadline = time.monotonic() + 5
        assert terminal_session.stdout is not None
        while time.monotonic() < deadline:
            ready, _, _ = select.select(
                [master, terminal_session.stdout], [], [], 0.05
            )
            if terminal_session.stdout in ready:
                break
            if master in ready:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    pass
        assert terminal_session.stdout in ready, "q did not exit the TUI"
        tui_returncode = terminal_session.stdout.readline().strip()
        assert tui_returncode == "0", f"TUI exited with {tui_returncode}"
        assert terminal_session.poll() is None, "PTY session ended before terminal inspection"
        assert b"\x1b[?1049h" in output, "TUI did not enter alternate screen"
        assert b"\x1b[?1049l" in output, "TUI did not restore alternate screen"
        assert termios.tcgetattr(slave) == original_terminal, "TUI did not restore terminal modes"
        assert terminal_session.stdin is not None
        terminal_session.stdin.write("\n")
        terminal_session.stdin.flush()
        terminal_session.wait(timeout=5)
        assert terminal_session.returncode == 0, "PTY session helper did not exit cleanly"

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
        if terminal_session is not None and terminal_session.poll() is None:
            try:
                os.killpg(terminal_session.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            terminal_session.wait(timeout=5)
        if daemon.poll() is None:
            daemon.send_signal(signal.SIGTERM)
            daemon.wait(timeout=5)
        assert not socket_path.exists(), "daemon socket remained after shutdown"
        if master is not None:
            os.close(master)
        if slave is not None:
            os.close(slave)
