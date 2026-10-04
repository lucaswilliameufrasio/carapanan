"""Exercise the actual terminal binary through a controlling Unix PTY.

Standard library only. Reducer/render tests assert full screen content; this
integration check covers real terminal initialization, event decoding and resize.
"""
import codecs
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time
import unicodedata


class Screen:
    """Small screen for Ratatui's CUP/erase output, including incremental CSI.

    Searching concatenated output is incorrect: differential redraws transmit
    only changed cells, so even a heading can arrive as several fragments.
    """

    def __init__(self):
        self.width, self.height = 80, 24
        self.cells = [[" "] * self.width for _ in range(self.height)]
        self.x = self.y = 0
        self.pending = ""

    def resize(self, width, height):
        self.width, self.height = width, height
        self.cells = [[" "] * width for _ in range(height)]
        self.x = self.y = 0

    def feed(self, text):
        self.pending += text
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if not match:
                    return
                values, _, command = match.groups()
                self.pending = self.pending[match.end():]
                numbers = [int(v or "0") for v in values.split(";")] if not values.startswith("?") else []
                if command in "Hf":
                    self.y = (numbers[0] or 1) - 1
                    self.x = ((numbers[1] if len(numbers) > 1 else 1) or 1) - 1
                elif command == "J" and numbers and numbers[0] == 2:
                    self.cells = [[" "] * self.width for _ in range(self.height)]
                elif command == "K" and self.y < self.height:
                    for x in range(self.x, self.width):
                        self.cells[self.y][x] = " "
                continue
            if self.pending == "\x1b":
                return
            character, self.pending = self.pending[0], self.pending[1:]
            if character == "\r":
                self.x = 0
            elif character == "\n":
                self.y += 1
            elif ord(character) >= 32:
                width = 0 if unicodedata.combining(character) else 2 if unicodedata.east_asian_width(character) in "WF" else 1
                if 0 <= self.y < self.height and 0 <= self.x < self.width:
                    self.cells[self.y][self.x] = character
                self.x += width

    def text(self):
        return "\n".join("".join(row) for row in self.cells)


pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv(sys.argv[1], [sys.argv[1], "--plain"])

screen = Screen()
decoder = codecs.getincrementaldecoder("utf-8")("replace")


def resize(width, height):
    screen.resize(width, height)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    os.kill(pid, signal.SIGWINCH)


def expect_output(expected, keys=b"", resize_to=None):
    if keys:
        os.write(fd, keys)
    if resize_to:
        resize(*resize_to)
    deadline = time.monotonic() + 5
    output = ""
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if not ready:
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if b"\x1b[6n" in data:
            os.write(fd, b"\x1b[1;1R")
        text = decoder.decode(data)
        output += text
        screen.feed(text)
        if expected in screen.text():
            return
    raise AssertionError(f"Missing {expected!r} in real PTY output: {output!r}")


try:
    resize(80, 24)
    expect_output("Próxima:")
    expect_output("Modelo da próxima mensagem", b"/model\r")
    os.write(fd, b"\x1b")
    time.sleep(0.12)  # Bare Escape must not be decoded as an Alt-prefixed next key.
    expect_output("Opções", b"\x10")  # Ctrl+P, as transmitted by normal SSH terminals.
    expect_output("Perfil da próxima mensagem", b"profile\r")
    os.write(fd, b"\x1b")
    time.sleep(0.12)
    expect_output("Raciocínio", b"\x1bv")
    expect_output("Raciocínio", resize_to=(160, 48))
    expect_output("Raciocínio", resize_to=(80, 24))
    expect_output("Menu cancelado", b"\x03")
    expect_output("linha 2", b"\x1b[200~linha 1\nlinha 2\x1b[201~")
    assert "linha 1" in screen.text()
    assert "Fila: 1" in screen.text(), screen.text()
    expect_output("Interrompido (mock)", b"\x1b")
    assert "linha 2" in screen.text(), screen.text()
    expect_output("Opções", b"\x10")
    expect_output("/resume", b"resume")
    expect_output("Executando", b"\r")
    expect_output("Interrompido (mock)", b"\x03")
    expect_output("exit: 0", b"\x0f")
    expect_output("Input limpo", b"\x03")
    os.write(fd, b"\x03")
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        child, status = os.waitpid(pid, os.WNOHANG)
        if child:
            assert os.waitstatus_to_exitcode(status) == 0
            break
        time.sleep(0.02)
    else:
        raise AssertionError("Second idle Ctrl+C did not terminate the TUI")
    print("Should decode real PTY pickers, resize, multiline paste, interruption, details and clean exit: passed")
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    os.close(fd)
