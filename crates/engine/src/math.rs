//! `Math.sin` and `Math.cos` as the game's server computes them.
//!
//! Java lets `Math.sin` and `Math.cos` be an ulp off, so their last bit
//! depends on the JVM and the CPU. The captures run the server on HotSpot on
//! x86-64, where every tier (the interpreter, C1 and C2) calls one stub that
//! HotSpot generates from Intel's LIBM (`StubGenerator::generate_libmSin`
//! and `generate_libmCos`). It is not correctly rounded: for about one kick
//! angle in 400 its sine or cosine differs from the correctly rounded value
//! by an ulp. So this module does the stub's arithmetic, operation for
//! operation, and gives the same bits for every angle the kick of newly
//! primed TNT draws (`harness/kick-trig.sh` compares a million of them with
//! the oracle runner's JVM). It uses only double additions and
//! multiplications, which round the same on every platform, WebAssembly
//! included; Rust never fuses them into a multiply-add.
//!
//! HotSpot on arm64 computes these differently (on about one kick angle in
//! six it differs from x86-64), so a server on an arm64 machine can throw
//! TNT a little differently from the one the captures ran on.

/// `32 / π`.
const PI32INV: f64 = f64::from_bits(0x40245f306dc9c883);
/// `π / 32` in three pieces. The first two have 32 bits or fewer, so a
/// multiple `n` of them is exact for any `n` below 2^20.
const P_1: f64 = f64::from_bits(0x3fb921fb54400000);
const P_2: f64 = f64::from_bits(0x3d90b4611a600000);
const P_3: f64 = f64::from_bits(0x3b63198a2e037073);

/// Taylor coefficients, for the sine of `r` (first) and the cosine (second):
/// `-1/3!` and `-1/2!`, `1/5!` and `1/4!`, `-1/7!` and `-1/6!`, `1/9!` and
/// `1/8!`.
const SC_1: [f64; 2] = [f64::from_bits(0xbfc5555555555555), f64::from_bits(0xbfe0000000000000)];
const SC_2: [f64; 2] = [f64::from_bits(0x3f81111111111111), f64::from_bits(0x3fa5555555555555)];
const SC_3: [f64; 2] = [f64::from_bits(0xbf2a01a01a01a01a), f64::from_bits(0xbf56c16c16c16c17)];
const SC_4: [f64; 2] = [f64::from_bits(0x3ec71de3a556c734), f64::from_bits(0x3efa01a01a01a01a)];

