const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_B0DF;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7FFF_FFFF;

/// The shared generator behind `Rand_Range` (src/sys/rand.c).
pub struct GameRandom {
    state: [u32; N],
    index: usize,
}

impl GameRandom {
    fn from_seed(seed: u32) -> Self {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            let previous = state[i - 1];
            state[i] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        Self { state, index: N }
    }

    pub fn from_key(key: &[u32]) -> Self {
        let mut random = Self::from_seed(19_650_218);
        let state = &mut random.state;
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..N.max(key.len()) {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                state[0] = state[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                state[0] = state[N - 1];
                i = 1;
            }
        }
        state[0] = 0x8000_0000;
        random
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            // The original refill lacks the reference's second loop, so words 227..622 are never regenerated.
            for kk in 0..N - M {
                let y = (self.state[kk] & UPPER_MASK) | (self.state[kk + 1] & LOWER_MASK);
                self.state[kk] = self.state[kk + M] ^ (y >> 1) ^ mag01(y);
            }
            let y = (self.state[N - 1] & UPPER_MASK) | (self.state[0] & LOWER_MASK);
            self.state[N - 1] = self.state[M - 1] ^ (y >> 1) ^ mag01(y);
            self.index = 0;
        }
        let y = self.state[self.index];
        self.index += 1;
        temper(y)
    }

    pub fn range(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }
}

fn mag01(y: u32) -> u32 {
    if y & 1 != 0 { MATRIX_A } else { 0 }
}

fn temper(mut y: u32) -> u32 {
    y ^= y >> 11;
    y ^= (y << 7) & 0x9D2C_5680;
    y ^= (y << 15) & 0xEFC6_0000;
    y ^ (y >> 18)
}

/// newlib `rand()` as linked at 0x2A9C78, the generator behind `Rand_IntRange`.
pub struct LibcRandom(u64);

impl LibcRandom {
    pub fn new(seed: u32) -> Self {
        Self(u64::from(seed))
    }

    pub fn rand(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(0x5851_F42D_4C95_7F2D).wrapping_add(1);
        ((self.0 >> 32) & 0x7FFF_FFFF) as i32
    }

    pub fn int_range(&mut self, a: i32, b: i32) -> i32 {
        if a == b {
            return a;
        }
        let (low, high) = if b < a { (b, a) } else { (a, b) };
        low + self.rand() % (high - low + 1)
    }
}

/// The VU0 R register (`Rand_Float01` / `Rand_FloatRange`, src/sys/rand_util.c): the PS2's 23-bit shift-register
/// generator, kept as the float pattern `0x3F800000 | mantissa` it hands out. `VRNEXT` shifts the mantissa left by
/// one and feeds back bit 4 xor bit 22 (the feedback taps are PCSX2's, `AdvanceLFSR`, not the decompilation's);
/// `VRINIT` takes a float's mantissa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vu0Random(u32);

/// `Vu0_Init`: `Rand_SeedFloat(0.1234141f)` at boot.
pub const VU0_BOOT_SEED: f32 = 0.123_414_1;

impl Default for Vu0Random {
    fn default() -> Self {
        Self::seeded(VU0_BOOT_SEED)
    }
}

impl Vu0Random {
    /// `VRINIT R, seed`.
    pub fn seeded(seed: f32) -> Self {
        Self(0x3F80_0000 | (seed.to_bits() & 0x007F_FFFF))
    }

    /// `VRNEXT`.
    fn next(&mut self) -> u32 {
        let r = self.0;
        let feed = ((r >> 4) ^ (r >> 22)) & 1;
        self.0 = ((r << 1) ^ feed) & 0x007F_FFFF | 0x3F80_0000;
        self.0
    }

    /// `Rand_Float01`: seven steps, `VRINIT` from the value just made (which leaves R as it is), seven more, minus
    /// 1.0.
    pub fn float01(&mut self) -> f32 {
        for _ in 0..7 {
            self.next();
        }
        let v = self.0;
        *self = Self(0x3F80_0000 | (v & 0x007F_FFFF));
        for _ in 0..7 {
            self.next();
        }
        f32::from_bits(self.0) - 1.0
    }

    /// `Rand_FloatRange(a, b)`: b itself when a == b, else low + Rand_Float01() * (high - low).
    pub fn float_range(&mut self, a: f32, b: f32) -> f32 {
        if b == a {
            return b;
        }
        let (lo, hi) = if b < a { (b, a) } else { (a, b) };
        lo + self.float01() * (hi - lo)
    }
}

/// `Rand_Init`: libc rand is seeded from the timer, then four of its values seed the shared generator.
pub fn init_from_timer(timer: u32) -> (GameRandom, LibcRandom) {
    let mut libc = LibcRandom::new(timer);
    let key: [u32; 4] = std::array::from_fn(|_| libc.rand() as u32);
    (GameRandom::from_key(&key), libc)
}

#[cfg(test)]
mod tests {
    use super::{GameRandom, LibcRandom, Vu0Random, temper};

    #[test]
    fn vu0_random_stays_in_its_float_range() {
        let mut r = Vu0Random::default();
        let first = r.float01();
        for _ in 0..1000 {
            let v = r.float01();
            assert!((0.0..1.0).contains(&v));
        }
        assert_ne!(first, r.float01());
        assert_eq!(r.float_range(3.0, 3.0), 3.0);
        let v = r.float_range(5.0, 2.0);
        assert!((2.0..5.0).contains(&v));
    }

    #[test]
    fn libc_rand_matches_the_linked_lcg() {
        let mut random = LibcRandom::new(1);
        assert_eq!(random.rand(), 0x5851_F42D);
    }

    #[test]
    fn first_refill_words_match_reference_mt19937() {
        let mut random = GameRandom::from_key(&[0x123, 0x234, 0x345, 0x456]);
        let first: Vec<u32> = (0..5).map(|_| random.next_u32()).collect();
        assert_eq!(
            first,
            [
                1_067_595_299,
                955_945_823,
                477_289_528,
                4_107_218_783,
                4_228_976_476
            ]
        );
    }

    #[test]
    fn refill_leaves_word_227_as_seeded() {
        let mut random = GameRandom::from_key(&[0x123, 0x234, 0x345, 0x456]);
        let seeded = random.state[227];
        for _ in 0..227 {
            random.next_u32();
        }
        assert_eq!(random.next_u32(), temper(seeded));
    }
}
