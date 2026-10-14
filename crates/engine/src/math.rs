//! `Math.sin` and `Math.cos` as the game's server computes them.
//!
//! Java lets `Math.sin` be off by up to an ulp, and the platform's `sin`
//! (which Rust calls) is off by one in a few hard cases where the true value
//! lies almost halfway between two doubles. In the 26.3 captures, which run
//! on x86-64, the game's result was the correctly rounded one every time,
//! including five TNT kicks where the platform's was not
//! (`tnt_cannon@r0~28` and others). So these are correctly rounded: the value
//! is worked out in double-double arithmetic, about 100 bits, and rounded
//! once. Arguments are the angles the game draws, below 2π; larger ones
//! would need a longer reduction.
//!
//! Known difference: the game's server (HotSpot on x86-64) is not always
//! correctly rounded either. For about 1 kick angle in 400 its sine or
//! cosine differs from these in the last bit. The committed captures happen
//! not to draw such an angle. `harness/kick-trig.sh` measures how many
//! differ on every oracle run; see HANDOFF.md.

/// A value as an unevaluated sum `hi + lo`, with `|lo|` at most half an ulp
/// of `hi`.
#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

/// `a + b` exactly, as a rounded sum and its error.
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd { hi: s, lo: (a - (s - bb)) + (b - bb) }
}

/// Dekker's split of `a` into two halves of 26 bits each.
fn split(a: f64) -> (f64, f64) {
    let t = 134217729.0 * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

/// `a * b` exactly, as a rounded product and its error, without relying on
/// a fused multiply-add so every platform gets the same bits.
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    Dd { hi: p, lo: ((ah * bh - p) + ah * bl + al * bh) + al * bl }
}

impl Dd {
    fn new(v: f64) -> Dd {
        Dd { hi: v, lo: 0.0 }
    }

    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.hi, o.hi);
        let t = two_sum(self.lo, o.lo);
        let s = two_sum(s.hi, s.lo + t.hi);
        two_sum(s.hi, s.lo + t.lo)
    }

    fn neg(self) -> Dd {
        Dd { hi: -self.hi, lo: -self.lo }
    }

    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.hi, o.hi);
        two_sum(p.hi, p.lo + (self.hi * o.lo + self.lo * o.hi))
    }

    /// Division by a small integer that a double holds exactly.
    fn div(self, d: f64) -> Dd {
        let q = self.hi / d;
        let r = self.add(two_prod(q, d).neg());
        let q2 = r.hi / d;
        two_sum(q, q2)
    }
}

/// π/2 in four pieces (fdlibm's `pio2_1` to `pio2_3t`); the first three
/// have 33 bits, so a small multiple of each is exact.
const PIO2: [f64; 4] = [1.5707963267341256, 6.077100506303966e-11, 2.0222662487111665e-21, 8.4784276603689e-32];

/// `x` minus the nearest multiple `k` of π/2, and `k` modulo 4.
fn reduce(x: f64) -> (Dd, i32) {
    assert!(x.abs() < 8.0, "Math.sin and Math.cos are only exact below 8, not {x}");
    let k = (x * std::f64::consts::FRAC_2_PI).round();
    // x - k * pio2_1 is exact: the two are within a factor of two.
    let mut r = Dd::new(x - k * PIO2[0]);
    for p in &PIO2[1..] {
        r = r.add(Dd::new(-k * p));
    }
    (r, (k as i32).rem_euclid(4))
}

/// Taylor series of sin (`odd`) or cos around 0, for `|r| <= π/4`.
fn series(r: Dd, odd: bool) -> Dd {
    let r2 = r.mul(r);
    let mut term = if odd { r } else { Dd::new(1.0) };
    let mut sum = term;
    let mut n = if odd { 2.0 } else { 1.0 };
    for _ in 0..20 {
        term = term.mul(r2).div(n * (n + 1.0)).neg();
        sum = sum.add(term);
        n += 2.0;
    }
    sum
}

/// `Math.sin`.
pub fn sin(x: f64) -> f64 {
    let (r, k) = reduce(x);
    let v = match k {
        0 => series(r, true),
        1 => series(r, false),
        2 => series(r, true).neg(),
        _ => series(r, false).neg(),
    };
    v.hi + v.lo
}

/// `Math.cos`.
pub fn cos(x: f64) -> f64 {
    let (r, k) = reduce(x);
    let v = match k {
        0 => series(r, false),
        1 => series(r, true).neg(),
        2 => series(r, false).neg(),
        _ => series(r, true),
    };
    v.hi + v.lo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_cases_round_as_the_game_does() {
        // Angles from tnt_cannon's seeds, and the sines the game used.
        for (x, s) in [
            (0.16773039758153624, 0.16694502967981745),
            (0.2791759897810657, 0.2755636353298531),
            (6.01659277419589, -0.2634458770178357),
            (3.777547258812227, -0.593945769612226),
            (5.2375781634190455, -0.8652291051413569),
        ] {
            assert_eq!(sin(x), s, "sin({x})");
        }
    }

    #[test]
    fn agrees_with_the_platform_to_an_ulp() {
        let ulps = |a: f64, b: f64| (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
        let mut x = 0.0001;
        while x < std::f64::consts::TAU {
            assert!(ulps(sin(x), x.sin()) <= 1, "sin({x}): {} vs {}", sin(x), x.sin());
            assert!(ulps(cos(x), x.cos()) <= 1, "cos({x}): {} vs {}", cos(x), x.cos());
            x += 0.000123;
        }
        assert_eq!((sin(0.0), cos(0.0)), (0.0, 1.0));
    }
}
