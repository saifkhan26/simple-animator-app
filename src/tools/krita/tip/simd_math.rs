//! The floating-point routines Krita's auto brush tips are computed with,
//! reproduced operation for operation so their rounding is Krita's:
//!
//! - `exp` as xsimd 9–13 computes it for `float` on the `fma3<avx2>`
//!   architecture Krita picks on any modern x86 CPU (Cephes-style range
//!   reduction, a Horner polynomial through hardware FMA). `f32::mul_add`
//!   is a correctly rounded fused multiply-add on every target, so this
//!   agrees with the hardware instruction everywhere.
//! - `VcExtraMath::erf` (libs/image/vc_extra_math.h), the single-precision
//!   Abramowitz–Stegun erf the vector mask code uses.
//! - boost's 53-bit `erf` (boost/math/special_functions/erf.hpp), which the
//!   scalar mask code uses on Windows, with the polynomial evaluation order
//!   boost picks for Krita's clang/MinGW build (`BOOST_MATH_POLY_METHOD 3`).
//! - `KisFastMath::atan2` (libs/image/kis_fast_math.cpp), a table lookup.
//!
//! xsimd is BSD-3-Clause; boost is the Boost Software License 1.0.

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::OnceLock;

/// xsimd `exp(batch<float, fma3<avx2>>)`, one lane.
pub fn xsimd_exp(a: f32) -> f32 {
    const INVLOG_2: f32 = 1.442_695_040_888_963_407_359_924_681_001_892_137_426_645_954_152_986_f32;
    let log_2hi = f32::from_bits(0x3f31_8000);
    let log_2lo = f32::from_bits(0xb95e_8083);
    const MAXLOG: f32 = 88.376_262_664_794_9;
    const MINLOG: f32 = -88.376_262_664_794_9;

    // reduce: k = nearbyint(a / ln 2); x = a - k ln 2, in two fused steps.
    let k = (INVLOG_2 * a).round_ties_even();
    let x = (-k).mul_add(log_2hi, a);
    let x = (-k).mul_add(log_2lo, x);

    // approx: horner<0.5, 1/6, 1/24, …>(x), each step fma(x, rest, c).
    let c = [0x3f00_0000u32, 0x3e2a_a9a5, 0x3d2a_a957, 0x3c09_8d8b, 0x3ab7_78cf].map(f32::from_bits);
    let mut y = c[4];
    for &ci in c[..4].iter().rev() {
        y = x.mul_add(y, ci);
    }
    let r = y.mul_add(x * x, x) + 1.0;

    // ldexp(r, to_int(k)): the power of two built straight into the
    // exponent bits, which wraps for very negative k exactly as xsimd does.
    let ik = (k as i32).wrapping_add(127).wrapping_shl(23);
    let mut out = r * f32::from_bits(ik as u32);
    if a <= MINLOG {
        out = 0.0;
    }
    if a >= MAXLOG {
        out = f32::INFINITY;
    }
    out
}

/// `VcExtraMath::erf`, one lane: precise to about 1e-5, which is plenty
/// for an 8-bit mask.
pub fn vc_erf(x: f32) -> f32 {
    let mut xa = x.abs();
    let limit = xa >= 9.3;
    if limit {
        xa = 0.0;
    }
    let sign = if x < 0.0 { -1.0f32 } else { 1.0 };
    const A1: f32 = 0.254_829_592;
    const A2: f32 = -0.284_496_736;
    const A3: f32 = 1.421_413_741;
    const A4: f32 = -1.453_152_027;
    const A5: f32 = 1.061_405_429;
    const P: f32 = 0.327_591_1;
    let t = 1.0f32 / (1.0f32 + P * xa);
    let poly = (((A5 * t + A4) * t + A3) * t + A2) * t + A1;
    let mut y = 1.0f32 - poly * t * xsimd_exp(-xa * xa);
    if limit {
        y = 1.0;
    }
    sign * y
}

/// boost's `evaluate_polynomial`, method 3 (two interleaved Horner chains).
fn poly(a: &[f64], x: f64) -> f64 {
    let x2 = x * x;
    match a.len() {
        5 => {
            let mut t0 = a[4] * x2 + a[2];
            let mut t1 = a[3] * x2 + a[1];
            t0 *= x2;
            t0 += a[0];
            t1 *= x;
            t0 + t1
        }
        6 => {
            let mut t0 = a[5] * x2 + a[3];
            let mut t1 = a[4] * x2 + a[2];
            t0 *= x2;
            t1 *= x2;
            t0 += a[1];
            t1 += a[0];
            t0 *= x;
            t0 + t1
        }
        7 => {
            let mut t0 = a[6] * x2 + a[4];
            let mut t1 = a[5] * x2 + a[3];
            t0 *= x2;
            t1 *= x2;
            t0 += a[2];
            t1 += a[1];
            t0 *= x2;
            t0 += a[0];
            t1 *= x;
            t0 + t1
        }
        _ => unreachable!("erf polynomials have 5 to 7 terms"),
    }
}

