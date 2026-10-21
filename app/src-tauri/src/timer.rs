//! Pomodoro timer state machine.
//!
//! The engine is pure: every method takes `now` explicitly, so it never reads the
//! clock itself. Remaining time is derived from deadlines rather than counted
//! ticks, which keeps it accurate however irregularly `tick` is called.

use std::time::{Duration, Instant};

use serde::Serialize;

/// Pause between a pomodoro ending and the break starting (as in the original app).
pub const BREAK_START_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerConfig {
    pub pomodoro: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// Every n-th completed pomodoro is followed by a long break.
    pub long_break_interval: u32,
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            pomodoro: Duration::from_secs(25 * 60),
            short_break: Duration::from_secs(5 * 60),
            long_break: Duration::from_secs(15 * 60),
            long_break_interval: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Pomodoro,
    /// The pomodoro just ended; the break starts after `BREAK_START_DELAY`.
    TimeOut,
    Break,
    /// The break has ended and the next pomodoro has not been started yet.
    BreakOver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    Short,
    Long,
}

/// Transitions reported by the engine, used to trigger sounds and notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimerEvent {
    PomodoroStarted,
    PomodoroStopped,
    PomodoroCompleted { completed: u32 },
    BreakStarted { kind: BreakKind },
    BreakStopped,
    BreakEnded,
}

/// What the UI needs to render the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimerSnapshot {
    pub phase: Phase,
    pub break_kind: Option<BreakKind>,
    /// Whole seconds left in the current phase, rounded up (25:00 shows until a full second has passed).
    pub remaining_secs: u64,
    pub total_secs: u64,
    pub completed: u32,
}

#[derive(Debug)]
pub struct TimerEngine {
    config: TimerConfig,
    phase: Phase,
    break_kind: Option<BreakKind>,
    deadline: Option<Instant>,
    total: Duration,
    completed: u32,
}

