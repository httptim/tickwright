//! `SculkSensorBlock`, `CalibratedSculkSensorBlock` and the vibration
//! system their block entities run (`VibrationSystem`).

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::{BlockEntity, Kind};

/// A vibration on its way to a sensor (`VibrationInfo`).
#[derive(Clone, Copy, Debug)]
pub struct Vibration {
    pub event: GameEvent,
    pub distance: f32,
    pub pos: Pos,
}

/// `SculkSensorBlockEntity` with its `VibrationSystem.Data`.
#[derive(Clone, Debug, Default)]
pub struct Sensor {
    pub last_frequency: i32,
    /// `VibrationSelector`: the best candidate heard and the tick it came.
    candidate: Option<(Vibration, i64)>,
    current: Option<Vibration>,
    travel_time: i32,
}

fn is_calibrated(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::CalibratedSculkSensor
}

fn listener_radius(level: &Level, state: StateId) -> i32 {
    if is_calibrated(level, state) { 16 } else { 8 }
}

fn active_ticks(level: &Level, state: StateId) -> i32 {
    if is_calibrated(level, state) { 10 } else { 30 }
}

/// `SculkSensorBlock.canActivate`.
fn can_activate(level: &Level, state: StateId) -> bool {
    level.prop(state, "sculk_sensor_phase") == "inactive"
}

fn sensor(level: &mut Level, pos: Pos) -> Option<&mut Sensor> {
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Sculk(s)) => Some(s),
        _ => None,
    }
}

fn center(p: Pos) -> [f64; 3] {
    [p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5]
}

