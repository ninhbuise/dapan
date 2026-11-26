"""Generates a short sample for every preset voice into tts-server/samples/, so you can
listen and pick one for TTS_LOCAL_VOICE.

    .venv\\Scripts\\python.exe samples.py
"""

import time
from pathlib import Path

from vieneu import Vieneu

TEXT = "Xin chào, mình là Dapan nè! Hôm nay bạn có muốn nghe nhạc không?"
out = Path(__file__).with_name("samples")
out.mkdir(exist_ok=True)

tts = Vieneu()
for label, voice_id in tts.list_preset_voices():
    t = time.time()
    audio = tts.infer(TEXT, voice=voice_id)
    path = out / f"{voice_id}.wav"
    tts.save(audio, str(path))
    print(f"{voice_id:<16} {label:<30} {time.time() - t:.2f}s -> {path.name}")
print(f"\nDone. Open the folder: {out}")
