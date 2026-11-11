#!/usr/bin/env bash
# Narration for the demo video with Piper (free, offline TTS):
#   PIPER=/path/to/piper PIPER_MODEL=/path/to/en-us-libritts-high.onnx [PIPER_SPEAKER=n] video/voice.sh OUT_DIR
# Writes OUT_DIR/01.wav, 02.wav, ... one per line of video/voice-lines.tsv.
# Then: python3 video/mix-voice.py VIDEO.mp4 OUT_DIR OUT.mp4
set -euo pipefail
: "${PIPER:?set PIPER to the piper binary}" "${PIPER_MODEL:?set PIPER_MODEL to a voice .onnx}"
out="${1:?usage: video/voice.sh OUT_DIR}"
mkdir -p "$out"
i=0
while IFS=$'\t' read -r _ text; do
  [ -n "$text" ] || continue
  i=$((i + 1))
  printf '%s\n' "$text" | "$PIPER" -m "$PIPER_MODEL" ${PIPER_SPEAKER:+-s "$PIPER_SPEAKER"} \
    --length_scale 0.95 --sentence_silence 0.2 -f "$out/$(printf %02d $i).wav" >/dev/null
done < "$(dirname "$0")/voice-lines.tsv"
echo "voice: $i lines in $out"