fn distance(a: Pos, b: Pos) -> f64 {
    let (a, b) = (center(a), center(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// `CalibratedSculkSensorBlock.getBackSignal`: the input from behind.
fn calibrated_input(level: &Level, pos: Pos, state: StateId) -> i32 {
    let d = level.dir(state, "facing").opposite();
    level.get_signal(pos.offset(d), d)
}

/// `Level.gameEvent`: every sensor in range hears it.
pub fn post(level: &mut Level, event: GameEvent, source: Pos) {
    let Some(frequency) = event.frequency() else { return };
    let mut sensors: Vec<(f64, Pos)> = level
        .block_entities
        .iter()
        .filter(|(_, be)| matches!(be, BlockEntity::Sculk(_)))
        .map(|(&p, _)| (distance(source, p), p))
        .collect();
    sensors.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| (a.1.x, a.1.y, a.1.z).cmp(&(b.1.x, b.1.y, b.1.z))));
    let now = level.game_time;
    for (d, p) in sensors {
        let state = level.get(p);
        let r = listener_radius(level, state) as f64;
        if d * d > r * r {
            continue;
        }
        // `VibrationSystem.Listener.handleGameEvent`.
        if sensor(level, p).is_some_and(|s| s.current.is_some()) {
            continue;
        }
        // `SculkSensorBlockEntity.VibrationUser.canReceiveVibration`.
        if p == source && matches!(event, GameEvent::BlockDestroy | GameEvent::BlockPlace) {
            continue;
        }
        if !can_activate(level, state) {
            continue;
        }
        if is_calibrated(level, state) {
            let input = calibrated_input(level, p, state);
            if input != 0 && input != frequency {
                continue;
            }
        }
        if is_occluded(level, source, p) {
            continue;
        }
        let v = Vibration { event, distance: d as f32, pos: source };
        let s = sensor(level, p).unwrap();
        // `VibrationSelector.addCandidate`.
        let replace = match s.candidate {
            None => true,
            Some((c, tick)) => {
                tick == now
                    && (v.distance < c.distance
                        || (v.distance == c.distance && frequency > c.event.frequency().unwrap_or(0)))
            }
        };
        if replace {
            s.candidate = Some((v, now));
        }
    }
}

/// `VibrationSystem.Listener.isOccluded`: wool on every line from just off
/// the source's center to the listener's.
fn is_occluded(level: &Level, from: Pos, to: Pos) -> bool {
    let a = center(from);
    let b = center(to);
    Dir::ALL.iter().all(|d| {
        let [dx, dy, dz] = d.step();
        let e = 1.0e-5f32 as f64;
        let start = [a[0] + dx as f64 * e, a[1] + dy as f64 * e, a[2] + dz as f64 * e];
        hits_occluder(level, start, b)
    })
}

/// `BlockGetter.isBlockInLine`: walk the blocks the segment passes through.
fn hits_occluder(level: &Level, from: [f64; 3], to: [f64; 3]) -> bool {
    // `BlockGetter.traverseBlocks`, as in vanilla: nudged ends, a voxel walk.
    let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
    let e = -1.0e-7;
    let end = [lerp(e, to[0], from[0]), lerp(e, to[1], from[1]), lerp(e, to[2], from[2])];
    let start = [lerp(e, from[0], to[0]), lerp(e, from[1], to[1]), lerp(e, from[2], to[2])];
    let mut x = start[0].floor() as i32;
    let mut y = start[1].floor() as i32;
    let mut z = start[2].floor() as i32;
    // `#occludes_vibration_signals`: the wools.
    let occludes = |x: i32, y: i32, z: i32| level.reg.name(level.get(Pos::new(x, y, z))).ends_with("_wool");
    if occludes(x, y, z) {
        return true;
    }
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    let sx = dx.signum() as i32;
    let sy = dy.signum() as i32;
    let sz = dz.signum() as i32;
    let tdx = if sx == 0 { f64::MAX } else { sx as f64 / dx };
    let tdy = if sy == 0 { f64::MAX } else { sy as f64 / dy };
    let tdz = if sz == 0 { f64::MAX } else { sz as f64 / dz };
    let frac = |v: f64| v - v.floor();
    let mut tx = tdx * if sx > 0 { 1.0 - frac(start[0]) } else { frac(start[0]) };
    let mut ty = tdy * if sy > 0 { 1.0 - frac(start[1]) } else { frac(start[1]) };
    let mut tz = tdz * if sz > 0 { 1.0 - frac(start[2]) } else { frac(start[2]) };
    while tx <= 1.0 || ty <= 1.0 || tz <= 1.0 {
        if tx < ty {
            if tx < tz {
                x += sx;
                tx += tdx;
            } else {
                z += sz;
                tz += tdz;
            }
        } else if ty < tz {
            y += sy;
            ty += tdy;
        } else {
            z += sz;
            tz += tdz;
        }
        if occludes(x, y, z) {
            return true;
        }
    }
    false
}

/// `VibrationSystem.Ticker.tick`, the block entity's ticker. Each change
/// to the vibration data marks the block entity changed, which comparators
/// hear.
pub fn tick_entity(level: &mut Level, pos: Pos) {
    let now = level.game_time;
    let Some(s) = sensor(level, pos) else { return };
    let mut changes = 0;
    if s.current.is_none()
        && let Some((v, tick)) = s.candidate
        && tick < now
    {
        s.current = Some(v);
        s.travel_time = v.distance.floor() as i32;
        s.candidate = None;
        changes += 1;
    }
    if let Some(v) = s.current {
        let mut changed = s.travel_time > 0;
        s.travel_time = (s.travel_time - 1).max(0);
        if s.travel_time <= 0 {
            s.current = None;
            receive(level, pos, v);
            changed = true;
        }
        if changed {
            changes += 1;
        }
    }
    for _ in 0..changes {
        super::container::set_changed(level, pos);
    }
}

/// `VibrationSystem.receiveVibration`, with the distance between blocks.
fn receive(level: &mut Level, pos: Pos, v: Vibration) {
    let d = {
        let (dx, dy, dz) = ((v.pos.x - pos.x) as f64, (v.pos.y - pos.y) as f64, (v.pos.z - pos.z) as f64);
        (dx * dx + dy * dy + dz * dz).sqrt() as f32
    };
    let state = level.get(pos);
    if can_activate(level, state) {
        let frequency = v.event.frequency().unwrap_or(0);
        if let Some(s) = sensor(level, pos) {
            s.last_frequency = frequency;
        }
        let power = (15 - ((15.0 / listener_radius(level, state) as f64) * d as f64).floor() as i32).max(1);
        activate(level, pos, state, power);
    }
}

/// `SculkSensorBlock.activate`.
fn activate(level: &mut Level, pos: Pos, state: StateId, power: i32) {
    let active = level.with_int(level.with(state, "sculk_sensor_phase", "active"), "power", power);
    level.set_block(pos, active, flags::ALL);
    level.schedule_tick(pos, level.block(state), active_ticks(level, state), 0);
    update_neighbours(level, pos, state);
    post(level, GameEvent::SculkSensorTendrilsClicking, pos);
    click_pitch(level, state);
}

/// The clicking sound's pitch, `level.getRandom().nextFloat() * 0.2F +
/// 0.8F`, drawn from the level's random source unless the sensor is
/// waterlogged and stays quiet.
fn click_pitch(level: &mut Level, state: StateId) {
    if !level.flag(state, "waterlogged") {
        level.random.next_float();
    }
}

/// `SculkSensorBlock.updateNeighbours`.
fn update_neighbours(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    level.update_neighbors_at(pos, block);
    level.update_neighbors_at(pos.below(), block);
}

/// `SculkSensorBlock.tick`.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    match level.prop(state, "sculk_sensor_phase") {
        "active" => {
            let cooldown = level.with_int(level.with(state, "sculk_sensor_phase", "cooldown"), "power", 0);
            level.set_block(pos, cooldown, flags::ALL);
            level.schedule_tick(pos, level.block(state), 10, 0);
            update_neighbours(level, pos, state);
        }
        "cooldown" => {
            let inactive = level.with(state, "sculk_sensor_phase", "inactive");
            level.set_block(pos, inactive, flags::ALL);
            click_pitch(level, state);
        }
        _ => {}
    }
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId) {
    if level.prop(state, "sculk_sensor_phase") == "active" {
        update_neighbours(level, pos, state);
    }
}

pub fn get_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if is_calibrated(level, state) && dir == level.dir(state, "facing") {
        return 0;
    }
    level.int(state, "power")
}

pub fn get_direct_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if dir == Dir::Up { get_signal(level, state, dir) } else { 0 }
}

/// `getAnalogOutputSignal`: the last frequency heard, while active.
pub fn analog_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Sculk(s)) if level.prop(state, "sculk_sensor_phase") == "active" => s.last_frequency,
        _ => 0,
    }
}