impl TimerEngine {
    pub fn new(config: TimerConfig) -> Self {
        Self {
            config,
            phase: Phase::Idle,
            break_kind: None,
            deadline: None,
            total: config.pomodoro,
            completed: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// New lengths apply from the next phase; the running phase keeps its deadline.
    pub fn set_config(&mut self, config: TimerConfig) {
        self.config = config;
        if self.phase == Phase::Idle {
            self.total = config.pomodoro;
        }
    }

    /// Start a pomodoro. Allowed from any phase except a running pomodoro;
    /// starting during a break or time-out skips the rest of it.
    pub fn start(&mut self, now: Instant) -> Vec<TimerEvent> {
        if self.phase == Phase::Pomodoro {
            return Vec::new();
        }
        self.phase = Phase::Pomodoro;
        self.break_kind = None;
        self.total = self.config.pomodoro;
        self.deadline = Some(now + self.total);
        vec![TimerEvent::PomodoroStarted]
    }

    /// Stop whatever is running and return to idle. A stopped pomodoro is not counted.
    pub fn stop(&mut self) -> Vec<TimerEvent> {
        let event = match self.phase {
            Phase::Idle => return Vec::new(),
            Phase::Pomodoro => TimerEvent::PomodoroStopped,
            Phase::TimeOut | Phase::Break | Phase::BreakOver => TimerEvent::BreakStopped,
        };
        self.reset_to_idle();
        vec![event]
    }

    /// Advance through every deadline that has passed by `now`.
    /// Loops so that a long gap (e.g. after system sleep) settles in one call.
    pub fn tick(&mut self, now: Instant) -> Vec<TimerEvent> {
        let mut events = Vec::new();
        while let Some(deadline) = self.deadline {
            if now < deadline {
                break;
            }
            match self.phase {
                Phase::Pomodoro => {
                    self.completed += 1;
                    self.phase = Phase::TimeOut;
                    self.deadline = Some(deadline + BREAK_START_DELAY);
                    events.push(TimerEvent::PomodoroCompleted {
                        completed: self.completed,
                    });
                }
                Phase::TimeOut => {
                    let kind = self.next_break_kind();
                    self.phase = Phase::Break;
                    self.break_kind = Some(kind);
                    self.total = match kind {
                        BreakKind::Short => self.config.short_break,
                        BreakKind::Long => self.config.long_break,
                    };
                    self.deadline = Some(deadline + self.total);
                    events.push(TimerEvent::BreakStarted { kind });
                }
                Phase::Break => {
                    self.phase = Phase::BreakOver;
                    self.deadline = None;
                    events.push(TimerEvent::BreakEnded);
                }
                Phase::Idle | Phase::BreakOver => self.deadline = None,
            }
        }
        events
    }

    pub fn snapshot(&self, now: Instant) -> TimerSnapshot {
        let remaining = match (self.phase, self.deadline) {
            (Phase::Pomodoro | Phase::Break, Some(deadline)) => {
                deadline.saturating_duration_since(now)
            }
            (Phase::Idle, _) => self.config.pomodoro,
            _ => Duration::ZERO,
        };
        TimerSnapshot {
            phase: self.phase,
            break_kind: self.break_kind,
            remaining_secs: ceil_secs(remaining),
            total_secs: ceil_secs(self.total),
            completed: self.completed,
        }
    }

    fn next_break_kind(&self) -> BreakKind {
        let interval = self.config.long_break_interval;
        if interval > 0 && self.completed.is_multiple_of(interval) {
            BreakKind::Long
        } else {
            BreakKind::Short
        }
    }

    fn reset_to_idle(&mut self) {
        self.phase = Phase::Idle;
        self.break_kind = None;
        self.deadline = None;
        self.total = self.config.pomodoro;
    }
}

fn ceil_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn cfg() -> TimerConfig {
        TimerConfig {
            pomodoro: 25 * MIN,
            short_break: 5 * MIN,
            long_break: 15 * MIN,
            long_break_interval: 4,
        }
    }

    /// Run one full pomodoro + break cycle starting at `t`; returns the time the break ended.
    fn run_cycle(e: &mut TimerEngine, t: Instant) -> (Instant, BreakKind) {
        e.start(t);
        let t = t + e.config.pomodoro;
        e.tick(t);
        let t = t + BREAK_START_DELAY;
        let events = e.tick(t);
        let TimerEvent::BreakStarted { kind } = events[0] else {
            panic!("expected break start, got {events:?}");
        };
        let t = t + e.total;
        assert_eq!(e.tick(t), vec![TimerEvent::BreakEnded]);
        (t, kind)
    }

    #[test]
    fn idle_shows_full_pomodoro_length() {
        let e = TimerEngine::new(cfg());
        let s = e.snapshot(Instant::now());
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(s.remaining_secs, 25 * 60);
    }

    #[test]
    fn countdown_rounds_up_to_whole_seconds() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        assert_eq!(e.start(t0), vec![TimerEvent::PomodoroStarted]);
        assert_eq!(e.snapshot(t0).remaining_secs, 1500);
        assert_eq!(
            e.snapshot(t0 + Duration::from_millis(1)).remaining_secs,
            1500
        );
        assert_eq!(
            e.snapshot(t0 + Duration::from_millis(1000)).remaining_secs,
            1499
        );
    }

