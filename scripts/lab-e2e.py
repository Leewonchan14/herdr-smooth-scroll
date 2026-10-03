#!/usr/bin/env python3
"""End-to-end proof for herdr-smooth-scroll in an isolated named Herdr session.

Starts a real Herdr client in a PTY, fills the focused pane with numbered lines, then invokes the
plugin's ``halfpage-up`` action and asserts:

  * the pane's scroll offset is sampled moving through intermediate values (stdlib proof),
  * the client renders intermediate frames while the scroll runs (needs ``pyte``: the raw stream is
    diff-rendered, so frames are reconstructed with a terminal emulator),
  * the final offset and the visible viewport match the halfpage target,
  * a scroll past the top edge stops at ``max_offset_from_bottom`` without any steps,
  * the plugin's state log recorded the run.

Never touches the default session: it uses ``--session herdr-smooth-scroll-lab`` only.
"""

import json
import os
import pty
import re
import select
import shutil
import signal
import socket
import subprocess
import sys
import termios
import threading
import time

SESSION = "herdr-smooth-scroll-lab"
SESSION_DIR = os.path.expanduser(f"~/.config/herdr/sessions/{SESSION}")
PLUGIN = "Leewonchan14.herdr-smooth-scroll"
SOCK = os.path.expanduser(f"~/.config/herdr/sessions/{SESSION}/herdr.sock")
STATE_GLOB_DIR = os.path.expanduser("~/.local/state/herdr/plugins")
FIXTURE_LINES = 300
SAMPLE_INTERVAL = 0.003

try:
    import pyte  # optional: reconstructs rendered frames from the client stream
except ImportError:
    pyte = None

ANSI_RE = re.compile(
    r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"   # OSC
    r"|\x1b\[[0-9;?]*[ -/]*[@-~]"          # CSI
    r"|\x1b[()][A-Z0-9]"                   # charset selection
)
# Private CSI sequences pyte cannot parse; they do not affect the rendered grid.
PRIVATE_CSI_RE = re.compile(r"\x1b\[\?[0-9;]*[A-Za-z]")
# OSC sequences (hyperlinks, titles); they do not affect the rendered grid.
OSC_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def sock_call(method, params, timeout=5):
    """One request on a fresh connection (the server answers one request per connection)."""
    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.connect(SOCK)
    try:
        file = stream.makefile("rwb")
        request = json.dumps({"id": "lab", "method": method, "params": params})
        file.write((request + "\n").encode())
        file.flush()
        stream.settimeout(timeout)
        line = file.readline()
        if not line:
            raise RuntimeError(f"{method} returned no response")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"{method} failed: {response['error']}")
        return response["result"]
    finally:
        stream.close()


def pane_scroll(pane_id):
    """Scroll metrics for a pane, or None while the pane's terminal has no metrics yet."""
    return sock_call("pane.get", {"pane_id": pane_id})["pane"].get("scroll")


def pane_top_line(screen):
    """First numbered row inside the pane area, from a rendered pyte screen.

    Pane rows look like ``263 ... ▕`` (the trailing block is the pane scrollbar), so only the
    leading number is matched.
    """
    for row in screen.display[1:]:
        segment = row.split("│")[-1].strip()
        match = re.match(r"(\d+)", segment)
        if match:
            return int(match.group(1))
    return None


