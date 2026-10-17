"""Historical frame-buffered observer atop the shared real PTY transport.

The production controller observes complete VT tokens in Rust. This research
observer intentionally retains its original raw-frame-boundary contract.
"""
from terminal import Terminal as LiveTerminal


class Terminal(LiveTerminal):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.pending_frame = bytearray()
        self.frame_serial = 0

    def feed_output(self, data):
        self.raw.extend(data)
        self.pending_frame.extend(data)
        while b'\x1b[?2026l' in self.pending_frame:
            boundary = self.pending_frame.index(b'\x1b[?2026l') + len(b'\x1b[?2026l')
            frame = self.pending_frame[:boundary]
            del self.pending_frame[:boundary]
            self.stream.feed(self.decoder.decode(frame))
            self.frame_serial += 1
