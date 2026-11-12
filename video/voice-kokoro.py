#!/usr/bin/env python3
"""Narration for the demo video with Kokoro (free, offline, Apache-2.0 TTS).

  pip install kokoro-onnx soundfile
  # model files from github.com/thewh1teagle/kokoro-onnx releases (model-files-v1.0):
  #   kokoro-v1.0.onnx and voices-v1.0.bin
  KOKORO_DIR=/path/to/models python3 video/voice-kokoro.py OUT_DIR [VOICE] [SPEED]

Writes OUT_DIR/01.wav, 02.wav, ... one per line of video/voice-lines.tsv.
VOICE defaults to af_heart, SPEED to 0.88 (slower than normal, easier to follow).
Then: python3 video/mix-voice.py VIDEO.mp4 OUT_DIR OUT.mp4
"""
import os
import pathlib
import sys

import soundfile as sf
from kokoro_onnx import Kokoro


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    out = pathlib.Path(sys.argv[1])
    voice = sys.argv[2] if len(sys.argv) > 2 else "af_heart"
    speed = float(sys.argv[3]) if len(sys.argv) > 3 else 0.88
    models = pathlib.Path(os.environ.get("KOKORO_DIR", "."))
    tts = Kokoro(str(models / "kokoro-v1.0.onnx"), str(models / "voices-v1.0.bin"))
    out.mkdir(parents=True, exist_ok=True)
    tsv = pathlib.Path(__file__).with_name("voice-lines.tsv")
    lines = [l.split("\t", 1)[1] for l in tsv.read_text().splitlines() if l.strip()]
    for i, text in enumerate(lines, 1):
        samples, rate = tts.create(text, voice=voice, speed=speed, lang="en-us")
        sf.write(out / f"{i:02d}.wav", samples, rate)
        print(f"{i:02d}.wav  {len(samples) / rate:5.2f}s  {text[:60]}")


if __name__ == "__main__":
    main()