class Lab:
    """A real Herdr client in a PTY, plus CLI access to its named session."""

    def __init__(self):
        import fcntl
        import struct

        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        env = dict(os.environ)
        env["TERM"] = "xterm-256color"
        env.pop("HERDR_SOCKET_PATH", None)
        self.proc = subprocess.Popen(
            ["herdr", "--session", SESSION],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            close_fds=True,
        )
        os.close(slave)
        self.output = bytearray()
        self.frames = []
        self._screen = None
        self._frame_stop = False
        self._frame_thread = None

    def pump(self, seconds):
        deadline = time.time() + seconds
        while time.time() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                break
            if not chunk:
                break
            self.output.extend(chunk)

    def plain_since(self, offset):
        """Client output after `offset`, with escape sequences removed."""
        return ANSI_RE.sub(" ", self.output[offset:].decode("utf-8", "replace"))

    def wait_for_regex(self, pattern, since, timeout=8.0):
        """Pump the client stream until the new output matches `pattern`."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if re.search(pattern, self.plain_since(since)):
                return True
            self.pump(0.25)
        return False

    def start_frames(self):
        """Continuously pump the client and record the rendered pane top line per frame."""
        if pyte is None:
            return
        self._screen = pyte.Screen(120, 40)
        self._stream = pyte.Stream(self._screen)
        self._stream.feed(self._clean(self.output))
        self.frames = [(time.monotonic(), pane_top_line(self._screen))]

        def run():
            while not self._frame_stop:
                ready, _, _ = select.select([self.master], [], [], 0.005)
                if not ready:
                    continue
                try:
                    chunk = os.read(self.master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                self.output.extend(chunk)
                self._stream.feed(self._clean(chunk))
                top = pane_top_line(self._screen)
                if self.frames[-1][1] != top:
                    self.frames.append((time.monotonic(), top))

        self._frame_thread = threading.Thread(target=run, daemon=True)
        self._frame_thread.start()

    def stop_frames(self):
        self._frame_stop = True
        if self._frame_thread is not None:
            self._frame_thread.join(timeout=2)
            self._frame_thread = None
        return self.frames

    @staticmethod
    def _clean(data):
        if isinstance(data, (bytes, bytearray)):
            data = data.decode("utf-8", "replace")
        # Keep cursor movement; drop OSC and private CSI sequences pyte cannot parse.
        return PRIVATE_CSI_RE.sub("", OSC_RE.sub("", data))

    def cli(self, *args):
        env = dict(os.environ)
        env["HERDR_SESSION"] = SESSION
        return subprocess.run(
            ["herdr", "--session", SESSION, *args],
            capture_output=True,
            text=True,
            env=env,
            timeout=30,
        )

    def close(self):
        try:
            self.cli("server", "stop")
        except Exception:
            pass
        self.proc.send_signal(signal.SIGTERM)
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        try:
            os.close(self.master)
        except OSError:
            pass


class OffsetSampler:
    """Samples the pane's scroll offset on a background thread."""

    def __init__(self, pane_id):
        self.pane_id = pane_id
        self.samples = []  # (monotonic time, offset)
        self._stop = False
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        while not self._stop:
            try:
                state = pane_scroll(self.pane_id)
                if state is not None:
                    self.samples.append((time.monotonic(), state["offset_from_bottom"]))
            except Exception:
                pass
            time.sleep(SAMPLE_INTERVAL)

    def start(self):
        self._thread.start()

    def stop(self):
        self._stop = True
        self._thread.join(timeout=2)


class ScrollEvents:
    """A `pane.scroll_changed` subscription; used to prove that no pane.scroll call happened."""

    def __init__(self, pane_id):
        self.stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.stream.connect(SOCK)
        self.file = self.stream.makefile("rwb")
        request = json.dumps(
            {
                "id": "sub",
                "method": "events.subscribe",
                "params": {"subscriptions": [{"type": "pane.scroll_changed", "pane_id": pane_id}]},
            }
        )
        self.file.write((request + "\n").encode())
        self.file.flush()
        self.stream.settimeout(5)
        ack = json.loads(self.file.readline())
        assert ack["result"]["type"] == "subscription_started", ack
        self.events = []
        self._stop = False
        self._thread = threading.Thread(target=self._read, daemon=True)
        self._thread.start()

    def _read(self):
        self.stream.settimeout(30)
        while not self._stop:
            try:
                line = self.file.readline()
            except OSError:
                break
            if not line:
                break
            event = json.loads(line)
            if event.get("event") == "pane.scroll_changed":
                self.events.append(event["data"]["scroll"]["offset_from_bottom"])

    def close(self):
        self._stop = True
        try:
            self.stream.close()
        except OSError:
            pass
        self._thread.join(timeout=2)


def first_visible_line(pane_id):
    text = subprocess.run(
        ["herdr", "--session", SESSION, "pane", "read", pane_id, "--source", "visible"],
        capture_output=True,
        text=True,
    ).stdout
    for line in text.splitlines():
        line = line.strip()
        if line:
            return line
    return ""


def state_log_text():
    for entry in os.listdir(STATE_GLOB_DIR):
        if "herdr-smooth-scroll" in entry:
            path = os.path.join(STATE_GLOB_DIR, entry, "herdr-smooth-scroll.log")
            if os.path.exists(path):
                return open(path).read()
    return ""


def distinct(values):
    out = []
    for value in values:
        if not out or out[-1] != value:
            out.append(value)
    return out