/// `exp(-z²)` with the error of squaring `z` folded back in, as boost does
/// above 1.5.
fn exp_minus_z_squared(z: f64) -> f64 {
    // hi: z with its mantissa cut to 26 bits (frexp / ldexp / floor).
    let (m, e) = frexp(z);
    let hi = (m * 2f64.powi(26)).floor() * 2f64.powi(e - 26);
    let lo = z - hi;
    let sq = z * z;
    let err_sqr = ((hi * hi - sq) + 2.0 * hi * lo) + lo * lo;
    (-sq).exp() * (-err_sqr).exp()
}

/// `frexp` for finite, non-zero `x`: `x = m · 2^e` with `0.5 <= |m| < 1`.
fn frexp(x: f64) -> (f64, i32) {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        let (m, e) = frexp(x * 2f64.powi(54));
        return (m, e - 54);
    }
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, exp - 1022)
}

/// boost's `erf_imp` for 53-bit precision, `invert == false`.
fn erf_imp(z: f64, invert: bool) -> f64 {
    if z < 0.0 {
        return if !invert {
            -erf_imp(-z, invert)
        } else if z < -0.5 {
            2.0 - erf_imp(-z, invert)
        } else {
            1.0 + erf_imp(-z, false)
        };
    }
    let mut invert = invert;
    let result;
    if z < 0.5 {
        if z < 1e-10 {
            result = if z == 0.0 {
                0.0
            } else {
                const C: f64 = 0.003_379_167_095_512_573_896_158_903_121_545_171_688;
                z * 1.125 + z * C
            };
        } else {
            const Y: f64 = 1.044_948_577_880_859_375;
            const P: [f64; 5] = [
                0.083_430_589_214_653_183_290_7,
                -0.338_165_134_459_360_935_041,
                -0.050_999_073_514_677_743_284_1,
                -0.007_727_583_458_021_332_884_87,
                -0.000_322_780_120_964_605_683_831,
            ];
            const Q: [f64; 5] = [
                1.0,
                0.455_004_033_050_794_024_546,
                0.087_522_260_014_225_254_955_4,
                0.008_585_719_250_744_062_127_72,
                0.000_370_900_071_787_748_000_569,
            ];
            let zz = z * z;
            result = z * (Y + poly(&P, zz) / poly(&Q, zz));
        }
    } else if if invert { z < 28.0 } else { z < 5.93f32 as f64 } {
        invert = !invert;
        if z < 1.5 {
            const Y: f64 = 0.405_935_764_312_744_140_625;
            const P: [f64; 6] = [
                -0.098_090_592_216_281_240_205,
                0.178_114_665_841_120_341_155,
                0.191_003_695_796_775_433_986,
                0.088_890_036_896_788_446_657_8,
                0.019_504_900_125_121_880_135_9,
                0.001_804_245_382_970_142_239_57,
            ];
            const Q: [f64; 7] = [
                1.0,
                1.847_590_709_830_022_178_45,
                1.426_280_048_455_113_245_08,
                0.578_052_804_889_902_404_909,
                0.123_850_974_679_008_642_33,
                0.011_338_523_357_700_141_101_7,
                0.337_511_472_483_094_676_155e-5,
            ];
            let r = Y + poly(&P, z - 0.5) / poly(&Q, z - 0.5);
            result = r * ((-z * z).exp() / z);
        } else if z < 2.5 {
            const Y: f64 = 0.506_728_172_302_246_093_75;
            const P: [f64; 6] = [
                -0.024_350_047_620_769_844_127_2,
                0.038_654_037_503_570_720_172_8,
                0.043_948_189_642_095_162_96,
                0.017_567_943_631_180_209_229_9,
                0.003_239_624_062_908_421_335_84,
                0.000_235_839_115_596_880_717_416,
            ];
            const Q: [f64; 6] = [
                1.0,
                1.539_914_949_485_524_471_82,
                0.982_403_709_157_920_235_114,
                0.325_732_924_782_444_448_493,
                0.056_392_183_742_047_816_037_3,
                0.004_103_697_239_789_045_758_84,
            ];
            let r = Y + poly(&P, z - 1.5) / poly(&Q, z - 1.5);
            result = r * (exp_minus_z_squared(z) / z);
        } else if z < 4.5 {
            const Y: f64 = 0.540_575_027_465_820_312_5;
            const P: [f64; 6] = [
                0.002_952_767_165_309_716_626_34,
                0.013_738_442_589_635_533_212_6,
                0.008_408_076_155_555_853_830_07,
                0.002_128_256_209_146_186_491_41,
                0.000_250_269_961_544_794_627_958,
                0.113_212_406_648_847_561_139e-4,
            ];
            const Q: [f64; 6] = [
                1.0,
                1.042_178_141_669_384_181_71,
                0.442_597_659_481_563_127_003,
                0.095_849_272_630_106_142_344_4,
                0.010_598_290_648_487_653_148_9,
                0.000_479_411_269_521_714_493_907,
            ];
            let r = Y + poly(&P, z - 3.5) / poly(&Q, z - 3.5);
            result = r * (exp_minus_z_squared(z) / z);
        } else {
            const Y: f64 = 0.557_909_011_840_820_312_5;
            const P: [f64; 7] = [
                0.006_280_571_706_269_648_919_37,
                0.017_538_983_405_249_330_881_8,
                -0.212_652_252_872_804_219_852,
                -0.687_717_681_153_649_930_619,
                -2.551_855_172_731_152_399_6,
                -3.227_294_517_641_437_185_17,
                -2.817_540_111_451_337_877_1,
            ];
            const Q: [f64; 7] = [
                1.0,
                2.792_577_509_805_752_822_28,
                11.056_723_792_780_016_156_5,
                15.930_646_027_911_794_143,
                22.936_737_652_288_057_722_4,
                13.506_417_019_180_288_914_5,
                5.484_091_822_386_417_415_84,
            ];
            let r = Y + poly(&P, 1.0 / z) / poly(&Q, 1.0 / z);
            result = r * (exp_minus_z_squared(z) / z);
        }
    } else {
        result = 0.0;
        invert = !invert;
    }
    if invert {
        1.0 - result
    } else {
        result
    }
}

