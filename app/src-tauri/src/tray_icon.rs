//! Draws the tray icon: a progress ring (the time left, shrinking clockwise from
//! twelve o'clock) with a small tomato calyx on top. Rendered at runtime because
//! it changes as the timer runs; plain RGBA, no image dependencies.

use std::f32::consts::TAU;

use crate::timer::{Phase, TimerSnapshot};

/// Rendered size in pixels; the OS scales it to the tray (16-24px at 100-150%).
pub const SIZE: u32 = 32;
/// The ring changes in this many steps per phase, so the icon is not redrawn every second.
const STEPS: u32 = 60;
/// Samples per pixel along each axis, for anti-aliasing.
const SUPERSAMPLE: u32 = 4;

const YELLOW: [u8; 3] = [0xf2, 0xc4, 0x0f]; // work (a deeper gold so it shows on light taskbars too)
const GREEN: [u8; 3] = [0x3f, 0xbf, 0x5f]; // break
const RED: [u8; 3] = [0xe8, 0x48, 0x3a]; // time's up
const LEAF: [u8; 3] = [0x4c, 0xaf, 0x50];
/// Unfilled part of the ring: mid gray, half transparent, visible on light and dark taskbars.
const TRACK: [u8; 4] = [0x8c, 0x8c, 0x8c, 0x70];

/// What the icon shows; equal keys produce identical icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconKey {
    color: [u8; 3],
    /// Filled steps out of `STEPS`.
    filled: u32,
}

impl IconKey {
    pub fn for_snapshot(s: &TimerSnapshot) -> Self {
        let fraction = |s: &TimerSnapshot| {
            if s.total_secs == 0 {
                0
            } else {
                // Round up so the ring only empties when the time is actually up.
                ((s.remaining_secs * STEPS as u64).div_ceil(s.total_secs) as u32).min(STEPS)
            }
        };
        match s.phase {
            Phase::Idle => Self {
                color: YELLOW,
                filled: STEPS,
            },
            Phase::Pomodoro => Self {
                color: YELLOW,
                filled: fraction(s),
            },
            Phase::Break => Self {
                color: GREEN,
                filled: fraction(s),
            },
            Phase::TimeOut | Phase::BreakOver => Self {
                color: RED,
                filled: STEPS,
            },
        }
    }

    /// The icon as straight (non-premultiplied) RGBA, `SIZE` × `SIZE`.
    pub fn render(&self) -> Vec<u8> {
        render(SIZE, self.filled as f32 / STEPS as f32, self.color)
    }
}

fn render(size: u32, remaining: f32, color: [u8; 3]) -> Vec<u8> {
    let n = size as f32;
    let (cx, cy) = (n / 2.0, n * 0.55);
    let (outer, inner) = (n * 0.43, n * 0.27);
    let filled_angle = remaining.clamp(0.0, 1.0) * TAU;
    // Two flat sepals either side of the top of the ring.
    let leaves = [(cx - n * 0.11, n * 0.14), (cx + n * 0.11, n * 0.14)];
    let (leaf_a, leaf_b) = (n * 0.13, n * 0.06);

    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for py in 0..size {
        for px in 0..size {
            // Premultiplied accumulation over the sub-samples.
            let mut acc = [0f32; 4];
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = px as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32;
                    let in_leaf = leaves.iter().any(|&(lx, ly)| {
                        let (dx, dy) = ((x - lx) / leaf_a, (y - ly) / leaf_b);
                        dx * dx + dy * dy <= 1.0
                    });
                    let (dx, dy) = (x - cx, y - cy);
                    let r = (dx * dx + dy * dy).sqrt();
                    let rgba = if in_leaf {
                        Some([LEAF[0], LEAF[1], LEAF[2], 255])
                    } else if (inner..=outer).contains(&r) {
                        // Clockwise angle from twelve o'clock.
                        let angle = dx.atan2(-dy).rem_euclid(TAU);
                        Some(if angle <= filled_angle {
                            [color[0], color[1], color[2], 255]
                        } else {
                            TRACK
                        })
                    } else {
                        None
                    };
                    if let Some([r, g, b, a]) = rgba {
                        let a = a as f32 / 255.0;
                        acc[0] += r as f32 * a;
                        acc[1] += g as f32 * a;
                        acc[2] += b as f32 * a;
                        acc[3] += a;
                    }
                }
            }
            let alpha = acc[3] / samples;
            if acc[3] > 0.0 {
                out.extend(
                    [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]].map(|c| c.round() as u8),
                );
            } else {
                out.extend([0, 0, 0]);
            }
            out.push((alpha * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::BreakKind;

    fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    fn snap(phase: Phase, remaining_secs: u64, total_secs: u64) -> TimerSnapshot {
        TimerSnapshot {
            phase,
            break_kind: (phase == Phase::Break).then_some(BreakKind::Short),
            remaining_secs,
            total_secs,
            completed: 0,
        }
    }

    // On the ring at three o'clock (inside the band) and at nine o'clock.
    const EAST: (u32, u32) = (27, 17);
    const WEST: (u32, u32) = (4, 17);

    #[test]
    fn full_ring_and_partial_ring() {
        let full = render(SIZE, 1.0, YELLOW);
        assert_eq!(full.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(&pixel(&full, EAST.0, EAST.1)[..3], &YELLOW);
        assert_eq!(&pixel(&full, WEST.0, WEST.1)[..3], &YELLOW);

        // 40% left: three o'clock (25%) is still lit, nine o'clock (75%) is track.
        let partial = render(SIZE, 0.4, YELLOW);
        assert_eq!(&pixel(&partial, EAST.0, EAST.1)[..3], &YELLOW);
        assert_eq!(pixel(&partial, WEST.0, WEST.1), TRACK);
    }

    #[test]
    fn centre_and_corners_are_transparent_and_leaf_is_green() {
        let img = render(SIZE, 1.0, RED);
        assert_eq!(pixel(&img, 16, 17)[3], 0, "hole in the ring");
        assert_eq!(pixel(&img, 0, 31)[3], 0, "corner");
        assert_eq!(pixel(&img, 12, 4), [LEAF[0], LEAF[1], LEAF[2], 255]);
    }

    #[test]
    fn keys_quantise_and_follow_phase() {
        let a = IconKey::for_snapshot(&snap(Phase::Pomodoro, 1500, 1500));
        assert_eq!(
            a,
            IconKey {
                color: YELLOW,
                filled: 60
            }
        );
        // 1499 s and 1476 s left round up to the same step, so no redraw in between.
        assert_eq!(
            IconKey::for_snapshot(&snap(Phase::Pomodoro, 1499, 1500)),
            IconKey::for_snapshot(&snap(Phase::Pomodoro, 1476, 1500))
        );
        assert_eq!(
            IconKey::for_snapshot(&snap(Phase::Pomodoro, 1, 1500)).filled,
            1
        );
        assert_eq!(
            IconKey::for_snapshot(&snap(Phase::Break, 150, 300)).color,
            GREEN
        );
        assert_eq!(
            IconKey::for_snapshot(&snap(Phase::TimeOut, 0, 1500)),
            IconKey {
                color: RED,
                filled: 60
            }
        );
        assert_eq!(
            IconKey::for_snapshot(&snap(Phase::Pomodoro, 5, 0)).filled,
            0,
            "no division by zero"
        );
    }
}
