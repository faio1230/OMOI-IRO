# OmoiIroBridge: Live 12 MIDI Remote Script that sends the song time sample
# stream used as the application's common clock (docs/10-live-bridge.md).
# Read-only. Select it as a Control Surface with Input / Output = None.

from .OmoiIroBridge import OmoiIroBridge


def create_instance(c_instance):
    return OmoiIroBridge(c_instance)
