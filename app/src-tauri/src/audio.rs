//! Sound playback on a dedicated thread.
//!
//! The rodio output device is not `Send` on every platform, so it lives on its own
//! thread and the rest of the app talks to it through a channel. If no audio
//! device is available the app keeps running silently.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use serde::Deserialize;

// Built-in sounds, synthesised by app/scripts/make-sounds.py.
const ALARM_WAV: &[u8] = include_bytes!("../resources/sounds/alarm.wav");
const BREAK_END_WAV: &[u8] = include_bytes!("../resources/sounds/break-end.wav");
const TICK_WAV: &[u8] = include_bytes!("../resources/sounds/tick.wav");

/// Ticking fades from this volume to silence over `TICK_FADE` (as in the original app).
const TICK_FADE_FROM: f32 = 0.7;
const TICK_FADE: Duration = Duration::from_secs(3);
const FADE_STEP: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub struct SoundSettings {
    pub enabled: bool,
    /// Fade the ticking out after a few seconds instead of ticking for the whole pomodoro.
    pub fade_ticking: bool,
    /// 0.0 – 1.0
    pub volume: f32,
    /// Custom sound files; `None` uses the built-in sounds. A custom alarm is
    /// used for both the end of a pomodoro and the end of a break.
    pub alarm_file: Option<PathBuf>,
    pub ticking_file: Option<PathBuf>,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            fade_ticking: true,
            volume: 1.0,
            alarm_file: None,
            ticking_file: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundKind {
    /// End of a pomodoro.
    Alarm,
    /// End of a break (built-in: a softer chime than the alarm).
    BreakEnd,
    Ticking,
}

enum Command {
    /// `Alarm` or `BreakEnd`.
    PlayAlarm(SoundKind),
    StartTicking,
    StopTicking,
    Configure(SoundSettings),
    /// Play a sound once from the settings window (a file, or the built-in one).
    Preview {
        kind: SoundKind,
        file: Option<PathBuf>,
        volume: f32,
    },
}

#[derive(Clone)]
pub struct Audio {
    tx: Sender<Command>,
}

impl Audio {
    pub fn spawn(settings: SoundSettings) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, settings))
            .expect("failed to spawn audio thread");
        Self { tx }
    }

    /// The sound for the end of a pomodoro.
    pub fn play_alarm(&self) {
        let _ = self.tx.send(Command::PlayAlarm(SoundKind::Alarm));
    }

    /// The sound for the end of a break.
    pub fn play_break_end(&self) {
        let _ = self.tx.send(Command::PlayAlarm(SoundKind::BreakEnd));
    }

    pub fn start_ticking(&self) {
        let _ = self.tx.send(Command::StartTicking);
    }

    pub fn stop_ticking(&self) {
        let _ = self.tx.send(Command::StopTicking);
    }

    pub fn configure(&self, settings: SoundSettings) {
        let _ = self.tx.send(Command::Configure(settings));
    }

    pub fn preview(&self, kind: SoundKind, file: Option<PathBuf>, volume: f32) {
        let _ = self.tx.send(Command::Preview { kind, file, volume });
    }
}

/// Check that `path` holds audio rodio can play.
pub fn decode_check(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Decoder::new(Cursor::new(bytes))
        .map(drop)
        .map_err(|e| e.to_string())
}

type Sound = Arc<[u8]>;

/// A custom sound file, or `None` (built-in) if unset or unreadable.
fn load_custom(file: Option<&Path>) -> Option<Sound> {
    let path = file?;
    match std::fs::read(path) {
        Ok(bytes) if Decoder::new(Cursor::new(bytes.clone())).is_ok() => Some(bytes.into()),
        Ok(_) => {
            eprintln!(
                "audio: {} is not playable, using built-in sound",
                path.display()
            );
            None
        }
        Err(e) => {
            eprintln!(
                "audio: cannot read {}: {e}, using built-in sound",
                path.display()
            );
            None
        }
    }
}

fn builtin(kind: SoundKind) -> Sound {
    Arc::from(match kind {
        SoundKind::Alarm => ALARM_WAV,
        SoundKind::BreakEnd => BREAK_END_WAV,
        SoundKind::Ticking => TICK_WAV,
    })
}

/// The custom alarm (used for both alarm kinds) or the built-in sound for `kind`.
fn alarm_for(custom: &Option<Sound>, kind: SoundKind) -> Sound {
    custom.clone().unwrap_or_else(|| builtin(kind))
}

struct Ticking {
    player: Player,
    fade_started: Option<Instant>,
    /// Volume the fade starts from.
    volume: f32,
}

