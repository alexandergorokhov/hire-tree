# Voice lines

`01.wav` to `13.wav`: the lines of `../voice-lines.tsv`, in order. Made offline and free with Kokoro (kokoro-onnx, Apache-2.0, model-files-v1.0), voice `af_heart`, speed 0.79: `KOKORO_DIR=... python3 video/voice-kokoro.py video/voice af_heart 0.79`. "A.I." and "M C P" are spelled so they are read as letters. `../voice-samples/` holds line 2 in five other voices.

The video timing follows these files: `scripts/clip.js` holds each scene until its line has been spoken and writes `out/clip/voice-starts.tsv`; then `python3 video/mix-voice.py VIDEO video/voice OUT out/clip/voice-starts.tsv`.
