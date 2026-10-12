//! What entity physics needs from blocks, from `data/<version>/physics.json`:
//! each state's collision boxes (as the game gives them with no world around
//! the block) and each block's friction, speed and jump factors and
//! explosion resistance.

use std::sync::LazyLock;

use anyhow::{Context, Result, bail};

use crate::registry::{BlockId, Registry, StateId};

/// A point or a velocity: the game's `Vec3`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;

    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Vec3;

    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn scale(self, f: f64) -> Vec3 {
        Vec3::new(self.x * f, self.y * f, self.z * f)
    }

    pub fn multiply(self, x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(self.x * x, self.y * y, self.z * z)
    }

    pub fn length_sqr(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn length(self) -> f64 {
        self.length_sqr().sqrt()
    }

    pub fn horizontal_distance_sqr(self) -> f64 {
        self.x * self.x + self.z * self.z
    }

    pub fn distance_to_sqr(self, o: Vec3) -> f64 {
        (self - o).length_sqr()
    }

    pub fn get(self, axis: u8) -> f64 {
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    pub fn with(self, axis: u8, v: f64) -> Vec3 {
        match axis {
            0 => Vec3::new(v, self.y, self.z),
            1 => Vec3::new(self.x, v, self.z),
            _ => Vec3::new(self.x, self.y, v),
        }
    }
}

/// An axis-aligned box: the game's `AABB`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    pub fn new(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
        Aabb { min: [x0.min(x1), y0.min(y1), z0.min(z1)], max: [x0.max(x1), y0.max(y1), z0.max(z1)] }
    }

    pub fn offset(self, x: f64, y: f64, z: f64) -> Aabb {
        Aabb {
            min: [self.min[0] + x, self.min[1] + y, self.min[2] + z],
            max: [self.max[0] + x, self.max[1] + y, self.max[2] + z],
        }
    }

    pub fn moved(self, v: Vec3) -> Aabb {
        self.offset(v.x, v.y, v.z)
    }

    /// `AABB.expandTowards`.
    pub fn expand_towards(self, v: Vec3) -> Aabb {
        let mut b = self;
        for (axis, d) in [v.x, v.y, v.z].into_iter().enumerate() {
            if d < 0.0 {
                b.min[axis] += d;
            } else if d > 0.0 {
                b.max[axis] += d;
            }
        }
        b
    }

    /// `AABB.inflate`.
    pub fn inflate(self, x: f64, y: f64, z: f64) -> Aabb {
        Aabb::new(self.min[0] - x, self.min[1] - y, self.min[2] - z, self.max[0] + x, self.max[1] + y, self.max[2] + z)
    }

    pub fn deflate(self, d: f64) -> Aabb {
        self.inflate(-d, -d, -d)
    }

    /// `AABB.intersects`: strict on every side.
    pub fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|a| self.min[a] < o.max[a] && self.max[a] > o.min[a])
    }

    pub fn center(&self) -> Vec3 {
        Vec3::new(
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
            (self.min[2] + self.max[2]) / 2.0,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BlockFactors {
    pub friction: f32,
    pub speed: f32,
    pub jump: f32,
    pub resistance: f32,
}

const DEFAULT_FACTORS: BlockFactors = BlockFactors { friction: 0.6, speed: 1.0, jump: 1.0, resistance: 0.0 };

pub struct Physics {
    shapes: Vec<Vec<Aabb>>,
    /// Shape index per state id.
    state_shape: Vec<u16>,
    factors: Vec<Option<BlockFactors>>,
    /// Shapes that reach outside their block (fences, walls), which the
    /// game's collision search looks for one block further out.
    large: Vec<bool>,
}

static PHYSICS_26_3: LazyLock<Physics> = LazyLock::new(|| {
    Physics::from_json(Registry::get("26.3").unwrap(), include_str!("../../../data/26.3/physics.json"))
        .expect("bundled 26.3 physics data")
});

impl Physics {
    pub fn get(version: &str) -> Result<&'static Physics> {
        match version {
            "26.3" => Ok(&PHYSICS_26_3),
            v => bail!("no block physics for Minecraft {v}"),
        }
    }

    fn from_json(reg: &Registry, text: &str) -> Result<Physics> {
        let raw: serde_json::Value = serde_json::from_str(text)?;
        let states = raw["states"].as_u64().context("states")? as usize;
        let f64s = |v: &serde_json::Value| -> Result<Vec<f64>> {
            v.as_array().context("a number list")?.iter().map(|x| x.as_f64().context("a number")).collect()
        };
        let mut shapes = Vec::new();
        for s in raw["shapes"].as_array().context("shapes")? {
            let mut boxes = Vec::new();
            for b in s.as_array().context("a shape")? {
                let b = f64s(b)?;
                boxes.push(Aabb { min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] });
            }
            shapes.push(boxes);
        }
        let runs: Vec<Vec<f64>> =
            raw["shape_runs"].as_array().context("shape_runs")?.iter().map(f64s).collect::<Result<_>>()?;
        let mut state_shape = vec![0u16; states];
        for (i, r) in runs.iter().enumerate() {
            let end = runs.get(i + 1).map_or(states, |n| n[0] as usize);
            state_shape[r[0] as usize..end].fill(r[1] as u16);
        }
        let mut factors = vec![None; reg.len()];
        for (name, f) in raw["blocks"].as_object().context("blocks")? {
            let id = reg.block_id(name).with_context(|| format!("physics for unknown block {name}"))?;
            let num = |k: &str| f[k].as_f64().unwrap_or(0.0) as f32;
            factors[id.0 as usize] = Some(BlockFactors {
                friction: num("friction"),
                speed: num("speed"),
                jump: num("jump"),
                resistance: num("resistance"),
            });
        }
        let large = shapes.iter().map(|s| s.iter().any(|b| (0..3).any(|a| b.min[a] < 0.0 || b.max[a] > 1.0))).collect();
        Ok(Physics { shapes, state_shape, factors, large })
    }

    /// The state's collision boxes, relative to its block.
    pub fn shape(&self, state: StateId) -> &[Aabb] {
        &self.shapes[self.state_shape[state.0 as usize] as usize]
    }

    pub fn is_full_block(&self, state: StateId) -> bool {
        let s = self.shape(state);
        s.len() == 1 && s[0].min == [0.0; 3] && s[0].max == [1.0; 3]
    }

    pub fn has_large_shape(&self, state: StateId) -> bool {
        self.large[self.state_shape[state.0 as usize] as usize]
    }

    pub fn factors(&self, block: BlockId) -> BlockFactors {
        self.factors.get(block.0 as usize).copied().flatten().unwrap_or(DEFAULT_FACTORS)
    }
}
