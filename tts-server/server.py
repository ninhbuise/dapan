"""Local Vietnamese TTS server for Dapan, using VieNeu-TTS v3 Turbo on the GPU.

    POST /tts     {"text": "...", "voice": "Mai Anh"}  ->  audio/wav (48 kHz mono)
    GET  /voices  ->  {"voices": [...], "default": "..."}
    GET  /health  ->  {"ok": true}

Cloned voices: put a clean 3-8 second WAV clip in tts-server/voices/<name>.wav and
restart; use <name> as the voice. Only clone voices you have permission to use.

Settings (environment variables):
    VIENEU_HOST   default 127.0.0.1 (only this PC can reach it)
    VIENEU_PORT   default 5005
    VIENEU_VOICE  default voice, default "Trúc Ly"
    VIENEU_KEEPWARM  seconds between keep-warm runs while idle, default 20 (0 = off)
"""

import io
import json
import os
import sys
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
from vieneu import Vieneu

HOST = os.environ.get("VIENEU_HOST", "127.0.0.1")
PORT = int(os.environ.get("VIENEU_PORT", "5005"))
DEFAULT_VOICE = os.environ.get("VIENEU_VOICE", "Trúc Ly")
VOICES_DIR = Path(__file__).with_name("voices")
MAX_TEXT = 1000
KEEPWARM = float(os.environ.get("VIENEU_KEEPWARM", "20"))
last_use = time.time()


def log(msg: str) -> None:
    print(time.strftime("%Y-%m-%d %H:%M:%S"), "[tts-server]", msg, flush=True)


try:
    import torch

    if torch.cuda.is_available():
        log(f"device: CUDA {torch.cuda.get_device_name(0)} (torch {torch.__version__})")
    else:
        log(f"device: CPU only - CUDA not available (torch {torch.__version__}); synthesis will be slow")
except Exception as e:  # noqa: BLE001
    log(f"could not check torch device: {e}")

log("loading VieNeu-TTS (first start downloads the model)...")
t0 = time.time()
tts = Vieneu()
SAMPLE_RATE = int(getattr(tts, "sample_rate", 0) or 48000)
lock = threading.Lock()  # one synthesis at a time on the GPU

preset = [voice_id for _label, voice_id in tts.list_preset_voices()]
cloned = []
if VOICES_DIR.is_dir():
    for clip in sorted(VOICES_DIR.glob("*.wav")):
        try:
            tts.add_voice(clip.stem, str(clip))
            cloned.append(clip.stem)
        except Exception as e:  # noqa: BLE001
            log(f"could not add voice {clip.name}: {e}")
ALL_VOICES = preset + cloned
log(f"ready in {time.time() - t0:.1f}s: {len(preset)} preset + {len(cloned)} cloned voices, {SAMPLE_RATE} Hz")

# Warm up so the first real request is fast.
try:
    with lock:
        tts.infer("Xin chào.", voice=DEFAULT_VOICE)
except Exception as e:  # noqa: BLE001
    log(f"warm-up failed (is VIENEU_VOICE '{DEFAULT_VOICE}' valid?): {e}")


def keep_warm() -> None:
    """While idle, run a tiny synthesis now and then so the GPU stays ready.

    Without this, the first request after a quiet minute takes 3-4 s instead of ~0.3 s."""
    global last_use
    while True:
        time.sleep(KEEPWARM / 2)
        if time.time() - last_use < KEEPWARM:
            continue
        if not lock.acquire(blocking=False):
            continue  # a real request is running
        try:
            tts.infer("ừ.", voice=DEFAULT_VOICE)
        except Exception as e:  # noqa: BLE001
            log(f"keep-warm failed: {e}")
        finally:
            last_use = time.time()
            lock.release()


if KEEPWARM > 0:
    threading.Thread(target=keep_warm, daemon=True).start()


def to_wav(audio) -> bytes:
    data = np.asarray(audio, dtype=np.float32).flatten()
    pcm = (np.clip(data, -1.0, 1.0) * 32767).astype("<i2").tobytes()
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm)
    return buf.getvalue()


class Handler(BaseHTTPRequestHandler):
    def _json(self, code: int, obj) -> None:
        body = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self._json(200, {"ok": True})
        elif self.path == "/voices":
            self._json(200, {"voices": ALL_VOICES, "default": DEFAULT_VOICE})
        else:
            self._json(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/tts":
            return self._json(404, {"error": "not found"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
            req = json.loads(self.rfile.read(length) or b"{}")
            text = str(req.get("text", "")).strip()[:MAX_TEXT]
            voice = req.get("voice") or DEFAULT_VOICE
            if not text:
                return self._json(400, {"error": "empty text"})
            if voice not in ALL_VOICES:
                return self._json(400, {"error": f"unknown voice '{voice}'", "voices": ALL_VOICES})
            global last_use
            t = time.time()
            with lock:
                audio = tts.infer(text, voice=voice)
                last_use = time.time()
            wav = to_wav(audio)
            secs = len(np.asarray(audio).flatten()) / SAMPLE_RATE
            log(f"{len(text)} chars -> {secs:.1f}s audio in {time.time() - t:.2f}s ({voice})")
        except Exception as e:  # noqa: BLE001
            log(f"error: {e}")
            return self._json(500, {"error": str(e)})
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(wav)))
        self.end_headers()
        self.wfile.write(wav)

    def log_message(self, *args):  # keep the log quiet; we log our own lines
        pass


if __name__ == "__main__":
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    log(f"listening on http://{HOST}:{PORT}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        sys.exit(0)
