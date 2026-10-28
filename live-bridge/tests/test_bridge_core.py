# Unit tests for bridge_core with a fake Song. Run from tools/live-bridge:
#   python -m unittest discover -s tests
# Record building and output only; not a Live measurement.

import json
import os
import socket
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "OmoiIroBridge"))
import bridge_core  # noqa: E402


class Smpte(object):
    def __init__(self, ms):
        self.hours, rest = divmod(ms, 3600000)
        self.minutes, rest = divmod(rest, 60000)
        self.seconds, self.frames = divmod(rest, 1000)


class TimeFormat(object):
    ms_time = "ms_time"


class FakeSong(object):
    current_song_time = 12.5
    tempo = 128.0
    is_playing = True
    signature_numerator = 3
    signature_denominator = 4
    loop = True
    loop_start = 16.0
    loop_length = 8.0

    def get_current_smpte_song_time(self, fmt):
        assert fmt == "ms_time"
        return Smpte(3723456)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class RecordTest(unittest.TestCase):
    def test_spec_fields(self):
        r = bridge_core.make_record(7, "listener", "song_time", 99, FakeSong(), TimeFormat)
        self.assertEqual(r["v"], 2)
        self.assertEqual(r["song_ms"], 3723456)
        self.assertEqual(r["beats"], 12.5)
        self.assertEqual(r["tempo"], 128.0)
        self.assertIs(r["is_playing"], True)
        self.assertEqual(r["sig"], [3, 4])
        self.assertEqual(r["loop"], {"on": True, "start_beats": 16.0, "length_beats": 8.0})
        self.assertEqual(r["errors"], [])
        self.assertEqual((r["seq"], r["kind"], r["event"], r["perf_ns"]), (7, "listener", "song_time", 99))
        self.assertLessEqual(len(bridge_core.encode(r)), bridge_core.MAX_DATAGRAM)

    def test_failed_reads_are_named_in_errors(self):
        class Partial(FakeSong):
            def get_current_smpte_song_time(self, fmt):
                raise RuntimeError("x")

            @property
            def loop(self):
                raise AttributeError("loop")

        r = bridge_core.make_record(1, "timer", "tick", 0, Partial(), TimeFormat)
        self.assertEqual(r["errors"], ["song_ms", "loop"])
        self.assertIsNone(r["song_ms"])
        self.assertIsNone(r["loop"])
        self.assertEqual(r["sig"], [3, 4])

    def test_oversize_is_dropped(self):
        self.assertIsNone(bridge_core.encode({"x": "a" * 600}))


class OutputTest(unittest.TestCase):
    def test_udp_and_debug_log_off_by_default(self):
        rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        rx.bind(("127.0.0.1", 0))
        rx.settimeout(1.0)
        out = bridge_core.Output(port=rx.getsockname()[1])
        self.assertIsNone(out.file)
        out.emit({"seq": 1})
        self.assertEqual(json.loads(rx.recv(1024)), {"seq": 1})
        out.emit({"x": "a" * 600})
        self.assertEqual(out.oversize, 1)
        out.close()
        rx.close()

    def test_debug_log_is_buffered(self):
        now = [0]
        path = os.path.join(tempfile.mkdtemp(), "d", "b.jsonl")
        out = bridge_core.Output(port=9, log_path=path, clock_ns=lambda: now[0])
        out.emit({"seq": 1})
        out.maybe_flush()
        self.assertEqual(read(path), "")
        now[0] = bridge_core.FLUSH_INTERVAL_NS
        out.maybe_flush()
        self.assertEqual(read(path).splitlines(), ['{"seq":1}'])
        out.close()


if __name__ == "__main__":
    unittest.main()
