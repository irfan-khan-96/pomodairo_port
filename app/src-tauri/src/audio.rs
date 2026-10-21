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

const ALARM_MP3: &[u8] = include_bytes!("../resources/sounds/alarm.mp3");
const TICKING_MP3: &[u8] = include_bytes!("../resources/sounds/ticking.mp3");

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
    /// Custom sound files; `None` uses the built-in sounds.
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
    Alarm,
    Ticking,
}

enum Command {
    PlayAlarm,
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

    pub fn play_alarm(&self) {
        let _ = self.tx.send(Command::PlayAlarm);
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

/// The configured sound, falling back to the built-in one if the file is unreadable.
fn load(file: Option<&Path>, builtin: &'static [u8]) -> Sound {
    if let Some(path) = file {
        match std::fs::read(path) {
            Ok(bytes) if Decoder::new(Cursor::new(bytes.clone())).is_ok() => return bytes.into(),
            Ok(_) => eprintln!(
                "audio: {} is not playable, using built-in sound",
                path.display()
            ),
            Err(e) => eprintln!(
                "audio: cannot read {}: {e}, using built-in sound",
                path.display()
            ),
        }
    }
    Arc::from(builtin)
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
    let mut alarm_sound = load(settings.alarm_file.as_deref(), ALARM_MP3);
    let mut ticking_sound = load(settings.ticking_file.as_deref(), TICKING_MP3);
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
            Ok(Command::PlayAlarm) => {
                ticking = None;
                alarm = None;
                if settings.enabled {
                    alarm = sink
                        .as_ref()
                        .and_then(|s| play(s, alarm_sound.clone(), settings.volume, false));
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
                    alarm_sound = load(new.alarm_file.as_deref(), ALARM_MP3);
                }
                if new.ticking_file != settings.ticking_file {
                    ticking_sound = load(new.ticking_file.as_deref(), TICKING_MP3);
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
                match kind {
                    SoundKind::Alarm => {
                        let sound = load(file.as_deref(), ALARM_MP3);
                        alarm = sink.as_ref().and_then(|s| play(s, sound, volume, false));
                    }
                    // Always faded, so the preview stops on its own.
                    SoundKind::Ticking => {
                        ticking = start_ticking(&load(file.as_deref(), TICKING_MP3), volume, true);
                    }
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
