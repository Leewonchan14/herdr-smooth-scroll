//! Pure animation math: how far to move, and how long to pause between lines.
//!
//! The delay schedule is a faithful port of `azorng/tmux-smooth-scroll`'s `animator.pl`:
//!
//! * base delay per line: `1000 + speed * 90` microseconds (speed 100 -> 10 ms),
//! * scrolls shorter than 10 lines are stretched up to 3x so a single tap still animates,
//! * the easing curve scales the per-step velocity (sine: 0.3x at the edges, 3x in the middle;
//!   quad: 0.2x to 3x; linear: constant).

use std::f64::consts::PI;

use crate::config::{Easing, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn parse(arg: &str) -> Option<Self> {
        match arg {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollKind {
    Normal,
    Halfpage,
    Fullpage,
}

impl ScrollKind {
    pub fn parse(arg: &str) -> Option<Self> {
        match arg {
            "normal" => Some(Self::Normal),
            "halfpage" => Some(Self::Halfpage),
            "fullpage" => Some(Self::Fullpage),
            _ => None,
        }
    }
}

/// Lines to move for this kind, honoring explicit config overrides.
pub fn resolve_lines(kind: ScrollKind, settings: &Settings, viewport_rows: usize) -> usize {
    match kind {
        ScrollKind::Normal => settings.normal_lines,
        ScrollKind::Halfpage => settings
            .halfpage_lines
            .unwrap_or((viewport_rows / 2).max(1)),
        ScrollKind::Fullpage => settings.fullpage_lines.unwrap_or(viewport_rows.max(1)),
    }
}

/// One line toward `direction`, or `None` when the pane is already at that edge.
pub fn next_offset(offset: u64, max: u64, direction: Direction) -> Option<u64> {
    match direction {
        Direction::Up if offset < max => Some(offset + 1),
        Direction::Down if offset > 0 => Some(offset - 1),
        _ => None,
    }
}

/// Base delay per line in microseconds: speed 0 -> 1 ms, speed 1000 -> 91 ms.
pub fn base_delay_us(speed: u32) -> u64 {
    1_000 + u64::from(speed.min(crate::config::SPEED_MAX)) * 90
}

/// Velocity factor for progress `t` (0.0-1.0); higher means shorter delay.
fn velocity(easing: Easing, t: f64) -> f64 {
    match easing {
        Easing::Linear => 1.0,
        Easing::Sine => 0.3 + (t * PI).sin() * 2.7,
        Easing::Quad => {
            let eased = if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            };
            0.2 + eased * 2.8
        }
    }
}

/// Delay after step `index` of `total` steps, in microseconds.
pub fn step_delay_us(base_us: u64, total: usize, index: usize, easing: Easing) -> u64 {
    let t = if total > 1 {
        index as f64 / (total - 1) as f64
    } else {
        0.0
    };
    // Short scrolls are stretched: 1 line runs at 3x the base delay, 10+ lines at 1x.
    let scale = if total > 0 && total < 10 {
        1.0 + 2.0 * (10 - total) as f64 / 9.0
    } else {
        1.0
    };
    (base_us as f64 * scale / velocity(easing, t)).ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_direction_and_kind_arguments() {
        assert_eq!(Direction::parse("up"), Some(Direction::Up));
        assert_eq!(Direction::parse("down"), Some(Direction::Down));
        assert_eq!(Direction::parse("sideways"), None);
        assert_eq!(ScrollKind::parse("normal"), Some(ScrollKind::Normal));
        assert_eq!(ScrollKind::parse("halfpage"), Some(ScrollKind::Halfpage));
        assert_eq!(ScrollKind::parse("fullpage"), Some(ScrollKind::Fullpage));
        assert_eq!(ScrollKind::parse("page"), None);
    }

    #[test]
    fn resolve_lines_uses_overrides_then_the_viewport() {
        let settings = Settings::default();
        assert_eq!(resolve_lines(ScrollKind::Normal, &settings, 40), 3);
        assert_eq!(resolve_lines(ScrollKind::Halfpage, &settings, 40), 20);
        assert_eq!(resolve_lines(ScrollKind::Fullpage, &settings, 40), 40);

        let overridden = Settings {
            halfpage_lines: Some(7),
            fullpage_lines: Some(11),
            ..Settings::default()
        };
        assert_eq!(resolve_lines(ScrollKind::Halfpage, &overridden, 40), 7);
        assert_eq!(resolve_lines(ScrollKind::Fullpage, &overridden, 40), 11);
    }

    #[test]
    fn resolve_lines_never_returns_zero_for_a_tiny_pane() {
        let settings = Settings::default();
        assert_eq!(resolve_lines(ScrollKind::Halfpage, &settings, 1), 1);
        assert_eq!(resolve_lines(ScrollKind::Fullpage, &settings, 0), 1);
    }

    #[test]
    fn next_offset_stops_at_both_edges() {
        assert_eq!(next_offset(3, 10, Direction::Up), Some(4));
        assert_eq!(next_offset(3, 10, Direction::Down), Some(2));
        assert_eq!(next_offset(10, 10, Direction::Up), None);
        assert_eq!(next_offset(0, 10, Direction::Down), None);
    }

    #[test]
    fn base_delay_scales_with_speed() {
        assert_eq!(base_delay_us(0), 1_000);
        assert_eq!(base_delay_us(100), 10_000);
        assert_eq!(base_delay_us(1000), 91_000);
        assert_eq!(base_delay_us(4000), 91_000);
    }

    #[test]
    fn sine_easing_is_slowest_at_the_edges_and_fastest_in_the_middle() {
        let base = 10_000;
        let total = 21;
        let first = step_delay_us(base, total, 0, Easing::Sine);
        let middle = step_delay_us(base, total, 10, Easing::Sine);
        let last = step_delay_us(base, total, 20, Easing::Sine);
        assert_eq!(first, last);
        assert!(
            middle < first / 4,
            "middle {middle} should be far below {first}"
        );
        // t = 0.5 -> velocity 3.0, so the middle step runs at a third of the base delay.
        assert_eq!(middle, 3_334);
    }

    #[test]
    fn quad_easing_matches_the_port() {
        let base = 10_000;
        let total = 11;
        assert_eq!(step_delay_us(base, total, 0, Easing::Quad), 50_000);
        // 10_000 / 1.6 lands on a float boundary; allow the ceil to round either way.
        let middle = step_delay_us(base, total, 5, Easing::Quad);
        assert!((6_250..=6_251).contains(&middle), "{middle}");
        assert_eq!(step_delay_us(base, total, 10, Easing::Quad), 3_334);
    }

    #[test]
    fn linear_easing_is_constant() {
        let base = 10_000;
        for index in 0..10 {
            assert_eq!(step_delay_us(base, 10, index, Easing::Linear), base);
        }
    }

    #[test]
    fn short_scrolls_are_stretched_up_to_three_times() {
        let base = 10_000;
        // 1 line: 3x stretch, sine edge velocity 0.3 -> 10x base.
        assert_eq!(step_delay_us(base, 1, 0, Easing::Sine), 100_000);
        // 3 lines: 2.56x stretch at the edges.
        assert_eq!(step_delay_us(base, 3, 0, Easing::Sine), 85_186);
        // 10 lines is the crossover; no stretch.
        assert_eq!(step_delay_us(base, 10, 0, Easing::Sine), 33_334);
    }
}
