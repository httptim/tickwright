//! `DaylightDetectorBlock` and its block entity, whose ticker recomputes the
//! output once a second.
//!
//! 26.x moved the day cycle into the world clock's timelines: the sky light
//! level and the sun's angle are tracks of `data/<version>/timelines.json`,
//! sampled at the level's day time. The sky is assumed open (sky light 15)
//! and the day time frozen, as the fixtures' harness keeps it.

use std::f32::consts::PI;
use std::sync::OnceLock;

use serde_json::Value;

use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::registry::StateId;

const TIMELINES_26_3: &str = include_str!("../../../../data/26.3/timelines.json");

/// A looping track: (ticks, value) keyframes over a period.
struct Track {
    period: i64,
    keys: Vec<(i64, f32)>,
    /// The control points of a cubic bezier ease, or linear.
    ease: Option<[f64; 4]>,
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
            let keys = t["keyframes"]
                .as_array()
                .expect("keyframes")
                .iter()
                .map(|k| (k["ticks"].as_i64().unwrap(), k["value"].as_f64().unwrap() as f32))
                .collect();
            let ease = t["ease"]["cubic_bezier"].as_array().map(|c| {
                let c: Vec<f64> = c.iter().map(|x| x.as_f64().unwrap()).collect();
                [c[0], c[1], c[2], c[3]]
            });
            Track { period, keys, ease }
        };
        Day {
            sky_light_level: track("minecraft:gameplay/sky_light_level"),
            sun_angle: track("minecraft:visual/sun_angle"),
        }
    })
}

/// The y of a cubic bezier ease from (0,0) to (1,1) at `x`.
fn bezier(c: [f64; 4], x: f64) -> f64 {
    let at = |s: f64, a: f64, b: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..60 {
        let s = (lo + hi) / 2.0;
        if at(s, c[0], c[2]) < x { lo = s } else { hi = s }
    }
    at((lo + hi) / 2.0, c[1], c[3])
}

impl Track {
    fn sample(&self, time: i64) -> f32 {
        let t = time.rem_euclid(self.period);
        let (first, last) = (self.keys[0], self.keys[self.keys.len() - 1]);
        // The last keyframe at or before `t` and the first after it,
        // wrapping around the period.
        let prev = self.keys.iter().rev().find(|k| k.0 <= t).copied().unwrap_or((last.0 - self.period, last.1));
        let next = self.keys.iter().find(|k| k.0 > t).copied().unwrap_or((first.0 + self.period, first.1));
        if next.0 == prev.0 {
            return next.1;
        }
        let x = (t - prev.0) as f64 / (next.0 - prev.0) as f64;
        let x = match self.ease {
            Some(c) => bezier(c, x),
            None => x,
        };
        prev.1 + (next.1 - prev.1) * x as f32
    }
}

/// `Level.getSkyDarken` from the sky light level track.
pub fn sky_darken(day_time: i64) -> i32 {
    15 - (day().sky_light_level.sample(day_time) * 15.0).ceil() as i32
}

/// `Level.getSunAngle` in radians.
pub fn sun_angle(day_time: i64) -> f32 {
    day().sun_angle.sample(day_time) * (PI / 180.0)
}

/// `DaylightDetectorBlock.updateSignalStrength`.
pub fn update_signal_strength(level: &mut Level, pos: Pos, state: StateId) {
    let mut i = 15 - sky_darken(level.day_time);
    let mut f = sun_angle(level.day_time);
    if level.flag(state, "inverted") {
        i = 15 - i;
    } else if i > 0 {
        let g = if f < PI { 0.0 } else { PI * 2.0 };
        f += (g - f) * 0.2;
        i = (i as f32 * f.cos()).round() as i32;
    }
    let i = i.clamp(0, 15);
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
    fn darken_spans_day_and_night() {
        assert_eq!(sky_darken(6000), 0);
        assert_eq!(sky_darken(18000), 11);
        assert!(sun_angle(6000).abs() < 1e-6);
    }
}
