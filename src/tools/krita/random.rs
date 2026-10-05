//! `KisRandomSource`: the random numbers a Krita stroke draws on for
//! scatter, fuzzy sensors, a pipe tip's cell and an auto tip's density.
//!
//! Krita's is boost's `taus88` (L'Ecuyer's three-component Tausworthe
//! generator, 2^88 period), seeded once per stroke from `qrand()`, so no two
//! strokes in Krita repeat each other either. Ported here exactly, from
//! boost's `linear_feedback_shift.hpp`, `xor_combine.hpp`,
//! `uniform_smallint.hpp` and `taus88.hpp` (Boost Software License 1.0),
//! so that with a fixed seed a stroke's random choices are the ones Krita
//! would make in the same order.

/// `linear_feedback_shift_engine<uint32_t, 32, K, Q, S>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lfsr<const K: u32, const Q: u32, const S: u32> {
    value: u32,
}

impl<const K: u32, const Q: u32, const S: u32> Lfsr<K, Q, S> {
    const W: u32 = 32;

    fn seeded(s0: u32) -> Self {
        let mut value = s0;
        let floor = 1u32 << (Self::W - K);
        if value < floor {
            value += floor;
        }
        Self { value }
    }

    fn next(&mut self) -> u32 {
        let v = self.value;
        let b = ((v << Q) ^ v) >> (K - S);
        let mask = u32::MAX << (Self::W - K);
        self.value = ((v & mask) << S) ^ b;
        self.value
    }
}

/// boost's `taus88`: three Tausworthe components, xor-combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taus88 {
    a: Lfsr<31, 13, 12>,
    b: Lfsr<29, 2, 4>,
    c: Lfsr<28, 3, 17>,
}

impl Taus88 {
    /// `taus88(v)`: every component seeded with the same value.
    pub fn new(seed: u32) -> Self {
        Self {
            a: Lfsr::seeded(seed),
            b: Lfsr::seeded(seed),
            c: Lfsr::seeded(seed),
        }
    }

    /// `operator()`: `xor_combine` evaluates its first engine first.
    pub fn next_u32(&mut self) -> u32 {
        let ab = self.a.next() ^ self.b.next();
        ab ^ self.c.next()
    }
}

/// `KisRandomSource`.
#[derive(Clone, Debug)]
pub struct RandomSource {
    rng: Taus88,
}

impl RandomSource {
    pub fn new(seed: u32) -> Self {
        Self { rng: Taus88::new(seed) }
    }

    /// A stroke's source, seeded the way Krita seeds one (`qrand()`): from
    /// whatever is at hand, different every stroke.
    pub fn for_stroke() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        // qrand() is 0..=RAND_MAX, 15 bits with MSVC's CRT.
        Self::new((h.finish() & 0x7fff) as u32)
    }

    /// `generate()`.
    pub fn generate(&mut self) -> u32 {
        self.rng.next_u32()
    }

    /// `generate(min, max)`: `boost::uniform_smallint`, a plain modulo.
    pub fn generate_range(&mut self, min: i32, max: i32) -> i32 {
        let range = (max as i64 - min as i64) as u64;
        let val = self.rng.next_u32() as u64;
        if range >= u32::MAX as u64 {
            (val as i64 + min as i64) as i32
        } else {
            ((val % (range + 1)) as i64 + min as i64) as i32
        }
    }

    /// `generateNormalized()`: 0..=1.
    pub fn generate_normalized(&mut self) -> f64 {
        self.rng.next_u32() as f64 / u32::MAX as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first outputs of boost's `taus88` for seed 1, worked through by
    /// hand from the three shift registers' definitions: each component is
    /// lifted to its floor (2, 8, 16) by the seeding rule, then stepped.
    #[test]
    fn taus88_steps_like_boost() {
        let mut a = Lfsr::<31, 13, 12>::seeded(1);
        assert_eq!(a.value, 3, "1 < 2, so 1 + 2");
        // b = ((3 << 13) ^ 3) >> 19 = 0; mask keeps bit 1 up: (2 << 12) ^ 0.
        assert_eq!(a.next(), 2 << 12);
        let mut b = Lfsr::<29, 2, 4>::seeded(1);
        assert_eq!(b.value, 9);
        let mut c = Lfsr::<28, 3, 17>::seeded(1);
        assert_eq!(c.value, 17);
        let (vb, vc) = (b.next(), c.next());
        // b: ((9 << 2) ^ 9) >> 25 = 0; (9 & !7) << 4 = 8 << 4.
        assert_eq!(vb, 128);
        // c: ((17 << 3) ^ 17) >> 11 = 0; (17 & !15) << 17 = 16 << 17.
        assert_eq!(vc, 16 << 17);
        assert_eq!(Taus88::new(1).next_u32(), (2 << 12) ^ 128 ^ (16 << 17));
    }

    #[test]
    fn a_seed_repeats_and_ranges_hold() {
        let mut a = RandomSource::new(12345);
        let mut b = RandomSource::new(12345);
        for _ in 0..1000 {
            assert_eq!(a.generate(), b.generate());
        }
        let mut r = RandomSource::new(7);
        let mut seen = [0usize; 7];
        for _ in 0..7000 {
            let v = r.generate_range(0, 6);
            assert!((0..=6).contains(&v));
            seen[v as usize] += 1;
        }
        assert!(seen.iter().all(|&n| n > 800), "{seen:?}");
        let mean: f64 = (0..10_000).map(|_| r.generate_normalized()).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
