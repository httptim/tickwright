//! `DaylightDetectorBlock` and its block entity, whose ticker recomputes the
//! output once a second.
//!
//! 26.x moved the day cycle into the world clock's timelines: the sky light
//! level and the sun's angle are tracks of `data/<version>/timelines.json`,
//! sampled at the level's day time. The sky is assumed open (sky light 15)
//! and the day time frozen, as the fixtures' harness keeps it.
//!
//! The sampling, the sun angle's cubic bezier ease and the detector's
//! cosine (`Mth.cos`, from its lookup table) follow the game's float
//! arithmetic. Rounding the output makes a last-bit difference in any of
//! them change it at some day times (`daylight_detector_transitions`).

use std::f32::consts::PI;
use std::sync::OnceLock;

use serde_json::Value;

use crate::level::{Level, flags};
use crate::physics::mth;
use crate::pos::Pos;
use crate::registry::StateId;

const TIMELINES_26_3: &str = include_str!("../../../../data/26.3/timelines.json");

/// A looping track, cut into segments between keyframes as
/// `KeyframeTrackSampler` bakes it, and sampled in floats as it does.
struct Track {
    period: i64,
    segments: Vec<Segment>,
    /// `None` for linear.
    ease: Option<CubicBezier>,
}

/// From `from` at `from_ticks` to `to` at `to_ticks`.
struct Segment {
    from_ticks: i64,
    from: f32,
    to_ticks: i64,
    to: f32,
}

struct Day {
    sky_light_level: Track,
    sun_angle: Track,
}

fn day() -> &'static Day {
    static DAY: OnceLock<Day> = OnceLock::new();
    DAY.get_or_init(|| {
        let v: Value = serde_json::from_str(TIMELINES_26_3).expect("data/26.3/timelines.json is valid");
        let day = &v["timeline/day"];
        let period = day["period_ticks"].as_i64().expect("period_ticks");
        let track = |name: &str| {
            let t = &day["tracks"][name];
            // The game reads these numbers straight to floats; for the
            // day's values that is the same as through a double.
            let keys: Vec<(i64, f32)> = t["keyframes"]
                .as_array()
                .expect("keyframes")
                .iter()
                .map(|k| (k["ticks"].as_i64().unwrap(), k["value"].as_f64().unwrap() as f32))
                .collect();
            let ease = t.get("ease").map(|e| {
                let c = e["cubic_bezier"].as_array().unwrap_or_else(|| panic!("{name}: easing {e} is not supported"));
                let c: Vec<f32> = c.iter().map(|x| x.as_f64().unwrap() as f32).collect();
                CubicBezier::new(c[0], c[1], c[2], c[3])
            });
            Track::new(period, &keys, ease)
        };
        Day {
            sky_light_level: track("minecraft:gameplay/sky_light_level"),
            sun_angle: track("minecraft:visual/sun_angle"),
        }
    })
}

/// `EasingType.CubicBezier`: a curve from (0,0) to (1,1), whose parameter
/// for a given x is found in floats to within 1e-5, by a few Newton-Raphson
/// steps and then by halving.
struct CubicBezier {
    x: Cubic,
    y: Cubic,
}

/// `CubicBezier.CubicCurve`: `((a t + b) t + c) t`.
#[derive(Clone, Copy)]
struct Cubic {
    a: f32,
    b: f32,
    c: f32,
}

impl Cubic {
    fn from_controls(v1: f32, v2: f32) -> Cubic {
        Cubic { a: 3.0 * v1 - 3.0 * v2 + 1.0, b: -6.0 * v1 + 3.0 * v2, c: 3.0 * v1 }
    }

    fn sample(self, t: f32) -> f32 {
        ((self.a * t + self.b) * t + self.c) * t
    }

    fn gradient(self, t: f32) -> f32 {
        (3.0 * self.a * t + 2.0 * self.b) * t + self.c
    }
}

impl CubicBezier {
    fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> CubicBezier {
        CubicBezier { x: Cubic::from_controls(x1, x2), y: Cubic::from_controls(y1, y2) }
    }

    fn apply(&self, x: f32) -> f32 {
        self.y.sample(self.solve_t(x))
    }

    fn solve_t(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..4 {
            let error = self.x.sample(t) - x;
            if error.abs() < 1.0e-5 {
                return t;
            }
            let gradient = self.x.gradient(t);
            if gradient < 1.0e-5 {
                break;
            }
            t -= (error / gradient).clamp(-0.25, 0.25);
        }
        self.solve_t_bisect(x, t)
    }

