# OmoiIroBridge core: record building and output. No Live imports, so it can be
# unit tested with a normal Python 3. Spec: docs/10-live-bridge.md.
#
# Read-only: nothing here changes Live's transport, tempo, loop or selection.

import json
import os
import socket
import time

FORMAT_VERSION = 2
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 9007
MAX_DATAGRAM = 512
FLUSH_EVERY = 64
FLUSH_INTERVAL_NS = 1000000000


def song_ms(t):
    """Milliseconds from a SmpteTime read with TimeFormat.ms_time.

    Live 12.4.7: attributes hours, minutes, seconds, frames; frames carries
    milliseconds for ms_time (confirmed 2026-10-28).
    """
    return ((int(t.hours) * 60 + int(t.minutes)) * 60 + int(t.seconds)) * 1000 + int(t.frames)


def make_record(seq, kind, event, perf_ns, song, time_format):
    """Build one record from `song` (Live.Song.Song or a test double).

    Failed reads are listed by field name in `errors` and the field is null.
    """
    rec = {"v": FORMAT_VERSION, "seq": seq, "kind": kind, "event": event, "perf_ns": perf_ns}
    errors = []

    def read(name, fn):
        try:
            rec[name] = fn()
        except Exception:  # noqa: BLE001 - never raise into Live
            rec[name] = None
            errors.append(name)

    read("song_ms", lambda: song_ms(song.get_current_smpte_song_time(time_format.ms_time)))
    read("beats", lambda: float(song.current_song_time))
    read("tempo", lambda: float(song.tempo))
    read("is_playing", lambda: bool(song.is_playing))
    read("sig", lambda: [int(song.signature_numerator), int(song.signature_denominator)])
    read("loop", lambda: {
        "on": bool(song.loop),
        "start_beats": float(song.loop_start),
        "length_beats": float(song.loop_length),
    })
    rec["errors"] = errors
    return rec


def encode(rec):
    """Compact JSON, or None when it would exceed MAX_DATAGRAM bytes."""
    data = json.dumps(rec, separators=(",", ":")).encode("utf-8")
    return data if len(data) <= MAX_DATAGRAM else None


class Output(object):
    """UDP send per record, optional buffered JSONL (debug). Never raises."""

    def __init__(self, host=DEFAULT_HOST, port=DEFAULT_PORT, log_path=None, clock_ns=time.perf_counter_ns):
        self.addr = (host, port)
        self.clock_ns = clock_ns
        self.sent = 0
        self.send_errors = 0
        self.oversize = 0
        self.buffer = []
        self.last_flush_ns = clock_ns()
        self.sock = None
        self.file = None
        try:
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.sock.setblocking(False)
        except Exception:  # noqa: BLE001
            self.sock = None
        if log_path:
            try:
                d = os.path.dirname(log_path)
                if d and not os.path.isdir(d):
                    os.makedirs(d)
                self.file = open(log_path, "a", encoding="utf-8")
            except Exception:  # noqa: BLE001
                self.file = None

    def emit(self, rec):
        try:
            data = encode(rec)
        except Exception:  # noqa: BLE001
            data = None
        if data is None:
            self.oversize += 1
            return
        if self.sock is not None:
            try:
                self.sock.sendto(data, self.addr)
                self.sent += 1
            except Exception:  # noqa: BLE001 - no listener, buffer full, ...
                self.send_errors += 1
        if self.file is not None:
            self.buffer.append(data.decode("utf-8"))

    def maybe_flush(self):
        now = self.clock_ns()
        if len(self.buffer) >= FLUSH_EVERY or (self.buffer and now - self.last_flush_ns >= FLUSH_INTERVAL_NS):
            self.flush()

    def flush(self):
        self.last_flush_ns = self.clock_ns()
        if self.file is None or not self.buffer:
            self.buffer = []
            return
        try:
            self.file.write("\n".join(self.buffer) + "\n")
            self.file.flush()
        except Exception:  # noqa: BLE001
            pass
        self.buffer = []

    def close(self):
        self.flush()
        for h in (self.file, self.sock):
            try:
                if h is not None:
                    h.close()
            except Exception:  # noqa: BLE001
                pass
        self.file = None
        self.sock = None


def debug_log_path():
    stamp = time.strftime("%Y%m%d-%H%M%S")
    return os.path.join(os.path.expanduser("~"), "OmoiIroBridge", "bridge-%s.jsonl" % stamp)
