//! Byte scans a word at a time, for the masks the selection tools walk. Long
//! runs of one value are the usual thing in a mask, and stepping over them
//! eight bytes to a compare is most of what makes an 8K one cheap to read.

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGHS: u64 = 0x8080_8080_8080_8080;

#[inline]
fn word(c: &[u8]) -> u64 {
    u64::from_le_bytes(c.try_into().expect("eight bytes"))
}

/// How many bytes at the start of `s` are `v`.
#[inline]
pub fn run_of(s: &[u8], v: u8) -> usize {
    let pat = ONES * v as u64;
    let mut chunks = s.chunks_exact(8);
    let mut i = 0;
    for c in &mut chunks {
        let diff = word(c) ^ pat;
        if diff != 0 {
            // Little-endian: the first byte in memory is the lowest.
            return i + (diff.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    i + chunks.remainder().iter().take_while(|&&b| b == v).count()
}

/// How many bytes at the end of `s` are `v`.
#[inline]
pub fn run_of_back(s: &[u8], v: u8) -> usize {
    let pat = ONES * v as u64;
    let mut chunks = s.rchunks_exact(8);
    let mut i = 0;
    for c in &mut chunks {
        let diff = word(c) ^ pat;
        if diff != 0 {
            return i + (diff.leading_zeros() / 8) as usize;
        }
        i += 8;
    }
    i + chunks.remainder().iter().rev().take_while(|&&b| b == v).count()
}

/// Where the first `v` in `s` is.
#[inline]
pub fn find(s: &[u8], v: u8) -> Option<usize> {
    let pat = ONES * v as u64;
    let mut chunks = s.chunks_exact(8);
    let mut i = 0;
    for c in &mut chunks {
        // A zero byte where `v` is. The borrow in the subtraction can mark
        // bytes above a real zero too, never below one, so the lowest mark
        // is the first `v`.
        let x = word(c) ^ pat;
        let zero = x.wrapping_sub(ONES) & !x & HIGHS;
        if zero != 0 {
            return Some(i + (zero.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    chunks.remainder().iter().position(|&b| b == v).map(|k| i + k)
}

/// A pixel of a [`flood`] map that the flood may not enter.
pub const SHUT: u8 = 0;
/// One it may, not yet reached.
pub const OPEN: u8 = 1;
/// One it reached.
pub const TAKEN: u8 = 2;

/// Flood `map`, `w` wide, from `seed`: every [`OPEN`] pixel joined to it
/// side by side becomes [`TAKEN`]. Returns their bounds, inclusive. A
/// scanline flood whose runs are found a word at a time.
pub fn flood(map: &mut [u8], w: usize, seed: (i32, i32)) -> (i32, i32, i32, i32) {
    let h = map.len() / w;
    let (sx, sy) = (seed.0 as usize, seed.1 as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx, sy);
    let mut stack = vec![(sx, sy)];
    while let Some((x, y)) = stack.pop() {
        let row = &mut map[y * w..(y + 1) * w];
        if row[x] != OPEN {
            continue;
        }
        let l = x - run_of_back(&row[..x], OPEN);
        let r = x + run_of(&row[x..], OPEN);
        row[l..r].fill(TAKEN);
        x0 = x0.min(l);
        x1 = x1.max(r - 1);
        y0 = y0.min(y);
        y1 = y1.max(y);
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= h {
                continue;
            }
            // Each run still open beside this one seeds a span of its own.
            let next = &map[ny * w + l..ny * w + r];
            let mut i = 0;
            while let Some(k) = find(&next[i..], OPEN) {
                stack.push((l + i + k, ny));
                i += k;
                i += run_of(&next[i..], OPEN);
            }
        }
    }
    (x0 as i32, y0 as i32, x1 as i32, y1 as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every length and position up to a few words, against the plain loops.
    #[test]
    fn scans_match_the_plain_loops() {
        for len in 0..40usize {
            for at in 0..=len {
                for other in [0u8, 1, 2, 254, 255] {
                    for v in [0u8, 1, 2, 255] {
                        if other == v {
                            continue;
                        }
                        let mut s = vec![v; len];
                        if at < len {
                            s[at] = other;
                        }
                        let run = s.iter().take_while(|&&b| b == v).count();
                        assert_eq!(run_of(&s, v), run, "{s:?}");
                        let back = s.iter().rev().take_while(|&&b| b == v).count();
                        assert_eq!(run_of_back(&s, v), back, "{s:?}");
                        assert_eq!(find(&s, other), s.iter().position(|&b| b == other), "{s:?}");
                        // Near misses (one bit off) before the hit, and the
                        // bytes the subtraction's borrow can falsely mark
                        // after it, must not move it.
                        let mut t = vec![other.wrapping_add(1); len];
                        if at < len {
                            t[at] = other;
                            for b in t.iter_mut().skip(at + 1) {
                                *b = other ^ 1;
                            }
                        }
                        assert_eq!(find(&t, other), t.iter().position(|&b| b == other), "{t:?}");
                    }
                }
            }
        }
    }
}
