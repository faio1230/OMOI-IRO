# Synthetic OmoiIroBridge sender for checks without Live. Sends v2 records built
# by bridge_core.make_record from a simulated transport, at ~60 Hz while
# playing (listener) plus a 100 ms timer, to 127.0.0.1:9007 by default.
#
# Scenario (seconds of wall time):
#   0.0  stopped at 0 ms (timer only)
#   1.0  play from 0
#   4.0  seek to 30000 ms (jump while playing)
#   6.0  loop wrap back to 20000 ms
#   8.0  stop (position kept)
#   9.0  play again
#  10.0  silence 0.6 s (bridge lost; no datagrams)
#  10.6  resume
#  12.0  stop, then return to 0 ms at 12.5
#  13.0  end
#
# This is a synthetic source. Results obtained with it are not Live results.
# Usage: python fake_bridge.py [--port 9007]
#        python fake_bridge.py --steady SECONDS [--start-ms MS]
#   --steady: play from --start-ms for SECONDS without events (long runs,
#   e.g. memory soak tests), then stop.
#   --meter: also send v3 meter records at ~30 Hz while playing: the master
#   and a track "Kick" pulsing at 120 bpm (synthetic levels, 0.1.3).

import argparse
import math
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "OmoiIroBridge"))
import bridge_core  # noqa: E402


class TimeFormat(object):
    ms_time = "ms_time"


class Smpte(object):
    def __init__(self, ms):
        ms = int(ms)
        self.hours, rest = divmod(ms, 3600000)
        self.minutes, rest = divmod(rest, 60000)
        self.seconds, self.frames = divmod(rest, 1000)


class Transport(object):
    signature_numerator = 4
    signature_denominator = 4
    loop = False
    loop_start = 0.0
    loop_length = 16.0
    tempo = 120.0

    def __init__(self):
        self.is_playing = False
        self.pos_ms = 0.0
        self.t_ref = time.perf_counter()

    def now_ms(self):
        if self.is_playing:
            return self.pos_ms + (time.perf_counter() - self.t_ref) * 1000.0
        return self.pos_ms

    def play(self):
        self.pos_ms = self.now_ms()
        self.t_ref = time.perf_counter()
        self.is_playing = True

    def stop(self):
        self.pos_ms = self.now_ms()
        self.is_playing = False

    def seek(self, ms):
        self.pos_ms = ms
        self.t_ref = time.perf_counter()

    @property
    def current_song_time(self):
        return self.now_ms() / 500.0

    def get_current_smpte_song_time(self, fmt):
        return Smpte(self.now_ms())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=bridge_core.DEFAULT_PORT)
    ap.add_argument("--steady", type=float, default=None, help="play steadily for this many seconds instead of the scenario")
    ap.add_argument("--start-ms", type=float, default=0.0, help="song position at the start of --steady")
    ap.add_argument("--meter", action="store_true", help="also send synthetic v3 meter records (master + [LED] Kick)")
    args = ap.parse_args()
    out = bridge_core.Output(port=args.port)
    song = Transport()
    seq = [0]

    def emit(kind, event):
        seq[0] += 1
        out.emit(bridge_core.make_record(seq[0], kind, event, time.perf_counter_ns(), song, TimeFormat))

    if args.steady is not None:
        song.seek(args.start_ms)
        events = [(0.5, lambda: (song.play(), emit("listener", "is_playing")))]
        end_s = 0.5 + args.steady
    else:
        end_s = 13.0
    events = events if args.steady is not None else [
        (1.0, lambda: (song.play(), emit("listener", "is_playing"))),
        (4.0, lambda: song.seek(30000.0)),
        (6.0, lambda: song.seek(20000.0)),
        (8.0, lambda: (song.stop(), emit("listener", "is_playing"))),
        (9.0, lambda: (song.play(), emit("listener", "is_playing"))),
        (12.0, lambda: (song.stop(), emit("listener", "is_playing"))),
        (12.5, lambda: song.seek(0.0)),
    ]
    t0 = time.perf_counter()
    emit("meta", "start")
    next_listener = t0
    next_timer = t0
    next_meter = t0

    class Level(object):
        def __init__(self, l, r):
            self.output_meter_left, self.output_meter_right = l, r
    while True:
        t = time.perf_counter() - t0
        if t >= end_s:
            break
        while events and t >= events[0][0]:
            events.pop(0)[1]()
        silent = args.steady is None and 10.0 <= t < 10.6
        now = time.perf_counter()
        if not silent and song.is_playing and now >= next_listener:
            emit("listener", "song_time")
            next_listener = now + 1.0 / 60.0
        if not silent and now >= next_timer:
            emit("timer", "tick")
            next_timer = now + 0.1
        if args.meter and not silent and song.is_playing and now >= next_meter:
            # A kick on every beat at 120 bpm (decaying), a slow swell on the master.
            beat = (now - t0) * 2.0
            kick = math.exp(-6.0 * (beat - math.floor(beat)))
            master = 0.35 + 0.25 * math.sin(now - t0) + 0.3 * kick
            out.emit(bridge_core.make_meter(time.perf_counter_ns(), Level(master, master * 0.9), [("Kick", Level(kick, kick))]))
            next_meter = now + bridge_core.METER_INTERVAL_NS / 1e9
        time.sleep(0.002)
    emit("meta", "stop")
    out.close()
    print("sent %d records (synthetic)" % seq[0])


if __name__ == "__main__":
    main()
