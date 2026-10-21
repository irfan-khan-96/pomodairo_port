"""Synthesises Pomodairo's built-in sounds (a "soft digital" set matching the
seven-segment clock) into src-tauri/resources/sounds/.

    python scripts/make-sounds.py

Standard library only; output is deterministic, so the WAVs can be regenerated
and diffed. Every sound is generated here, so the app has no third-party audio.

- tick.wav       2 s loop: a soft quartz "tick" and a slightly lower "tock".
- alarm.wav      Pomodoro over: three pairs of short, rounded digital beeps.
- break-end.wav  Break over: a gentle rising two-note chime (E6 -> A6), so it is
                 distinguishable from the alarm without looking.
"""

import math
import random
import struct
import wave
from pathlib import Path

RATE = 44_100
OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "resources" / "sounds"


def silence(seconds: float) -> list[float]:
    return [0.0] * round(seconds * RATE)


def mix_into(buf: list[float], sound: list[float], at_seconds: float) -> None:
    start = round(at_seconds * RATE)
    for i, s in enumerate(sound):
        if start + i < len(buf):
            buf[start + i] += s


def tone(freq: float, seconds: float, partials=((1, 1.0),), attack=0.004, release=0.004, decay=None) -> list[float]:
    """Additive tone. `partials` are (harmonic number, amplitude); `decay` is an
    exponential time constant in seconds (None = sustained until the release)."""
    n = round(seconds * RATE)
    out = []
    for i in range(n):
        t = i / RATE
        s = sum(a * math.sin(2 * math.pi * freq * h * t) for h, a in partials)
        env = min(1.0, t / attack) if attack else 1.0
        env *= min(1.0, (seconds - t) / release) if release else 1.0
        if decay:
            env *= math.exp(-t / decay)
        out.append(s * env)
    return out


def click(pitch: float, rng: random.Random) -> list[float]:
    """A soft mechanical tick: a short resonant ping, a lower body, a noise transient."""
    n = round(0.06 * RATE)
    out = []
    for i in range(n):
        t = i / RATE
        ping = math.sin(2 * math.pi * pitch * t) * math.exp(-t / 0.0035)
        body = 0.6 * math.sin(2 * math.pi * pitch * 0.44 * t) * math.exp(-t / 0.008)
        noise = 0.3 * rng.uniform(-1, 1) * math.exp(-t / 0.0012)
        attack = min(1.0, t / 0.0003)
        out.append((ping + body + noise) * attack)
    return out


def normalise(buf: list[float], peak: float) -> list[float]:
    top = max(abs(s) for s in buf) or 1.0
    return [s * peak / top for s in buf]


def write(name: str, buf: list[float]) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / name
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", round(max(-1.0, min(1.0, s)) * 32767)) for s in buf))
    print(f"wrote {path.relative_to(OUT.parent.parent.parent)} ({len(buf) / RATE:.2f} s)")


def make_tick() -> list[float]:
    rng = random.Random(25)  # fixed seed: identical output on every run
    buf = silence(2.0)
    mix_into(buf, click(2600, rng), 0.0)
    mix_into(buf, click(2200, rng), 1.0)
    return normalise(buf, 0.30)


def make_alarm() -> list[float]:
    # Odd harmonics give the "digital" edge; kept low so it isn't shrill.
    beep = tone(1760, 0.08, partials=((1, 1.0), (3, 0.18), (5, 0.05)), attack=0.004, release=0.012)
    buf = silence(1.65)
    for pair in range(3):
        at = pair * 0.52
        mix_into(buf, beep, at)
        mix_into(buf, beep, at + 0.14)
    return normalise(buf, 0.45)


def make_break_end() -> list[float]:
    chime = ((1, 1.0), (2, 0.3), (3, 0.08))  # bell-ish
    buf = silence(1.4)
    mix_into(buf, tone(1318.5, 0.6, chime, attack=0.006, release=0.05, decay=0.12), 0.0)
    mix_into(buf, tone(1760.0, 1.2, chime, attack=0.006, release=0.1, decay=0.28), 0.16)
    return normalise(buf, 0.45)


if __name__ == "__main__":
    write("tick.wav", make_tick())
    write("alarm.wav", make_alarm())
    write("break-end.wav", make_break_end())
