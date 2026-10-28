# DJSCBridgeProbe: Live 12 MIDI Remote Script for the bridge measurement.
#
# Reads the song time (ms_time and smpte_30), beats, tempo and is_playing
# on Song listeners and on the ~100 ms display timer, and sends one JSON
# datagram per record to 127.0.0.1:9007 plus a buffered local JSONL file.
# It never changes Live's transport, tempo or anything else.
#
# Select it in Preferences > Link, Tempo & MIDI > Control Surface with
# Input / Output = None.

from .DJSCBridgeProbe import DJSCBridgeProbe


def create_instance(c_instance):
    return DJSCBridgeProbe(c_instance)
