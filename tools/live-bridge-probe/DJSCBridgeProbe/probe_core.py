# DJSCBridgeProbe core: record building, SMPTE parsing, buffered output.
# No Live imports, so it can be unit tested with a normal Python 3.
#
# Measurement probe only. Nothing here changes Live's transport or tempo.

import json
import os
import socket
import time

DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 9007
FLUSH_EVERY = 64          # records
FLUSH_INTERVAL_NS = 1000000000

SMPTE_FIELDS = ("hours", "minutes", "seconds", "frames")


def smpte_fields(t):
    """Read an SmpteTime-like object. Returns (dict, error or None).

    Expected attributes (Live Object Model): hours, minutes, seconds, frames.
    For TimeFormat.ms_time the `frames` field carries milliseconds. str(t) is
    always kept so a wrong attribute guess is visible in the record.
    """
    out = {"str": None}
    try:
        out["str"] = str(t)
    except Exception as e:  # noqa: BLE001 - never raise into Live
        out["str"] = None
        out["str_error"] = repr(e)
    missing = []
    for name in SMPTE_FIELDS:
        try:
            out[name] = int(getattr(t, name))
        except Exception:  # noqa: BLE001
            missing.append(name)
    return out, ("missing " + ",".join(missing)) if missing else None


def ms_from_fields(f):
    """Milliseconds from ms_time fields (frames = milliseconds)."""
    try:
        return ((f["hours"] * 60 + f["minutes"]) * 60 + f["seconds"]) * 1000 + f["frames"]
    except (KeyError, TypeError):
        return None


def make_record(seq, kind, event, perf_ns, song, time_format):
    """Build one record from `song` (Live.Song.Song or a test double).

    `time_format` is the TimeFormat namespace (Live.Song.TimeFormat), with
    `ms_time` and `smpte_30`.
    """
    rec = {"v": 1, "seq": seq, "kind": kind, "event": event, "perf_ns": perf_ns}
    errors = []
    try:
        ms, err = smpte_fields(song.get_current_smpte_song_time(time_format.ms_time))
        rec["smpte_ms_fields"] = ms
        rec["smpte_ms"] = ms_from_fields(ms)
        if err:
            errors.append("ms_time: " + err)
    except Exception as e:  # noqa: BLE001
        errors.append("ms_time: " + repr(e))
    try:
        f30, err = smpte_fields(song.get_current_smpte_song_time(time_format.smpte_30))
        rec["smpte30"] = f30
        if err:
            errors.append("smpte_30: " + err)
    except Exception as e:  # noqa: BLE001
        errors.append("smpte_30: " + repr(e))
    for key, attr in (("beats", "current_song_time"), ("tempo", "tempo"), ("is_playing", "is_playing")):
        try:
            v = getattr(song, attr)
            rec[key] = bool(v) if key == "is_playing" else float(v)
        except Exception as e:  # noqa: BLE001
            errors.append(attr + ": " + repr(e))
    if errors:
        rec["errors"] = errors
    return rec


class Output(object):
    """UDP send per record plus a buffered JSONL file. Never raises."""

    def __init__(self, host=DEFAULT_HOST, port=DEFAULT_PORT, path=None, clock_ns=time.perf_counter_ns):
        self.addr = (host, port)
        self.clock_ns = clock_ns
        self.sent = 0
        self.send_errors = 0
        self.write_errors = 0
        self.buffer = []
        self.last_flush_ns = clock_ns()
        self.sock = None
        self.file = None
        self.path = path
        try:
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.sock.setblocking(False)
        except Exception:  # noqa: BLE001
            self.sock = None
        if path:
            try:
                d = os.path.dirname(path)
                if d and not os.path.isdir(d):
                    os.makedirs(d)
                self.file = open(path, "a", encoding="utf-8")
            except Exception:  # noqa: BLE001
                self.file = None
                self.write_errors += 1

    def emit(self, rec):
        try:
            line = json.dumps(rec, separators=(",", ":"))
        except Exception:  # noqa: BLE001
            self.send_errors += 1
            return
        if self.sock is not None:
            try:
                self.sock.sendto(line.encode("utf-8"), self.addr)
                self.sent += 1
            except Exception:  # noqa: BLE001 - no listener, buffer full, ...
                self.send_errors += 1
        if self.file is not None:
            self.buffer.append(line)

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
            self.write_errors += 1
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


def default_log_path():
    stamp = time.strftime("%Y%m%d-%H%M%S")
    return os.path.join(os.path.expanduser("~"), "DJSCBridgeProbe", "bridge-%s.jsonl" % stamp)
