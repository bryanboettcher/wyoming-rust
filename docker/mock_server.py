#!/usr/bin/env python3
"""
Mock Wyoming server that simulates Home Assistant's side of the satellite protocol.
Used for testing the Rust satellite implementation.
"""
import json
import struct
import math
import time
import socket
import sys


def write_event(sock, event_type, data=None, payload=None):
    """Write a Wyoming protocol event to the socket."""
    header = {"type": event_type, "version": "1.5.2"}
    data_bytes = b""
    if data:
        data_bytes = json.dumps(data).encode("utf-8")
        header["data_length"] = len(data_bytes)
    payload_bytes = payload or b""
    if payload_bytes:
        header["payload_length"] = len(payload_bytes)
    header_line = json.dumps(header).encode("utf-8") + b"\n"
    sock.sendall(header_line + data_bytes + payload_bytes)
    print(f"→ Sent: {event_type}", end="")
    if data:
        print(f" (data: {len(data_bytes)} bytes)", end="")
    if payload_bytes:
        print(f" (payload: {len(payload_bytes)} bytes)", end="")
    print()


def read_event(sock):
    """Read a Wyoming protocol event from the socket."""
    # Read header line (until \n)
    header_line = b""
    while not header_line.endswith(b"\n"):
        chunk = sock.recv(1)
        if not chunk:
            return None
        header_line += chunk

    header = json.loads(header_line)
    data_length = header.get("data_length", 0)
    payload_length = header.get("payload_length", 0)

    data = {}
    if data_length > 0:
        data_bytes = b""
        while len(data_bytes) < data_length:
            data_bytes += sock.recv(data_length - len(data_bytes))
        data = json.loads(data_bytes)

    payload = b""
    if payload_length > 0:
        while len(payload) < payload_length:
            payload += sock.recv(payload_length - len(payload))

    return {"type": header["type"], "data": data, "payload": payload}


def generate_sine_pcm(freq=440, duration_ms=100, rate=16000):
    """Generate PCM audio data (16-bit mono) containing a sine wave."""
    samples = int(rate * duration_ms / 1000)
    pcm = b""
    for i in range(samples):
        sample = int(16000 * math.sin(2 * math.pi * freq * i / rate))
        pcm += struct.pack("<h", sample)
    return pcm