def main():
    if shutil.which("herdr") is None:
        print("herdr not on PATH", file=sys.stderr)
        return 2

    log_before = state_log_text()
    # Start from a clean session: a stale saved workspace (for example one whose cwd no longer
    # exists) restores without a live terminal and the lab would never see scroll metrics.
    subprocess.run(
        ["herdr", "--session", SESSION, "server", "stop"], capture_output=True, text=True
    )
    time.sleep(0.3)
    shutil.rmtree(SESSION_DIR, ignore_errors=True)
    lab = Lab()
    failures = []
    try:
        # Wait for the session server to answer.
        for _ in range(60):
            probe = lab.cli("pane", "list")
            if probe.returncode == 0 and "pane_id" in probe.stdout:
                break
            time.sleep(0.5)
        else:
            failures.append("lab session never became reachable")
            raise SystemExit(report(failures))

        if PLUGIN not in lab.cli("plugin", "list").stdout:
            failures.append(f"{PLUGIN} is not visible in the lab session (run `herdr plugin link .`)")
            raise SystemExit(report(failures))

        pane_id = json.loads(lab.cli("pane", "list").stdout)["result"]["panes"][0]["pane_id"]

        # Fill the pane with numbered lines and wait for the scrollback and metrics to settle.
        lab.cli("pane", "run", pane_id, f"seq 1 {FIXTURE_LINES}")
        deadline = time.time() + 20
        baseline = None
        while time.time() < deadline:
            state = pane_scroll(pane_id)
            if state and state["max_offset_from_bottom"] >= FIXTURE_LINES // 2:
                baseline = state
                break
            time.sleep(0.25)
        if baseline is None:
            failures.append("fixture output never produced scrollback metrics")
            raise SystemExit(report(failures))
        expected_lines = max(1, baseline["viewport_rows"] // 2)
        expected_final = baseline["offset_from_bottom"] + expected_lines
        top_before = first_visible_line(pane_id)

        sampler = OffsetSampler(pane_id)
        sampler.start()
        lab.start_frames()
        invoked = lab.cli("plugin", "action", "invoke", "halfpage-up", "--plugin", PLUGIN)
        if invoked.returncode != 0 or '"error"' in invoked.stdout:
            failures.append(f"halfpage-up invoke failed: {invoked.stdout} {invoked.stderr}")
        # Let the sampler and the frame reader observe the settle after the action exits.
        time.sleep(0.4)
        sampler.stop()
        frames = lab.stop_frames()

        # Offset samples: the pane must land on the target without regressing.
        offsets = distinct(offset for _, offset in sampler.samples)
        if offsets and offsets[-1] != expected_final:
            failures.append(f"last sampled offset was {offsets[-1]}, expected {expected_final}")
        if any(b <= a for a, b in zip(offsets, offsets[1:])):
            failures.append(f"sampled offsets were not strictly increasing: {offsets[:10]}...")

        final = pane_scroll(pane_id)
        if final is None or final["offset_from_bottom"] != expected_final:
            failures.append(f"final offset {final and final['offset_from_bottom']} != expected {expected_final}")

        top_after = first_visible_line(pane_id)
        try:
            moved = int(top_before) - int(top_after)
        except ValueError:
            moved = None
        if moved != expected_lines:
            failures.append(
                f"visible viewport moved {moved} lines, expected {expected_lines} "
                f"({top_before!r} -> {top_after!r})"
            )

        # Frame-level proof: the rendered pane top line must walk down across intermediate frames.
        # (The raw client stream is diff-rendered, so a plain-text search cannot see this.)
        if pyte is None:
            print("note: install `pyte` to enable the rendered-frame proof")
        else:
            tops = [(t, top) for t, top in frames if top is not None]
            distinct_tops = distinct(top for _, top in tops)
            if len(distinct_tops) < 3:
                failures.append(f"client rendered only {len(distinct_tops)} distinct frames: {distinct_tops}")
            if any(b >= a for a, b in zip(distinct_tops, distinct_tops[1:])):
                failures.append(f"rendered frames did not scroll upward: {distinct_tops[:10]}...")
            if distinct_tops and distinct_tops[-1] != int(top_after):
                failures.append(
                    f"last rendered top line was {distinct_tops[-1]}, expected {top_after}"
                )
            if len(tops) >= 2:
                span = tops[-1][0] - tops[0][0]
                # The frame reader can be starved and stamp a burst of frames late, so this is a
                # sanity check for "not a single instantaneous repaint", not a pacing measurement.
                if span < 0.03:
                    failures.append(
                        f"frames moved in {span * 1000:.1f} ms; the client saw a jump "
                        f"(tops {distinct_tops[:6]}...)"
                    )
                if span > 5.0:
                    failures.append(f"frames moved over {span:.1f} s; the steps were not paced")
            if not failures:
                print(
                    f"rendered {len(distinct_tops)} frames: {distinct_tops[0]} -> {distinct_tops[-1]}"
                )

        # A scroll past the top edge must stop at max_offset_from_bottom without stepping.
        edge_state = pane_scroll(pane_id)
        top = edge_state["max_offset_from_bottom"] if edge_state else None
        if top is None:
            failures.append("scroll metrics disappeared before the edge check")
            raise SystemExit(report(failures))
        sock_call("pane.scroll", {"pane_id": pane_id, "offset_from_bottom": top})
        edge_events = ScrollEvents(pane_id)
        lab.cli("plugin", "action", "invoke", "page-up", "--plugin", PLUGIN)
        time.sleep(1.0)
        edge_events.close()
        if edge_events.events:
            failures.append(f"edge scroll emitted {len(edge_events.events)} steps")
        edge_after = pane_scroll(pane_id)
        if edge_after is None or edge_after["offset_from_bottom"] != top:
            failures.append("edge scroll moved past max_offset_from_bottom")

        log = state_log_text()[len(log_before):]
        if "kind=Halfpage" not in log or f"steps={expected_lines}" not in log:
            failures.append(f"state log did not record the halfpage run: {log[-200:]!r}")
    finally:
        lab.close()
    return report(failures)


def report(failures):
    if failures:
        print("\nFAIL")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("OK: smooth scroll verified end to end")
    return 0


if __name__ == "__main__":
    sys.exit(main())
