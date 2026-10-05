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
import json
from pathlib import Path


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
    flags = [] if os.environ.get("CARAPANA_REVIEW_COLOR") == "1" else ["--plain"]
    os.execv(sys.argv[1], [sys.argv[1], *flags])

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


def capture(name):
    directory = os.environ.get("CARAPANA_REVIEW_DIR")
    if directory:
        # A redraw can arrive in multiple writes. Capture only after draining
        # the full frame, not as soon as its first heading happens to match.
        deadline = time.monotonic() + 0.25
        while time.monotonic() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                data = os.read(fd, 65536)
                if b"\x1b[6n" in data:
                    os.write(fd, b"\x1b[1;1R")
                screen.feed(decoder.decode(data))
        path = Path(directory)
        path.mkdir(parents=True, exist_ok=True)
        (path / f"{name}.json").write_text(json.dumps({"name":name,"width":screen.width,"height":screen.height,"lines":["".join(row) for row in screen.cells]}, ensure_ascii=False))


try:
    resize(80, 24)
    expect_output("Próxima:")
    assert "Permitir esta ação?" not in screen.text()
    assert "183" not in screen.text()
    capture("empty-80x24")
    expect_output("Nova versão disponível", b"/update\r")
    capture("updates-available-80x24")
    expect_output("Verificação pendente", b"\r")
    expect_output("sem reiniciar o agente", b"\r")
    capture("updates-interface-installed-80x24")
    expect_output("Rollback simulado concluído", b"\r")
    # Rolled back: Later, Package, Channel. Select Package without Alt/F keys.
    expect_output("Pacote de demonstração", b"\x1b[B\r")
    expect_output("Runtime · reinício seguro", b"runtime\r")
    expect_output("Verificação pendente", b"\r")
    expect_output("Reinício pendente", b"\r")
    capture("updates-runtime-blocked-80x24")
    expect_output("Ação bloqueada", b"\x1b[B\r")
    expect_output("Ponto seguro na fixture", b"\r")
    expect_output("nenhum processo reiniciado", b"\r")
    capture("updates-runtime-installed-80x24")
    expect_output("Rollback simulado concluído", b"\r")
    expect_output("Pacote de demonstração", b"\x1b[B\r")
    expect_output("Assinatura inválida", b"assinatura\r")
    expect_output("Verificação pendente", b"\r")
    expect_output("Atualização bloqueada", b"\r")
    capture("updates-invalid-80x24")
    expect_output("Pacote de demonstração", b"\x1b[B\r")
    expect_output("Migração", b"migra\r")
    expect_output("Atualização agendada", b"\x1b[B\r")
    expect_output("Verificação pendente", b"\r")
    expect_output("Reinício pendente", b"\r")
    expect_output("nenhum processo reiniciado", b"\r")
    assert "Simular rollback" not in screen.text()
    capture("updates-migration-80x24")
    # Later hides only the notice; Ctrl+P can still find /update.
    expect_output("Próxima:", b"\r")
    expect_output("Opções", b"\x10")
    expect_output("Atualizações · mock", b"update\r")
    os.write(fd, b"\x1b")
    time.sleep(0.12)
    expect_output("Colagem multilinha", b"?")
    expect_output("Escolher modelo", b"\x1b[C")
    expect_output("/intervene", b"\x1b[C")
    expect_output("Retomar trabalho pausado", b"\x1b[C")
    capture("help-control-80x24")
    os.write(fd, b"\x1b")
    time.sleep(0.12)
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
    assert "Na fila" not in screen.text(), screen.text()
    expect_output("Mensagem recebida", b"\r")
    assert "auth/service.rs" not in screen.text()
    capture("ordinary-chat-80x24")
    expect_output("Em andamento", b"/demo\r")
    capture("submitted-80x24")
    expect_output("Read auth/service.rs")
    expect_output("Permitir esta ação?")
    assert "Demonstração:" in screen.text()
    capture("approval-80x24")
    expect_output("Permitir esta ação?", resize_to=(160, 48))
    capture("approval-160x48")
    expect_output("Permitir esta ação?", resize_to=(80, 24))
    # Tab moves to the composer; Enter queues a message, never authorizes.
    expect_output("Tab volta", b"\t")
    expect_output("Na fila (1)", b"pedido seguinte\r")
    assert "Permitir esta ação?" in screen.text()
    capture("approval-with-queue-80x24")
    expect_output("Negar e pausar", b"\t")
    expect_output("Em pausa", b"\r")  # Deny is selected by default.
    assert "Na fila (1)" in screen.text()
    expect_output("Opções", b"\x10")
    expect_output("/resume", b"resume")
    expect_output("Permitir esta ação?", b"\r")
    expect_output("Edit auth/service.rs", b"1\r")
    expect_output("Test cargo test")
    expect_output("exit: 0", b"\x0f")
    expect_output("Test cargo test", b"\x0f")  # Collapse diagnostic details again.
    expect_output("Rotação demonstrada")
    expect_output("> pedido seguinte")
    capture("queue-advanced-80x24")
    expect_output("Interrompido (mock)", b"\x1b")
    assert "Em pausa" in screen.text()
    capture("interrupted-80x24")
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
    print("Should validate real PTY updates, message → activity → approval → result → queue advancement, resize and interruption: passed")
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    os.close(fd)