def handle_satellite(conn, addr):
    """Handle a single satellite connection."""
    print(f"\n{'='*60}")
    print(f"Satellite connected from {addr}")
    print(f"{'='*60}\n")

    try:
        # Step 1: Send describe event
        print("[1] Sending describe event...")
        write_event(conn, "describe")

        # Step 2: Wait for info response
        print("[2] Waiting for info response...")
        event = read_event(conn)
        if not event or event["type"] != "info":
            print(f"✗ Expected info, got: {event['type'] if event else 'None'}")
            return
        print(f"← Received: info")
        print(f"   Satellite info: {json.dumps(event['data'], indent=2)}")

        # Step 3: Send run-satellite event
        print("\n[3] Sending run-satellite event...")
        write_event(conn, "run-satellite", data={})

        # Step 4: Wait for audio-start + streaming-started (either order)
        print("[4] Waiting for audio-start and streaming-started...")
        needed = {"audio-start", "streaming-started"}
        while needed:
            event = read_event(conn)
            if not event:
                print("✗ Connection closed waiting for stream start")
                return
            if event["type"] in needed:
                print(f"← Received: {event['type']}")
                needed.discard(event["type"])
            else:
                print(f"   (ignoring {event['type']} during handshake)")

        # Step 5: Receive audio chunks
        print("\n[5] Receiving audio chunks...")
        chunk_count = 0
        total_audio_bytes = 0

        # Receive chunks until we have enough for detection (~1 second = 50 chunks @ 20ms)
        while chunk_count < 50:
            event = read_event(conn)
            if not event:
                print("✗ Connection closed during audio streaming")
                return

            if event["type"] == "audio-chunk":
                chunk_count += 1
                total_audio_bytes += len(event["payload"])
                if chunk_count % 10 == 0:
                    print(f"   Received {chunk_count} chunks ({total_audio_bytes} bytes total)")
            else:
                print(f"   Unexpected event during streaming: {event['type']}")

        print(f"   Total: {chunk_count} chunks, {total_audio_bytes} bytes\n")

        # Step 6: Simulate wake word detection
        # Note: satellite keeps streaming audio during Triggered/Processing states,
        # so we need to set a short timeout to drain incoming chunks between steps.
        print("[6] Simulating wake word detection...")
        write_event(conn, "detection", data={
            "name": "test_wake_word",
            "timestamp": int(time.time() * 1000)
        })

        # Drain incoming audio chunks briefly
        conn.settimeout(0.2)
        drained = 0
        try:
            while True:
                ev = read_event(conn)
                if ev and ev["type"] == "audio-chunk":
                    drained += 1
                elif ev:
                    print(f"← Received (during drain): {ev['type']}")
        except (socket.timeout, BlockingIOError):
            pass
        conn.settimeout(None)
        if drained:
            print(f"   (drained {drained} audio chunks during detection)")

        # Step 7: Send voice-started (pipeline processing)
        print("[7] Starting voice pipeline...")
        write_event(conn, "voice-started", data={
            "timestamp": int(time.time() * 1000)
        })

        # Drain more audio chunks while "processing"
        conn.settimeout(0.5)
        drained = 0
        try:
            while True:
                ev = read_event(conn)
                if ev and ev["type"] == "audio-chunk":
                    drained += 1
                elif ev:
                    print(f"← Received (during processing): {ev['type']}")
        except (socket.timeout, BlockingIOError):
            pass
        conn.settimeout(None)
        if drained:
            print(f"   (drained {drained} audio chunks during processing)")

        # Step 8: Send audio-start for TTS response
        # This triggers Responding state, satellite stops mic capture
        print("\n[8] Sending TTS audio response...")
        write_event(conn, "audio-start", data={
            "rate": 16000,
            "width": 2,
            "channels": 1,
            "timestamp": int(time.time() * 1000)
        })

        # Step 9: Send audio chunks with test sine wave
        print("[9] Sending audio chunks...")
        for i in range(10):
            pcm = generate_sine_pcm(freq=440 + (i * 50), duration_ms=100, rate=16000)
            write_event(conn, "audio-chunk", data={
                "rate": 16000,
                "width": 2,
                "channels": 1,
                "timestamp": int(time.time() * 1000)
            }, payload=pcm)
            time.sleep(0.1)

        # Step 10: Send audio-stop
        print("[10] Stopping TTS audio...")
        write_event(conn, "audio-stop", data={
            "timestamp": int(time.time() * 1000)
        })

        # Step 11: Send voice-stopped
        print("[11] Stopping voice pipeline...")
        write_event(conn, "voice-stopped", data={
            "timestamp": int(time.time() * 1000)
        })

        # Step 12: Wait for played + streaming-stopped (either order)
        print("[12] Waiting for played and streaming-stopped...")
        needed = {"played", "streaming-stopped"}
        while needed:
            event = read_event(conn)
            if not event:
                print("✗ Connection closed waiting for shutdown")
                return
            if event["type"] in needed:
                print(f"← Received: {event['type']}")
                needed.discard(event["type"])
            else:
                print(f"   (ignoring {event['type']} during shutdown)")
        print()

        print(f"{'='*60}")
        print("✓ Test completed successfully")
        print(f"{'='*60}\n")
        return True

    except Exception as e:
        print(f"\n✗ Error handling satellite: {e}")
        import traceback
        traceback.print_exc()
        return False
    finally:
        conn.close()


def main():
    """Run the mock Wyoming server."""
    host = "0.0.0.0"
    port = 10700

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind((host, port))
    server.listen(1)

    print(f"Mock Wyoming server listening on {host}:{port}")
    print("Waiting for satellite connection...\n")

    try:
        conn, addr = server.accept()
        success = handle_satellite(conn, addr)
        if success:
            print("Test passed, exiting.")
            sys.exit(0)
        else:
            print("Test failed, exiting.")
            sys.exit(1)
    except KeyboardInterrupt:
        print("\nShutting down server...")
    finally:
        server.close()


if __name__ == "__main__":
    main()