    fn solve_t_bisect(&self, x: f32, initial: f32) -> f32 {
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut t = initial;
        while t0 < t1 {
            let error = self.x.sample(t) - x;
            if error.abs() < 1.0e-5 {
                return t;
            }
            if error < 0.0 {
                t0 = t;
            } else {
                t1 = t;
            }
            t = (t1 + t0) / 2.0;
        }
        t
    }
}

impl Track {
    /// `KeyframeTrackSampler.bakeSegments` for a looping track.
    fn new(period: i64, keys: &[(i64, f32)], ease: Option<CubicBezier>) -> Track {
        let (first, last) = (keys[0], keys[keys.len() - 1]);
        let segment =
            |(from_ticks, from): (i64, f32), (to_ticks, to): (i64, f32)| Segment { from_ticks, from, to_ticks, to };
        let mut segments = vec![segment((last.0 - period, last.1), first)];
        segments.extend(keys.windows(2).map(|w| segment(w[0], w[1])));
        segments.push(segment(last, (first.0 + period, first.1)));
        Track { period, segments, ease }
    }

    /// `KeyframeTrackSampler.sample`.
    fn sample(&self, time: i64) -> f32 {
        let t = time.rem_euclid(self.period);
        let s = self.segments.iter().find(|s| t < s.to_ticks).unwrap_or(&self.segments[self.segments.len() - 1]);
        if t <= s.from_ticks {
            return s.from;
        }
        if t >= s.to_ticks {
            return s.to;
        }
        let alpha = (t - s.from_ticks) as f32 / (s.to_ticks - s.from_ticks) as f32;
        let eased = match &self.ease {
            Some(b) => b.apply(alpha),
            None => alpha,
        };
        // `Mth.lerp`.
        s.from + eased * (s.to - s.from)
    }
}

/// `Level.updateSkyBrightness`. The sky light level attribute is the track
/// times the default 15 (its `multiply` modifier), kept within 0 to 15.
pub fn sky_darken(day_time: i64) -> i32 {
    let level = (15.0f32 * day().sky_light_level.sample(day_time)).clamp(0.0, 15.0);
    (15.0 - level) as i32
}

/// `Level.getSunAngle` in radians.
pub fn sun_angle(day_time: i64) -> f32 {
    day().sun_angle.sample(day_time) * (std::f64::consts::PI / 180.0) as f32
}

/// The output `DaylightDetectorBlock.updateSignalStrength` works out under
/// open sky at `day_time`.
pub fn power(day_time: i64, inverted: bool) -> i32 {
    let mut i = 15 - sky_darken(day_time);
    let mut f = sun_angle(day_time);
    if inverted {
        i = 15 - i;
    } else if i > 0 {
        let g = if f < PI { 0.0 } else { PI * 2.0 };
        f += (g - f) * 0.2;
        i = java_round(i as f32 * mth::cos(f as f64));
    }
    i.clamp(0, 15)
}

/// `Math.round(float)`: the nearest integer, halves rounding up.
fn java_round(x: f32) -> i32 {
    (x as f64 + 0.5).floor() as i32
}

/// `DaylightDetectorBlock.updateSignalStrength`.
pub fn update_signal_strength(level: &mut Level, pos: Pos, state: StateId) {
    let i = power(level.day_time, level.flag(state, "inverted"));
    if level.int(state, "power") != i {
        let state = level.with_int(state, "power", i);
        level.set_block(pos, state, flags::ALL);
    }
}

/// `DaylightDetectorBlockEntity.tick`, once every 20 game ticks.
pub fn tick_entity(level: &mut Level, pos: Pos) {
    if level.game_time % 20 == 0 {
        let state = level.get(pos);
        update_signal_strength(level, pos, state);
    }
}

/// `useWithoutItem`: flip the mode and recompute at once.
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    let state = level.with_bool(state, "inverted", !level.flag(state, "inverted"));
    level.set_block(pos, state, flags::CLIENTS);
    update_signal_strength(level, pos, state);
}

pub fn get_signal(level: &Level, state: StateId) -> i32 {
    level.int(state, "power")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_rounds_as_the_game_does() {
        // daylight_detector_transitions' capture at the day times where
        // f32::cos, or easing the sun angle in doubles, gives another value.
        for (t, p) in [(170, 8), (539, 9), (2447, 12), (10625, 11), (11461, 9), (12927, 3), (23073, 3), (23533, 5)] {
            assert_eq!(power(t, false), p, "day time {t}");
        }
    }

    #[test]
    fn darken_spans_day_and_night() {
        assert_eq!(sky_darken(6000), 0);
        assert_eq!(sky_darken(18000), 11);
        assert!(sun_angle(6000).abs() < 1e-6);
    }
}