/// For `B = m * π / 32`, `m` from 0 to 63, as bits: `cos(B) - sigma`,
/// `sin(B)`, the rest of `sin(B)` to about 24 bits, and `sigma`, the power
/// of two (or zero) nearest `cos(B)`. The tests check these against the
/// values worked out to about 100 bits.
#[rustfmt::skip]
const TABLE: [[u64; 4]; 64] = [
    [0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0x3ff0000000000000],
    [0xbf73b92e176d6d31, 0x3fb917a6bc29b42c, 0xbc3e2718e0000000, 0x3ff0000000000000],
    [0xbf93ad06011469fb, 0x3fc8f8b83c69a60b, 0xbc626d19c0000000, 0x3ff0000000000000],
    [0xbfa60bea939d225a, 0x3fd294062ed59f06, 0xbc75d28da0000000, 0x3ff0000000000000],
    [0xbfb37ca1866b95cf, 0x3fd87de2a6aea963, 0xbc672cede0000000, 0x3ff0000000000000],
    [0xbfbe3a6873fa1279, 0x3fde2b5d3806f63b, 0x3c5e0d8920000000, 0x3ff0000000000000],
    [0xbfc592675bc57974, 0x3fe1c73b39ae68c8, 0x3c8b25dd20000000, 0x3ff0000000000000],
    [0xbfcd0dfe53aba2fd, 0x3fe44cf325091dd6, 0x3c68076a20000000, 0x3ff0000000000000],
    [0x3fca827999fcef32, 0x3fe6a09e667f3bcd, 0xbc8bdd3420000000, 0x3fe0000000000000],
    [0x3fc133cc94247758, 0x3fe8bc806b151741, 0xbc82c5e120000000, 0x3fe0000000000000],
    [0x3fac73b39ae68c87, 0x3fea9b66290ea1a3, 0x3c39f630e0000000, 0x3fe0000000000000],
    [0xbf9d4a2c7f909c4e, 0x3fec38b2f180bdb1, 0xbc76e0b180000000, 0x3fe0000000000000],
    [0xbfbe087565455a75, 0x3fed906bcf328d46, 0x3c7457e620000000, 0x3fe0000000000000],
    [0x3fa4a03176acf82d, 0x3fee9f4156c62dda, 0x3c8760b1e0000000, 0x3fd0000000000000],
    [0xbfac1d1f0e5967d5, 0x3fef6297cff75cb0, 0x3c75621720000000, 0x3fd0000000000000],
    [0xbf9ba1650f592f50, 0x3fefd88da3d12526, 0xbc887df640000000, 0x3fc0000000000000],
    [0x0000000000000000, 0x3ff0000000000000, 0x0000000000000000, 0x0000000000000000],
    [0x3f9ba1650f592f50, 0x3fefd88da3d12526, 0xbc887df640000000, 0xbfc0000000000000],
    [0x3fac1d1f0e5967d5, 0x3fef6297cff75cb0, 0x3c75621720000000, 0xbfd0000000000000],
    [0xbfa4a03176acf82d, 0x3fee9f4156c62dda, 0x3c8760b1e0000000, 0xbfd0000000000000],
    [0x3fbe087565455a75, 0x3fed906bcf328d46, 0x3c7457e620000000, 0xbfe0000000000000],
    [0x3f9d4a2c7f909c4e, 0x3fec38b2f180bdb1, 0xbc76e0b180000000, 0xbfe0000000000000],
    [0xbfac73b39ae68c87, 0x3fea9b66290ea1a3, 0x3c39f630e0000000, 0xbfe0000000000000],
    [0xbfc133cc94247758, 0x3fe8bc806b151741, 0xbc82c5e120000000, 0xbfe0000000000000],
    [0xbfca827999fcef32, 0x3fe6a09e667f3bcd, 0xbc8bdd3420000000, 0xbfe0000000000000],
    [0x3fcd0dfe53aba2fd, 0x3fe44cf325091dd6, 0x3c68076a20000000, 0xbff0000000000000],
    [0x3fc592675bc57974, 0x3fe1c73b39ae68c8, 0x3c8b25dd20000000, 0xbff0000000000000],
    [0x3fbe3a6873fa1279, 0x3fde2b5d3806f63b, 0x3c5e0d8920000000, 0xbff0000000000000],
    [0x3fb37ca1866b95cf, 0x3fd87de2a6aea963, 0xbc672cede0000000, 0xbff0000000000000],
    [0x3fa60bea939d225a, 0x3fd294062ed59f06, 0xbc75d28da0000000, 0xbff0000000000000],
    [0x3f93ad06011469fb, 0x3fc8f8b83c69a60b, 0xbc626d19c0000000, 0xbff0000000000000],
    [0x3f73b92e176d6d31, 0x3fb917a6bc29b42c, 0xbc3e2718e0000000, 0xbff0000000000000],
    [0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0xbff0000000000000],
    [0x3f73b92e176d6d31, 0xbfb917a6bc29b42c, 0x3c3e2718e0000000, 0xbff0000000000000],
    [0x3f93ad06011469fb, 0xbfc8f8b83c69a60b, 0x3c626d19c0000000, 0xbff0000000000000],
    [0x3fa60bea939d225a, 0xbfd294062ed59f06, 0x3c75d28da0000000, 0xbff0000000000000],
    [0x3fb37ca1866b95cf, 0xbfd87de2a6aea963, 0x3c672cede0000000, 0xbff0000000000000],
    [0x3fbe3a6873fa1279, 0xbfde2b5d3806f63b, 0xbc5e0d8920000000, 0xbff0000000000000],
    [0x3fc592675bc57974, 0xbfe1c73b39ae68c8, 0xbc8b25dd20000000, 0xbff0000000000000],
    [0x3fcd0dfe53aba2fd, 0xbfe44cf325091dd6, 0xbc68076a20000000, 0xbff0000000000000],
    [0xbfca827999fcef32, 0xbfe6a09e667f3bcd, 0x3c8bdd3420000000, 0xbfe0000000000000],
    [0xbfc133cc94247758, 0xbfe8bc806b151741, 0x3c82c5e120000000, 0xbfe0000000000000],
    [0xbfac73b39ae68c87, 0xbfea9b66290ea1a3, 0xbc39f630e0000000, 0xbfe0000000000000],
    [0x3f9d4a2c7f909c4e, 0xbfec38b2f180bdb1, 0x3c76e0b180000000, 0xbfe0000000000000],
    [0x3fbe087565455a75, 0xbfed906bcf328d46, 0xbc7457e620000000, 0xbfe0000000000000],
    [0xbfa4a03176acf82d, 0xbfee9f4156c62dda, 0xbc8760b1e0000000, 0xbfd0000000000000],
    [0x3fac1d1f0e5967d5, 0xbfef6297cff75cb0, 0xbc75621720000000, 0xbfd0000000000000],
    [0x3f9ba1650f592f50, 0xbfefd88da3d12526, 0x3c887df640000000, 0xbfc0000000000000],
    [0x0000000000000000, 0xbff0000000000000, 0x0000000000000000, 0x0000000000000000],
    [0xbf9ba1650f592f50, 0xbfefd88da3d12526, 0x3c887df640000000, 0x3fc0000000000000],
    [0xbfac1d1f0e5967d5, 0xbfef6297cff75cb0, 0xbc75621720000000, 0x3fd0000000000000],
    [0x3fa4a03176acf82d, 0xbfee9f4156c62dda, 0xbc8760b1e0000000, 0x3fd0000000000000],
    [0xbfbe087565455a75, 0xbfed906bcf328d46, 0xbc7457e620000000, 0x3fe0000000000000],
    [0xbf9d4a2c7f909c4e, 0xbfec38b2f180bdb1, 0x3c76e0b180000000, 0x3fe0000000000000],
    [0x3fac73b39ae68c87, 0xbfea9b66290ea1a3, 0xbc39f630e0000000, 0x3fe0000000000000],
    [0x3fc133cc94247758, 0xbfe8bc806b151741, 0x3c82c5e120000000, 0x3fe0000000000000],
    [0x3fca827999fcef32, 0xbfe6a09e667f3bcd, 0x3c8bdd3420000000, 0x3fe0000000000000],
    [0xbfcd0dfe53aba2fd, 0xbfe44cf325091dd6, 0xbc68076a20000000, 0x3ff0000000000000],
    [0xbfc592675bc57974, 0xbfe1c73b39ae68c8, 0xbc8b25dd20000000, 0x3ff0000000000000],
    [0xbfbe3a6873fa1279, 0xbfde2b5d3806f63b, 0xbc5e0d8920000000, 0x3ff0000000000000],
    [0xbfb37ca1866b95cf, 0xbfd87de2a6aea963, 0x3c672cede0000000, 0x3ff0000000000000],
    [0xbfa60bea939d225a, 0xbfd294062ed59f06, 0x3c75d28da0000000, 0x3ff0000000000000],
    [0xbf93ad06011469fb, 0xbfc8f8b83c69a60b, 0x3c626d19c0000000, 0x3ff0000000000000],
    [0xbf73b92e176d6d31, 0xbfb917a6bc29b42c, 0x3c3e2718e0000000, 0x3ff0000000000000],
];

