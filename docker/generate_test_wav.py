#!/usr/bin/env python3
"""Generate a 3-second test WAV file (16kHz mono 16-bit) with a tone burst."""
import struct
import wave
import math

RATE = 16000
DURATION = 3.0  # seconds
FREQ = 440  # Hz

samples = int(RATE * DURATION)
with wave.open("docker/test.wav", "w") as f:
    f.setnchannels(1)
    f.setsampwidth(2)
    f.setframerate(RATE)
    for i in range(samples):
        # Tone for first 2 seconds, then silence
        if i < int(RATE * 2):
            sample = int(8000 * math.sin(2 * math.pi * FREQ * i / RATE))
        else:
            sample = 0
        f.writeframes(struct.pack("<h", sample))

print(f"Generated docker/test.wav: {samples} samples, {DURATION}s, {RATE}Hz mono 16-bit")