fn run(rx: Receiver<Command>, settings: SoundSettings) {
    let sink: Option<MixerDeviceSink> = match DeviceSinkBuilder::open_default_sink() {
        Ok(mut sink) => {
            sink.log_on_drop(false);
            Some(sink)
        }
        Err(err) => {
            eprintln!("audio: no output device, sounds disabled: {err}");
            None
        }
    };
    let mut custom_alarm = load_custom(settings.alarm_file.as_deref());
    let mut ticking_sound = load_custom(settings.ticking_file.as_deref())
        .unwrap_or_else(|| builtin(SoundKind::Ticking));
    let mut settings = settings;
    let mut alarm: Option<Player> = None;
    let mut ticking: Option<Ticking> = None;

    let start_ticking = |sound: &Sound, volume: f32, fade: bool| {
        sink.as_ref()
            .and_then(|s| play(s, sound.clone(), volume, true))
            .map(|player| {
                let start_volume = if fade {
                    TICK_FADE_FROM * volume
                } else {
                    volume
                };
                player.set_volume(start_volume);
                Ticking {
                    player,
                    fade_started: fade.then(Instant::now),
                    volume: start_volume,
                }
            })
    };

    loop {
        let timeout = if ticking.as_ref().is_some_and(|t| t.fade_started.is_some()) {
            FADE_STEP
        } else {
            Duration::from_secs(3600)
        };

        match rx.recv_timeout(timeout) {
            Ok(Command::PlayAlarm(kind)) => {
                ticking = None;
                alarm = None;
                if settings.enabled {
                    let sound = alarm_for(&custom_alarm, kind);
                    alarm = sink
                        .as_ref()
                        .and_then(|s| play(s, sound, settings.volume, false));
                }
            }
            Ok(Command::StartTicking) => {
                ticking = None;
                if settings.enabled {
                    ticking = start_ticking(&ticking_sound, settings.volume, settings.fade_ticking);
                }
            }
            Ok(Command::StopTicking) => ticking = None,
            Ok(Command::Configure(new)) => {
                if new.alarm_file != settings.alarm_file {
                    custom_alarm = load_custom(new.alarm_file.as_deref());
                }
                if new.ticking_file != settings.ticking_file {
                    ticking_sound = load_custom(new.ticking_file.as_deref())
                        .unwrap_or_else(|| builtin(SoundKind::Ticking));
                }
                settings = new;
                if !settings.enabled {
                    ticking = None;
                    alarm = None;
                }
            }
            Ok(Command::Preview { kind, file, volume }) => {
                ticking = None;
                alarm = None;
                let sound = load_custom(file.as_deref()).unwrap_or_else(|| builtin(kind));
                match kind {
                    SoundKind::Alarm | SoundKind::BreakEnd => {
                        alarm = sink.as_ref().and_then(|s| play(s, sound, volume, false));
                    }
                    // Always faded, so the preview stops on its own.
                    SoundKind::Ticking => ticking = start_ticking(&sound, volume, true),
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        // Advance the ticking fade-out.
        if let Some(t) = &ticking
            && let Some(start) = t.fade_started
        {
            let progress = start.elapsed().as_secs_f32() / TICK_FADE.as_secs_f32();
            if progress >= 1.0 {
                ticking = None;
            } else {
                t.player.set_volume(t.volume * (1.0 - progress));
            }
        }
        if alarm.as_ref().is_some_and(|p| p.empty()) {
            alarm = None;
        }
    }
}

/// Start playing `sound`; it stops when the returned `Player` is dropped.
fn play(sink: &MixerDeviceSink, sound: Sound, volume: f32, looped: bool) -> Option<Player> {
    let decoder = match Decoder::new(Cursor::new(sound)) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("audio: failed to decode sound: {err}");
            return None;
        }
    };
    let player = Player::connect_new(sink.mixer());
    player.set_volume(volume);
    if looped {
        player.append(decoder.repeat_infinite());
    } else {
        player.append(decoder);
    }
    Some(player)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_sounds_decode() {
        for kind in [SoundKind::Alarm, SoundKind::BreakEnd, SoundKind::Ticking] {
            let decoder = Decoder::new(Cursor::new(builtin(kind))).expect("decodes");
            assert!(decoder.total_duration().is_some_and(|d| d > Duration::ZERO), "{kind:?}");
        }
    }

    #[test]
    fn custom_alarm_replaces_both_alarm_kinds() {
        let custom: Option<Sound> = Some(Arc::from(&b"custom"[..]));
        assert_eq!(&*alarm_for(&custom, SoundKind::BreakEnd), b"custom");
        assert_eq!(&*alarm_for(&None, SoundKind::BreakEnd), BREAK_END_WAV);
        assert_eq!(&*alarm_for(&None, SoundKind::Alarm), ALARM_WAV);
    }
}
