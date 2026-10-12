//! `LegacyRandomSource`: the 48-bit linear congruential generator behind
//! the level's random source, with Minecraft's own bounded and floating
//! point draws (`BitRandomSource`), which differ from `java.util.Random`'s in
//! places.

const MULTIPLIER: i64 = 0x5DEECE66D;
const INCREMENT: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct JavaRandom {
    seed: i64,
    next_gaussian: Option<f64>,
}

impl JavaRandom {
    /// `new LegacyRandomSource(seed)`.
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom { seed: 0, next_gaussian: None };
        r.set_seed(seed);
        r
    }

    /// From the scrambled internal seed, as the harness records it.
    pub fn from_internal(seed: i64) -> Self {
        JavaRandom { seed: seed & MASK, next_gaussian: None }
    }

    /// The scrambled internal seed.
    pub fn internal(&self) -> i64 {
        self.seed
    }

    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed ^ MULTIPLIER) & MASK;
        self.next_gaussian = None;
    }

    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT)) & MASK;
        (self.seed >> (48 - bits)) as i32
    }

    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `BitRandomSource.nextInt(bound)`.
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if bound & -bound == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let j = self.next(31);
            let k = j % bound;
            if j.wrapping_sub(k).wrapping_add(bound - 1) >= 0 {
                return k;
            }
        }
    }

    /// `RandomSource.nextIntBetweenInclusive`.
    pub fn next_int_between_inclusive(&mut self, min: i32, max: i32) -> i32 {
        self.next_int_bounded(max - min + 1) + min
    }

    pub fn next_long(&mut self) -> i64 {
        let hi = self.next(32) as i64;
        let lo = self.next(32) as i64;
        (hi << 32).wrapping_add(lo)
    }

    pub fn next_boolean(&mut self) -> bool {
        self.next(1) != 0
    }

    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5e-8_f32
    }

    /// `BitRandomSource.nextDouble`: note the float constant.
    pub fn next_double(&mut self) -> f64 {
        let i = self.next(26) as i64;
        let j = self.next(27) as i64;
        ((i << 27) + j) as f64 * (1.110_223e-16_f32 as f64)
    }

    /// `MarsagliaPolarGaussian.nextGaussian`.
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(g) = self.next_gaussian.take() {
            return g;
        }
        loop {
            let a = 2.0 * self.next_double() - 1.0;
            let b = 2.0 * self.next_double() - 1.0;
            let s = a * a + b * b;
            if s < 1.0 && s != 0.0 {
                let m = (-2.0 * s.ln() / s).sqrt();
                self.next_gaussian = Some(b * m);
                return a * m;
            }
        }
    }

    /// `RandomSource.triangle`.
    pub fn triangle(&mut self, mode: f64, deviation: f64) -> f64 {
        mode + deviation * (self.next_double() - self.next_double())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_util_random() {
        // new java.util.Random(42): nextInt() -1170105035, nextInt(10) 0,
        // nextLong() -6519408338692630574 after those, as Java prints them.
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_int(), -1170105035);
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_int_bounded(10), 0);
        assert_eq!(r.next_int_bounded(10), 3);
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_long(), -5025562857975149833);
    }
}