    #[test]
    fn nothing_happens_before_deadline() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        assert!(e.tick(t0 + 25 * MIN - Duration::from_millis(1)).is_empty());
        assert_eq!(e.phase(), Phase::Pomodoro);
    }

    #[test]
    fn pomodoro_completes_then_break_starts_after_delay() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        let t1 = t0 + 25 * MIN;
        assert_eq!(
            e.tick(t1),
            vec![TimerEvent::PomodoroCompleted { completed: 1 }]
        );
        assert_eq!(e.phase(), Phase::TimeOut);
        assert_eq!(e.snapshot(t1).remaining_secs, 0);

        assert!(
            e.tick(t1 + BREAK_START_DELAY - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(
            e.tick(t1 + BREAK_START_DELAY),
            vec![TimerEvent::BreakStarted {
                kind: BreakKind::Short
            }]
        );
        let s = e.snapshot(t1 + BREAK_START_DELAY);
        assert_eq!(
            (s.phase, s.remaining_secs, s.total_secs),
            (Phase::Break, 300, 300)
        );
    }

    #[test]
    fn every_fourth_break_is_long() {
        let mut t = Instant::now();
        let mut e = TimerEngine::new(cfg());
        let mut kinds = Vec::new();
        for _ in 0..8 {
            let (end, kind) = run_cycle(&mut e, t);
            kinds.push(kind);
            t = end;
        }
        use BreakKind::*;
        assert_eq!(
            kinds,
            vec![Short, Short, Short, Long, Short, Short, Short, Long]
        );
        assert_eq!(e.snapshot(t).completed, 8);
    }

    #[test]
    fn long_break_interval_zero_never_gives_long_break() {
        let mut e = TimerEngine::new(TimerConfig {
            long_break_interval: 0,
            ..cfg()
        });
        let mut t = Instant::now();
        for _ in 0..5 {
            let (end, kind) = run_cycle(&mut e, t);
            assert_eq!(kind, BreakKind::Short);
            t = end;
        }
    }

    #[test]
    fn break_over_waits_for_next_start() {
        let mut e = TimerEngine::new(cfg());
        let (t, _) = run_cycle(&mut e, Instant::now());
        assert_eq!(e.phase(), Phase::BreakOver);
        assert!(e.tick(t + 60 * MIN).is_empty());
        assert_eq!(e.start(t), vec![TimerEvent::PomodoroStarted]);
    }

    #[test]
    fn stopping_pomodoro_does_not_count_it() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        assert_eq!(e.stop(), vec![TimerEvent::PomodoroStopped]);
        let s = e.snapshot(t0 + 30 * MIN);
        assert_eq!(
            (s.phase, s.completed, s.remaining_secs),
            (Phase::Idle, 0, 1500)
        );
        assert!(e.tick(t0 + 30 * MIN).is_empty());
    }

    #[test]
    fn stopping_break_returns_to_idle() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        e.tick(t0 + 25 * MIN + BREAK_START_DELAY);
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.stop(), vec![TimerEvent::BreakStopped]);
        assert_eq!(e.phase(), Phase::Idle);
        assert!(e.stop().is_empty());
    }

    #[test]
    fn start_during_break_skips_it() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        let t1 = t0 + 25 * MIN + BREAK_START_DELAY;
        e.tick(t1);
        assert_eq!(e.start(t1), vec![TimerEvent::PomodoroStarted]);
        assert_eq!(e.snapshot(t1).remaining_secs, 1500);
    }

    #[test]
    fn start_while_running_is_ignored() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        assert!(e.start(t0 + MIN).is_empty());
        assert_eq!(e.snapshot(t0 + MIN).remaining_secs, 24 * 60);
    }

    #[test]
    fn long_gap_settles_all_transitions_at_once() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        assert_eq!(
            e.tick(t0 + 3 * 60 * MIN),
            vec![
                TimerEvent::PomodoroCompleted { completed: 1 },
                TimerEvent::BreakStarted {
                    kind: BreakKind::Short
                },
                TimerEvent::BreakEnded,
            ]
        );
        assert_eq!(e.phase(), Phase::BreakOver);
    }

    #[test]
    fn config_change_applies_to_next_phase_only() {
        let t0 = Instant::now();
        let mut e = TimerEngine::new(cfg());
        e.start(t0);
        e.set_config(TimerConfig {
            pomodoro: 50 * MIN,
            ..cfg()
        });
        assert_eq!(e.snapshot(t0).remaining_secs, 1500);
        e.stop();
        assert_eq!(e.snapshot(t0).remaining_secs, 3000);
    }
}