/// Where `x` falls: `Main` for `2^-252 <= |x| < 90112`, the only range the
/// game's angles reach; `Tiny` below it, zero included.
enum Range {
    Tiny,
    Main,
}

fn range(x: f64) -> Range {
    // The stub's own test, on the top 16 bits of `x`.
    let t = (((x.to_bits() >> 32) as u32) & 0x7fff_0000).wrapping_sub(0x3030_0000);
    if t <= 0x10c5_0000 {
        Range::Main
    } else {
        assert!((t as i32) < 0x10c5_0000, "Math.sin and Math.cos are only worked out below 90112, not {x}");
        Range::Tiny
    }
}

/// `sin(x + quarter * π / 2)`, for `x` in the main range.
///
/// Reduces `x` to `n * π / 32 + r + c`, looks up `B = (n + 16 * quarter)
/// * π / 32`, and adds up
///
/// ```text
/// sin(B + r + c) = sin(B) + sigma * r + (cos(B) - sigma) * r
///                + sin(B) * (cos(r) - 1) + cos(B) * (sin(r) - r)
///                + c * (cos(B) - sin(B) * r)
/// ```
///
/// in the stub's order, the large terms with their rounding errors kept.
fn kernel(x: f64, quarter: i32) -> f64 {
    let n = (x * PI32INV + 0.5f64.copysign(x)) as i32;
    let nf = n as f64;
    let row = &TABLE[(n.wrapping_add(16 * quarter) & 63) as usize];
    let [c_hl, s_hi, s_lo, sigma] = row.map(f64::from_bits);

    // x - n * (P_1 + P_2 + P_3) = r + c.
    let m2 = P_2 * nf;
    let r1 = x - P_1 * nf;
    let r = r1 - m2;
    let neg_c = nf * P_3 - ((r1 - r) - m2);

    // The polynomials for sin(r) - r and cos(r) - 1, side by side.
    let r2 = r * r;
    let r4 = r2 * r2;
    let poly = |i: usize| (SC_1[i] + SC_2[i] * r2) + (SC_3[i] + SC_4[i] * r1 * r) * r4;
    let cos_b = c_hl + sigma;
    let pol_s = poly(0) * (cos_b * r * r2);
    let pol_c = poly(1) * (s_hi * r2);

    let rs = sigma * r;
    let med = r * c_hl;
    let res_int = rs + s_hi;
    let res_hi = med + res_int;
    let corr = neg_c * (s_hi * r - cos_b) + s_lo;
    let k2 = (s_hi - res_int) + rs;
    let k3 = (res_int - res_hi) + med;
    res_hi + ((((corr + k2) + k3) + pol_s) + pol_c)
}

