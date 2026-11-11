#!/usr/bin/env python3
"""Lay the narration under the demo video.

  python3 video/mix-voice.py VIDEO.mp4 VOICE_DIR OUT.mp4

VOICE_DIR holds 01.wav, 02.wav, ... one per line of video/voice-lines.tsv
(make them with video/voice.sh). Each line starts at its time in the tsv, or
just after the line before it ends, whichever is later; if the last line runs
past the video, the last frame is held. Needs ffmpeg and ffprobe.
"""
import pathlib
import subprocess
import sys

GAP = 0.25  # seconds of silence kept between two lines


def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                          "-of", "csv=p=0", str(path)], capture_output=True, text=True, check=True)
    return float(out.stdout.strip())


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    video, voice_dir, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
    tsv = pathlib.Path(__file__).with_name("voice-lines.tsv")
    starts = [float(line.split("\t", 1)[0]) for line in tsv.read_text().splitlines() if line.strip()]
    clips, t = [], 0.0
    for i, want in enumerate(starts, 1):
        wav = voice_dir / f"{i:02d}.wav"
        at = max(want, t + GAP if clips else want)
        d = duration(wav)
        clips.append((wav, at))
        late = at - want
        print(f"line {i:2d}  {at:6.2f}s  {d:5.2f}s long" + (f"  ({late:.1f}s late)" if late > 0.05 else ""))
        t = at + d
    vlen = duration(video)
    hold = max(0.0, t + 1.0 - vlen)
    inputs, parts = ["-i", str(video)], []
    for n, (wav, at) in enumerate(clips, 1):
        inputs += ["-i", str(wav)]
        ms = int(at * 1000)
        parts.append(f"[{n}:a]aresample=48000,adelay={ms}|{ms}[a{n}]")
    mix = "".join(f"[a{n}]" for n in range(1, len(clips) + 1))
    graph = ";".join(parts) + f";{mix}amix=inputs={len(clips)}:normalize=0,loudnorm=I=-16:TP=-1.5[aout]"
    vf = ["-vf", f"tpad=stop_mode=clone:stop_duration={hold:.2f}"] if hold else ["-c:v", "copy"]
    subprocess.run(["ffmpeg", "-y", "-v", "error", *inputs, "-filter_complex", graph,
                    "-map", "0:v", "-map", "[aout]", *vf, "-c:a", "aac", "-b:a", "160k",
                    "-movflags", "+faststart", out], check=True)
    print(f"{out}: video {vlen:.1f}s, narration ends {t:.1f}s" + (f", last frame held {hold:.1f}s" if hold else ""))


if __name__ == "__main__":
    main()
