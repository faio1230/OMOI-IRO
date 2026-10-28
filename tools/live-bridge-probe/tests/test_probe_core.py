# Unit tests for probe_core with a fake Song. Run from tools/live-bridge-probe:
#   python -m unittest discover -s tests
# These test record building and output only; they are not a Live measurement.

import json
import os
import socket
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "DJSCBridgeProbe"))
import probe_core  # noqa: E402


class Smpte(object):
    def __init__(self, h, m, s, f):
        self.hours, self.minutes, self.seconds, self.frames = h, m, s, f

    def __str__(self):
        return "%02d:%02d:%02d:%03d" % (self.hours, self.minutes, self.seconds, self.frames)


class TimeFormat(object):
    ms_time = "ms_time"
    smpte_30 = "smpte_30"


class FakeSong(object):
    current_song_time = 12.5
    tempo = 120.0
    is_playing = True

    def get_current_smpte_song_time(self, fmt):
        if fmt == "ms_time":
            return Smpte(0, 0, 6, 250)
        return Smpte(0, 0, 6, 7)


class NoAttrSmpte(object):
    def __str__(self):
        return "0:0:6:250"


class OddSong(FakeSong):
    def get_current_smpte_song_time(self, fmt):
        return NoAttrSmpte()


class RecordTest(unittest.TestCase):
    def test_record_fields(self):
        r = probe_core.make_record(3, "listener", "song_time", 123, FakeSong(), TimeFormat)
        self.assertEqual(r["smpte_ms"], 6250)
        self.assertEqual(r["smpte30"]["frames"], 7)
        self.assertEqual(r["beats"], 12.5)
        self.assertEqual(r["tempo"], 120.0)
        self.assertIs(r["is_playing"], True)
        self.assertEqual((r["seq"], r["kind"], r["event"], r["perf_ns"]), (3, "listener", "song_time", 123))
        self.assertNotIn("errors", r)
        json.dumps(r)

    def test_unknown_attributes_keep_str_and_report_error(self):
        r = probe_core.make_record(1, "timer", "tick", 0, OddSong(), TimeFormat)
        self.assertIsNone(r["smpte_ms"])
        self.assertEqual(r["smpte_ms_fields"]["str"], "0:0:6:250")
        self.assertTrue(any("missing hours" in e for e in r["errors"]))

    def test_exceptions_are_captured(self):
        class Broken(FakeSong):
            def get_current_smpte_song_time(self, fmt):
                raise RuntimeError("boom")
        r = probe_core.make_record(1, "timer", "tick", 0, Broken(), TimeFormat)
        self.assertEqual(len(r["errors"]), 2)
        self.assertEqual(r["beats"], 12.5)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class OutputTest(unittest.TestCase):
    def test_udp_and_buffered_file(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(1.0)
        port = rx.getsockname()[1]
        d = tempfile.mkdtemp()
        path = os.path.join(d, "sub", "b.jsonl")
        now = [0]
        out = probe_core.Output(port=port, path=path, clock_ns=lambda: now[0])
        out.emit({"seq": 1})
        out.emit({"seq": 2})
        self.assertEqual(json.loads(rx.recv(65536)), {"seq": 1})
        self.assertEqual(json.loads(rx.recv(65536)), {"seq": 2})
        out.maybe_flush()  # neither count nor interval reached
        self.assertEqual(read(path), "")
        now[0] = probe_core.FLUSH_INTERVAL_NS
        out.maybe_flush()
        self.assertEqual(read(path).splitlines(), ['{"seq":1}', '{"seq":2}'])
        out.close()
        rx.close()

    def test_no_listener_does_not_raise(self):
        out = probe_core.Output(port=9, path=None)
        for i in range(5):
            out.emit({"seq": i})
        out.close()


if __name__ == "__main__":
    unittest.main()