/// `boost::math::erf`, double precision.
pub fn boost_erf(z: f64) -> f64 {
    erf_imp(z, false)
}

/// `KisATanTable`: atan sampled across 0..=1 for the fast arctangent.
fn atan_table() -> &'static [f64] {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    TABLE.get_or_init(|| {
        const MAX_SECOND_DERIV_IN_RANGE: f64 = 0.6495;
        const MAX_ERROR: f64 = 0.0001;
        let nf = (MAX_SECOND_DERIV_IN_RANGE / (8.0 * MAX_ERROR)).sqrt();
        let n = nf as u32 + 1;
        let y = 10.0f64;
        let mut t = vec![0.0; n as usize + 1];
        for i in 1..=n {
            let x = (y / i as f64) * n as f64;
            t[i as usize] = y.atan2(x);
        }
        t
    })
}

fn calc_angle(x: f64, y: f64) -> f64 {
    let t = atan_table();
    let n = t.len() - 1;
    let di = (y / x) * n as f64;
    let i = di as i32;
    if i as usize >= n {
        return y.atan2(x);
    }
    let i = i as usize;
    (di - i as f64) * (t[i + 1] - t[i]) + t[i]
}

/// `KisFastMath::atan2`.
pub fn fast_atan2(y: f64, x: f64) -> f64 {
    if y == 0.0 {
        return if x >= 0.0 { 0.0 } else { PI };
    }
    if x == 0.0 {
        return if y > 0.0 { FRAC_PI_2 } else { -FRAC_PI_2 };
    }
    if x > 0.0 {
        if y > 0.0 {
            if x > y {
                calc_angle(x, y)
            } else {
                FRAC_PI_2 - calc_angle(y, x)
            }
        } else {
            let y = -y;
            if x > y {
                -calc_angle(x, y)
            } else {
                -FRAC_PI_2 + calc_angle(y, x)
            }
        }
    } else {
        let x = -x;
        if y > 0.0 {
            if x > y {
                PI - calc_angle(x, y)
            } else {
                FRAC_PI_2 + calc_angle(y, x)
            }
        } else {
            let y = -y;
            if x > y {
                -PI + calc_angle(x, y)
            } else {
                -FRAC_PI_2 - calc_angle(y, x)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsimd_exp_is_close_to_exp_and_exact_at_zero() {
        assert_eq!(xsimd_exp(0.0), 1.0);
        for i in -800..=800 {
            let x = i as f32 * 0.1;
            let want = (x as f64).exp();
            let got = xsimd_exp(x) as f64;
            assert!(((got - want) / want).abs() < 4e-7, "{x}: {got} vs {want}");
        }
        assert_eq!(xsimd_exp(-90.0), 0.0);
        assert_eq!(xsimd_exp(89.0), f32::INFINITY);
    }

    #[test]
    fn the_erfs_agree_with_each_other() {
        for i in -400..=400 {
            let x = i as f64 * 0.01;
            let b = boost_erf(x);
            let v = vc_erf(x as f32) as f64;
            assert!((b - v).abs() < 2e-6, "{x}: {b} vs {v}");
        }
        // Known values.
        assert!((boost_erf(0.5) - 0.520_499_877_813_046_5).abs() < 1e-16);
        assert!((boost_erf(1.0) - 0.842_700_792_949_714_9).abs() < 1e-16);
        assert!((boost_erf(2.0) - 0.995_322_265_018_952_7).abs() < 1e-16);
        assert!((boost_erf(-3.0) + 0.999_977_909_503_001_4).abs() < 1e-16);
        assert_eq!(boost_erf(7.0), 1.0);
    }

    #[test]
    fn fast_atan2_stays_within_its_error_bound() {
        for i in 0..360 {
            let a = (i as f64 + 0.5).to_radians();
            let (y, x) = a.sin_cos();
            let got = fast_atan2(y, x);
            let want = y.atan2(x);
            assert!((got - want).abs() < 2e-4, "{i}: {got} vs {want}");
        }
        assert_eq!(fast_atan2(0.0, -1.0), PI);
    }
}