/// `Math.sin`.
pub fn sin(x: f64) -> f64 {
    match range(x) {
        Range::Main => kernel(x, 0),
        // Zero and subnormal numbers, then the rest of the tiny ones.
        Range::Tiny if x.to_bits() & 0x7ff0_0000_0000_0000 == 0 => x * f64::from_bits(0x3fefffffffffffff),
        Range::Tiny => (x * f64::from_bits(0x4360000000000000) - x) * f64::from_bits(0x3c80000000000000),
    }
}

/// `Math.cos`.
pub fn cos(x: f64) -> f64 {
    match range(x) {
        Range::Main => kernel(x, 1),
        Range::Tiny => 1.0 - x.abs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value as an unevaluated sum `hi + lo`, with `|lo|` at most half an
    /// ulp of `hi`: the reference the stub is checked against.
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

    /// `a * b` exactly, as a rounded product and its error (Dekker).
    fn two_prod(a: f64, b: f64) -> Dd {
        let split = |a: f64| {
            let t = 134217729.0 * a;
            let hi = t - (t - a);
            (hi, a - hi)
        };
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
            two_sum(q, r.hi / d)
        }

        /// Rounded once to a double.
        fn round(self) -> f64 {
            self.hi + self.lo
        }
    }

    /// π/2 in four pieces (fdlibm's `pio2_1` to `pio2_3t`); the first three
    /// have 33 bits, so a small multiple of each is exact.
    const PIO2: [f64; 4] = [1.5707963267341256, 6.077100506303966e-11, 2.0222662487111665e-21, 8.4784276603689e-32];

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

    /// Sine and cosine of `r + k * π / 2`.
    fn quadrant(r: Dd, k: i32) -> (Dd, Dd) {
        let (s, c) = (series(r, true), series(r, false));
        match k.rem_euclid(4) {
            0 => (s, c),
            1 => (c, s.neg()),
            2 => (s.neg(), c.neg()),
            _ => (c.neg(), s),
        }
    }

    /// Sine and cosine of `x` (below 8), to about 100 bits.
    fn exact(x: f64) -> (Dd, Dd) {
        let k = (x * std::f64::consts::FRAC_2_PI).round();
        // x - k * pio2_1 is exact: the two are within a factor of two.
        let mut r = Dd::new(x - k * PIO2[0]);
        for p in &PIO2[1..] {
            r = r.add(Dd::new(-k * p));
        }
        quadrant(r, k as i32)
    }

    /// Sine and cosine of `m * π / 32`, to about 100 bits.
    fn exact_table(m: i32) -> (Dd, Dd) {
        let k = (m as f64 / 16.0).round() as i32;
        let j = (m - 16 * k) as f64;
        let r = PIO2.iter().fold(Dd::new(0.0), |r, p| r.add(Dd::new(p * j / 16.0)));
        quadrant(r, k)
    }

    #[test]
    fn rounds_as_the_game_does() {
        // Kick angles from tnt_cannon's seeds, and the sines the game used:
        // the platform's sin is an ulp off on these.
        for (x, s) in [
            (0.16773039758153624, 0.16694502967981745),
            (0.2791759897810657, 0.2755636353298531),
            (6.01659277419589, -0.2634458770178357),
            (3.777547258812227, -0.593945769612226),
            (5.2375781634190455, -0.8652291051413569),
        ] {
            assert_eq!(sin(x), s, "sin({x})");
        }
        // Kick angles from harness/kick-trig.sh where the oracle runner's
        // JVM (Temurin 25.0.4.1 on x86-64) is not correctly rounded, and
        // what it gave.
        for (x, s, c) in [
            (4.0691711172702325, Some(-0.8001699153137781), None),
            (4.9755611016053125, Some(-0.9655696267535826), None),
            (2.0049151577019253, None, Some(-0.4206111320135118)),
            (0.5211042632243493, None, Some(0.8672699642179156)),
            (0.7548808569109708, Some(0.6852018952724912), None),
            (5.912670801448523, None, Some(0.9321411690010072)),
        ] {
            let (es, ec) = exact(x);
            assert_eq!(sin(x), s.unwrap_or(es.round()), "sin({x})");
            assert_eq!(cos(x), c.unwrap_or(ec.round()), "cos({x})");
            assert!(s.is_some_and(|s| s != es.round()) || c.is_some_and(|c| c != ec.round()));
        }
    }

    #[test]
    fn table_holds_sines_and_cosines() {
        for (m, row) in TABLE.iter().enumerate() {
            let [c_hl, s_hi, s_lo, sigma] = row.map(f64::from_bits);
            let (s, c) = exact_table(m as i32);
            assert_eq!(s_hi, s.round(), "sin({m}π/32)");
            // The rest of the sine, to 24 bits.
            let rest = s.add(Dd::new(-s_hi)).add(Dd::new(-s_lo)).round();
            assert!(rest.abs() <= s_lo.abs() * f64::EPSILON * (1 << 28) as f64, "rest of sin({m}π/32)");
            if sigma == 0.0 {
                assert!(c.round().abs() < 1e-30, "cos({m}π/32) is 0");
            } else {
                let (cos, to) = (c.round(), |p: f64| (c.round() - p).abs());
                assert_eq!(sigma.abs().to_bits() & ((1 << 52) - 1), 0, "sigma for {m} is a power of two");
                let nearest = to(sigma) <= to(sigma * 2.0) && to(sigma) <= to(sigma / 2.0);
                assert!(
                    sigma.signum() == cos.signum() && nearest,
                    "sigma for {m} is the power of two nearest the cosine"
                );
            }
            assert_eq!(c_hl, c.add(Dd::new(-sigma)).round(), "cos({m}π/32) - sigma");
        }
    }

    #[test]
    fn stays_within_an_ulp_of_the_true_value() {
        let ulps = |a: f64, b: f64| (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
        let mut x = 0.0001;
        while x < std::f64::consts::TAU {
            let (s, c) = exact(x);
            assert!(ulps(sin(x), s.round()) <= 1, "sin({x}): {} vs {}", sin(x), s.round());
            assert!(ulps(cos(x), c.round()) <= 1, "cos({x}): {} vs {}", cos(x), c.round());
            x += 0.000123;
        }
    }

    #[test]
    fn zero_and_tiny_angles() {
        assert_eq!((sin(0.0).to_bits(), cos(0.0)), (0, 1.0));
        assert_eq!(sin(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!((sin(1e-300), cos(1e-300)), (1e-300, 1.0));
    }
}
