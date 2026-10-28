import time

import Live
from _Framework.ControlSurface import ControlSurface

from . import bridge_core

# Debug only: also append every record to ~/OmoiIroBridge/bridge-<time>.jsonl.
DEBUG_LOG = False
HOST = bridge_core.DEFAULT_HOST
PORT = bridge_core.DEFAULT_PORT


class OmoiIroBridge(ControlSurface):
    """Sends Live's song time, beats, tempo, transport, signature and loop,
    and (0.1.3) the output meters of the master and of up to four tracks
    named `[LED] ...` about 30 times a second.

    Read-only. Callbacks build one small dict and send one datagram.
    """

    def __init__(self, c_instance):
        ControlSurface.__init__(self, c_instance)
        self._seq = 0
        self._out = None
        self._listening = False
        self._meter_tracks = []
        self._last_meter_ns = 0
        self._last_scan_ns = 0
        self._meter_listening = False
        # Meter problems are logged once each (they would repeat 30 times a second).
        self._meter_logged = set()
        try:
            self._time_format = Live.Song.TimeFormat
            self._out = bridge_core.Output(HOST, PORT, bridge_core.debug_log_path() if DEBUG_LOG else None)
            song = self.song()
            song.add_current_song_time_listener(self._on_song_time)
            song.add_is_playing_listener(self._on_is_playing)
            song.add_tempo_listener(self._on_tempo)
            self._listening = True
            self._emit("meta", "start")
        except Exception as e:  # noqa: BLE001 - never break Live
            self._log_error("init", e)
        try:
            # The master meter changes many times a second while sound plays:
            # its listener paces the meter records (thinned to ~30 per second).
            self.song().master_track.add_output_meter_left_listener(self._on_meter)
            self._meter_listening = True
            self._scan_tracks()
        except Exception as e:  # noqa: BLE001
            self._log_meter_once("meter init", e)

    def _scan_tracks(self):
        self._last_scan_ns = time.perf_counter_ns()
        try:
            self._meter_tracks = bridge_core.led_tracks(self.song().tracks)
        except Exception as e:  # noqa: BLE001
            self._meter_tracks = []
            self._log_meter_once("track scan", e)

    def _send_meter(self, now_ns):
        if self._out is None:
            return
        self._last_meter_ns = now_ns
        try:
            self._out.emit(bridge_core.make_meter(now_ns, self.song().master_track, self._meter_tracks))
        except Exception as e:  # noqa: BLE001
            self._log_meter_once("meter", e)

    def _log_meter_once(self, where, e):
        if where not in self._meter_logged:
            self._meter_logged.add(where)
            self._log_error(where + " (logged once)", e)

    # Its own listener: the song-time listener above is not slowed by meters.
    def _on_meter(self):
        try:
            now = time.perf_counter_ns()
            if now - self._last_meter_ns >= bridge_core.METER_INTERVAL_NS:
                self._send_meter(now)
        except Exception as e:  # noqa: BLE001
            self._log_meter_once("meter listener", e)

    def _emit(self, kind, event):
        if self._out is None:
            return
        try:
            self._seq += 1
            rec = bridge_core.make_record(self._seq, kind, event, time.perf_counter_ns(), self.song(), self._time_format)
            self._out.emit(rec)
        except Exception as e:  # noqa: BLE001
            self._log_error(event, e)

    def _on_song_time(self):
        self._emit("listener", "song_time")

    def _on_is_playing(self):
        self._emit("listener", "is_playing")

    def _on_tempo(self):
        self._emit("listener", "tempo")

    # Live calls update_display about every 100 ms: liveness while stopped,
    # and the only position source while Arrangement recording (P0 finding).
    def update_display(self):
        try:
            ControlSurface.update_display(self)
        except Exception as e:  # noqa: BLE001
            self._log_error("update_display", e)
        self._emit("timer", "tick")
        now = time.perf_counter_ns()
        if now - self._last_scan_ns >= bridge_core.TRACK_SCAN_NS:
            self._scan_tracks()
        if now - self._last_meter_ns >= bridge_core.METER_FALLBACK_NS:
            self._send_meter(now)
        if self._out is not None:
            try:
                self._out.maybe_flush()
            except Exception as e:  # noqa: BLE001
                self._log_error("flush", e)

    def disconnect(self):
        try:
            if self._listening:
                song = self.song()
                for remove, has, cb in (
                    (song.remove_current_song_time_listener, song.current_song_time_has_listener, self._on_song_time),
                    (song.remove_is_playing_listener, song.is_playing_has_listener, self._on_is_playing),
                    (song.remove_tempo_listener, song.tempo_has_listener, self._on_tempo),
                ):
                    if has(cb):
                        remove(cb)
                self._listening = False
            if self._meter_listening:
                master = song.master_track
                if master.output_meter_left_has_listener(self._on_meter):
                    master.remove_output_meter_left_listener(self._on_meter)
                self._meter_listening = False
            self._emit("meta", "stop")
            if self._out is not None:
                self._out.close()
                self._out = None
        except Exception as e:  # noqa: BLE001
            self._log_error("disconnect", e)
        try:
            ControlSurface.disconnect(self)
        except Exception as e:  # noqa: BLE001
            self._log_error("disconnect", e)

    def _log_error(self, where, e):
        try:
            self.log_message("OmoiIroBridge error in %s: %r" % (where, e))
        except Exception:  # noqa: BLE001
            pass
