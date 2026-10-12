//! Custom character passwords: the codecs of the main executable (`ChrPass_*`, src/sys/password_chara.c, this
//! game's 34-character passwords; `OldPass_DecodeChar`, src/sys/password_old.c, the previous game's 32-character
//! ones) and the password screen's checks (`PassChk_*`, src/menu/password_check.c).
//!
//! Both codecs pack fields into a 32-byte bit buffer (least significant bit first), add bit-count checksums and a
//! parity block, XOR the buffer from byte 4 on with the low bytes of an MT19937 state seeded with
//! `init_by_array({seed, seed * 2, seed * 3, seed * 4})` (read from index 1 and tempered, never twisted), and write
//! it as text with 6 bits a character. Bits 0..31, the seed, stay in clear; a new password's seed is two libc
//! `rand()` draws.

use crate::rng::LibcRandom;

const MT_N: usize = 624;
const BUF_SIZE: usize = 32;

/// This game's characters, in value order.
const CHARSET: &[u8; 64] = b"QRST2VWX1JKLMN5P0BCD3FGHYZ4bcd9fwxyz!#$%6pqrst8vgh7jklmn&@-+*()?";
/// The previous game's.
const OLD_CHARSET: &[u8; 64] = b"QRSTUVWXIJKLMNOPABCDEFGHYZabcdefwxyz!#$%opqrstuvghijklmn&@-+*()?";

/// The key stream: an MT19937 state that is only read (`*_MtSeedByArray`, `*_MtNext`).
struct KeyStream {
    state: Vec<u32>,
    index: usize,
}

impl KeyStream {
    /// `init_genrand`.
    fn seed(seed: u32) -> Self {
        let mut state = vec![0u32; MT_N];
        state[0] = seed;
        for i in 1..MT_N {
            state[i] = 1_812_433_253u32
                .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        Self { state, index: 1 }
    }

    /// `init_by_array({seed, seed * 2, seed * 3, seed * 4})`.
    fn for_password(seed: u32) -> Self {
        let key = [
            seed,
            seed.wrapping_mul(2),
            seed.wrapping_mul(3),
            seed.wrapping_mul(4),
        ];
        let mut k = Self::seed(19_650_218);
        let s = &mut k.state;
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..MT_N.max(key.len()) {
            s[i] = (s[i] ^ (s[i - 1] ^ (s[i - 1] >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= MT_N {
                s[0] = s[MT_N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..MT_N - 1 {
            s[i] = (s[i] ^ (s[i - 1] ^ (s[i - 1] >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= MT_N {
                s[0] = s[MT_N - 1];
                i = 1;
            }
        }
        k.index = 1;
        k
    }

    /// The next word, tempered (there is no refill).
    fn next(&mut self) -> u32 {
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^ (y >> 18)
    }
}

/// The bit buffer.
struct Bits([u8; BUF_SIZE]);

impl Bits {
    fn put(&mut self, value: u32, pos: usize, count: usize) {
        for i in 0..count {
            let p = i + pos;
            if value >> i & 1 != 0 {
                self.0[p / 8] |= 1 << (p % 8);
            } else {
                self.0[p / 8] &= !(1 << (p % 8));
            }
        }
    }

    fn bit(&self, pos: usize) -> u32 {
        u32::from(self.0[pos / 8] >> (pos % 8) & 1)
    }

    fn get(&self, pos: usize, count: usize) -> u32 {
        (0..count).fold(0, |v, i| v | self.bit(i + pos) << i)
    }

    fn count(&self, bits: usize) -> u32 {
        (0..bits).map(|i| self.bit(i)).sum()
    }

    /// `*_TextToBits`: 6 bits a character; characters outside the set are skipped.
    fn from_text(text: &[u8], charset: &[u8; 64]) -> Self {
        let mut b = Bits([0; BUF_SIZE]);
        let mut pos = 0;
        for c in text {
            if let Some(v) = charset.iter().position(|x| x == c) {
                b.put(v as u32, pos, 6);
                pos += 6;
            }
        }
        b
    }

    /// `*_BitsToText`.
    fn to_text(&self, bits: usize, charset: &[u8; 64]) -> String {
        (0..bits / 6)
            .map(|n| charset[self.get(n * 6, 6) as usize] as char)
            .collect()
    }

    /// The key stream over bytes 4..`end`.
    fn crypt(&mut self, seed: u32, end: usize) {
        let mut k = KeyStream::for_password(seed);
        for i in 4..end {
            self.0[i] ^= k.next() as u8;
        }
    }

    /// The parity block: bytes 16..24 are bytes 0..8 XOR 8..16.
    fn parity_ok(&self) -> bool {
        (0..8).all(|i| self.0[i + 16] == self.0[i] ^ self.0[i + 8])
    }
}

/// `ChrPassData`: what this game's password carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChrPass {
    pub chara: i32,
    /// 1-based item ids, 0 = empty.
    pub items: [i32; 8],
    /// Item slots on top of the character's own.
    pub extra_slots: i32,
    pub unk28: i32,
}

impl ChrPass {
    /// `ChrPass_Decode`: a 34-character password whose bit counts and parity check.
    pub fn decode(text: &str) -> Option<Self> {
        if text.len() != 34 {
            return None;
        }
        let mut b = Bits::from_text(text.as_bytes(), CHARSET);
        b.crypt(b.get(0, 32), 26);
        if b.count(0x77) != b.get(0x77, 8) {
            return None;
        }
        let sum = b.count(0xC4);
        // The second, 4-bit copy of the sum lies past the 34 characters: the original compares the low half of
        // the byte just checked.
        if sum != b.get(0xC4, 8) || sum & 0xF != b.get(0xC4, 4) || !b.parity_ok() {
            return None;
        }
        Some(Self {
            chara: b.get(0x20, 8) as i32,
            items: std::array::from_fn(|n| b.get(0x28 + n * 9, 9) as i32),
            extra_slots: b.get(0x70, 4) as i32,
            unk28: b.get(0x74, 1) as i32,
        })
    }

    /// `ChrPass_Encode`: the 34-character text, with a seed of two `rand()` draws.
    pub fn encode(&self, libc: &mut LibcRandom) -> String {
        let hi = libc.rand() as u32;
        let lo = libc.rand() as u32;
        let seed = (hi << 16) | (lo & 0xFFFF);
        let mut b = Bits([0; BUF_SIZE]);
        b.put(seed, 0, 32);
        b.put(self.chara as u32, 0x20, 8);
        for (n, item) in self.items.iter().enumerate() {
            b.put(*item as u32, 0x28 + n * 9, 9);
        }
        b.put(self.extra_slots as u32, 0x70, 4);
        b.put(self.unk28 as u32, 0x74, 1);
        let sum = b.count(0x80);
        b.put(sum, 0x77, 8);
        for i in 0..8 {
            b.0[i + 16] = b.0[i] ^ b.0[i + 8];
        }
        let sum = b.count(0xC4);
        b.put(sum, 0xC4, 8);
        b.put(sum, 0xCC, 4);
        b.crypt(seed, 26);
        b.to_text(0xD0, CHARSET)
    }
}

/// `OldPassChar`: what the previous game's password carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OldPass {
    /// The previous game's character id.
    pub chara: i32,
    /// The previous game's item ids, 1-based.
    pub items: [i32; 7],
    pub unk20: i32,
    pub unk24: i32,
    pub unk28: i32,
    pub unk2c: i32,
}

impl OldPass {
    /// `OldPass_DecodeChar`: a 32-character password whose bit count and parity check.
    pub fn decode(text: &str) -> Option<Self> {
        if text.len() != 32 {
            return None;
        }
        let mut b = Bits::from_text(text.as_bytes(), OLD_CHARSET);
        b.crypt(b.get(0, 32), 24);
        if b.count(0x77) != b.get(0x77, 8) || !b.parity_ok() {
            return None;
        }
        Some(Self {
            chara: b.get(0x20, 8) as i32,
            items: std::array::from_fn(|n| b.get(0x28 + n * 10, 10) as i32),
            unk20: b.get(0x6E, 3) as i32,
            unk24: b.get(0x71, 1) as i32,
            unk28: b.get(0x72, 4) as i32,
            unk2c: b.get(0x76, 1) as i32,
        })
    }

    /// `OldPass_EncodeChar` (no caller in the game; the tests use it).
    #[cfg(test)]
    pub(crate) fn encode(&self, seed: u32) -> String {
        let mut b = Bits([0; BUF_SIZE]);
        b.put(seed, 0, 32);
        b.put(self.chara as u32, 0x20, 8);
        for (n, item) in self.items.iter().enumerate() {
            b.put(*item as u32, 0x28 + n * 10, 10);
        }
        b.put(self.unk20 as u32, 0x6E, 3);
        b.put(self.unk24 as u32, 0x71, 1);
        b.put(self.unk28 as u32, 0x72, 4);
        b.put(self.unk2c as u32, 0x76, 1);
        let sum = b.count(0x80);
        b.put(sum, 0x77, 8);
        for i in 0..8 {
            b.0[i + 16] = b.0[i] ^ b.0[i + 8];
        }
        b.crypt(seed, 24);
        b.to_text(0xC0, OLD_CHARSET)
    }
}

/// `PASSCHK_OLD_ITEM_NUM`, `PASSCHK_NONE`.
const OLD_ITEM_COUNT: u32 = 0x109;
const NONE: i32 = 999;

/// `PassChk`: the password pack's tables (section 19 of the password screen's pack) and the character and item
/// tables of common file 4.
#[derive(Clone, Debug, Default)]
pub struct PassChk {
    /// Section 1: the previous game's character id → this game's.
    old_chara: Vec<i32>,
    /// Section 2: per item of the previous game, (level, defence, attack).
    old_item: Vec<[u8; 3]>,
    /// Section 3: [6 level classes][3 types][7] item ids, 999 = none.
    preset: Vec<i32>,
    /// Section 5: per character, (mask of the item groups it may take, whether a password may give it).
    chara: Vec<(u16, u16)>,
    /// Section 4: per item, (flags, group).
    item: Vec<(u8, u8)>,
    /// Common file 4, section 1: per character, (flags, slots at level 0).
    chr_tbl: Vec<(u16, u16)>,
    /// Section 2: per item, (type, kind, slots, flags).
    item_tbl: Vec<(u8, u8, u8, u32)>,
}

fn section(pack: &[u8], n: usize) -> &[u8] {
    section_bytes(pack, n).unwrap_or_default()
}

/// `MPACK_AT(pack, n)` up to the next section: header word 0 counts the sections, so the last one runs to the
/// end of the pack.
pub fn section_bytes(pack: &[u8], n: usize) -> std::io::Result<&[u8]> {
    let start = bt3_data::pack_section(pack, n)?;
    let word = |i: usize| {
        pack.get(i * 4..i * 4 + 4)
            .map(|b| (u32::from_le_bytes(b.try_into().unwrap()) & !3) as usize)
    };
    let from = pack.len() - start.len();
    let end = match (word(0), word(n + 1)) {
        (Some(count), Some(end)) if n < count && end >= from && end <= pack.len() => end,
        _ => pack.len(),
    };
    Ok(&pack[from..end])
}

impl PassChk {
    /// `PassChk_Init` over the password pack, with common file 4.
    pub fn new(pack: &[u8], chara_table: &[u8]) -> Self {
        let words = |b: &[u8]| -> Vec<i32> {
            b.chunks_exact(4)
                .map(|w| i32::from_le_bytes(w.try_into().unwrap()))
                .collect()
        };
        let u16_at = |b: &[u8], at: usize| {
            b.get(at..at + 2)
                .map_or(0, |v| u16::from_le_bytes([v[0], v[1]]))
        };
        let table = |n: usize| section(chara_table, n);
        let (charas, items) = (table(1), table(2));
        Self {
            old_chara: words(section(pack, 1)),
            old_item: section(pack, 2)
                .chunks_exact(4)
                .map(|e| [e[1], e[2], e[3]])
                .collect(),
            preset: words(section(pack, 3))
                .into_iter()
                .take(6 * 3 * 7)
                .collect(),
            chara: section(pack, 5)
                .chunks_exact(4)
                .map(|e| {
                    (
                        u16::from_le_bytes([e[0], e[1]]),
                        u16::from_le_bytes([e[2], e[3]]),
                    )
                })
                .collect(),
            item: section(pack, 4)
                .chunks_exact(2)
                .map(|e| (e[0], e[1]))
                .collect(),
            chr_tbl: (0..0xA1)
                .map(|c| (u16_at(charas, c * 0x3C + 8), u16_at(charas, c * 0x3C + 0xE)))
                .collect(),
            item_tbl: (0..0x15E)
                .map(|i| {
                    let e = i * 0x28;
                    let b = |k: usize| items.get(e + k).copied().unwrap_or(0);
                    let flags = items
                        .get(e + 0x14..e + 0x18)
                        .map_or(0, |w| u32::from_le_bytes(w.try_into().unwrap()));
                    (b(0), b(1), b(3), flags)
                })
                .collect(),
        }
    }

    /// The character's flags in common file 4 (bit 0: the attribute icon's other picture).
    pub fn chara_flags(&self, chara: i32) -> u16 {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.chr_tbl.get(c))
            .map_or(0, |c| c.0)
    }

    fn base_slots(&self, chara: usize) -> i32 {
        self.chr_tbl.get(chara).map_or(0, |c| i32::from(c.1))
    }

    /// `PassChk_ConvertOld`: a previous game's character becomes this game's with the seven preset items of the
    /// class of its items' summed levels and the type their attack and defence give.
    pub fn convert_old(&self, old: &OldPass) -> ChrPass {
        let chara = self.old_chara.get(old.chara as usize).copied().unwrap_or(0);
        let (mut level, mut defense, mut attack) = (0u32, 0u32, 0u32);
        for &item in &old.items {
            let i = (item - 1) as u32;
            if i < OLD_ITEM_COUNT
                && let Some(e) = self.old_item.get(i as usize)
            {
                level += u32::from(e[0]);
                defense += u32::from(e[1]);
                attack += u32::from(e[2]);
            }
        }
        let class = match level {
            1..=7 => 1,
            8..=14 => 2,
            15..=21 => 3,
            22..=28 => 4,
            29..=35 => 5,
            _ => 0,
        };
        let kind = if attack < defense {
            0
        } else if defense < attack {
            1
        } else {
            2
        };
        let mut out = ChrPass {
            chara,
            extra_slots: 7 - self.base_slots(chara.max(0) as usize),
            ..ChrPass::default()
        };
        for i in 0..7 {
            let id = self
                .preset
                .get(class * 21 + kind * 7 + i)
                .copied()
                .unwrap_or(NONE);
            out.items[i] = if id != NONE { id + 1 } else { 0 };
        }
        out
    }

    /// `PassChk_IsOldValid`: a known character, allowed items only.
    pub fn is_old_valid(&self, old: &OldPass) -> bool {
        let mut chara = old.chara as u32;
        if chara >= 0xCA || chara == 0x82 || chara == 0x84 {
            return false;
        }
        if chara == 0xC8 {
            chara = 0x82;
        }
        let Some(&(mask, valid)) = self.chara.get(chara as usize) else {
            return false;
        };
        if valid == 0 || old.unk20 as u32 >= 3 {
            return false;
        }
        for &item in &old.items {
            if item == 0 {
                continue;
            }
            let i = (item - 1) as u32;
            if i >= 0x15F {
                return false;
            }
            let (flags, group) = self.item.get(i as usize).copied().unwrap_or((1, 0));
            if flags & 1 != 0 || flags & 0x20 == 0 {
                return false;
            }
            if i == 0xA2 && matches!(chara, 0x24..=0x26 | 0x53 | 0x76 | 0x77) {
                return false;
            }
            if group != 0 && mask & (1 << (group - 1)) == 0 {
                return false;
            }
        }
        true
    }

    /// `PassChk_IsValid`: a character below 161, items below 350 without a gap, none twice, no two of one type
    /// and kind, a type 2 item in the eighth place only, none the character's class refuses, and a slot cost
    /// within the character's slots and the extra ones (an item of type 3 sets the extra slots to "up to
    /// seven" first, written back).
    pub fn is_valid(&self, pass: &mut ChrPass) -> bool {
        let chara = pass.chara as u32;
        if chara > 0xA0 {
            return false;
        }
        let base = self.base_slots(chara as usize);
        let mut extra = pass.extra_slots;
        if base + extra > 7 {
            return false;
        }
        let entry = |i: u32| {
            self.item_tbl
                .get(i as usize)
                .copied()
                .unwrap_or((0, 0, 0, 0))
        };
        let (mut fill, mut cost) = (false, 0);
        let class = self.chr_tbl.get(chara as usize).map_or(0, |c| c.0);
        for i in 0..8 {
            let id = pass.items[i];
            if id == 0 {
                if i < 6 && pass.items[i + 1] != 0 {
                    return false;
                }
                continue;
            }
            let item = (id - 1) as u32;
            let (kind_type, kind, slots, flags) = entry(item);
            if kind_type == 3 {
                fill = true;
            }
            if item >= 0x15E {
                return false;
            }
            for j in 0..7 {
                if j != i && item == (pass.items[j] - 1) as u32 {
                    return false;
                }
            }
            for j in i + 1..7 {
                let other = (pass.items[j] - 1) as u32;
                if other < 0x15E {
                    let o = entry(other);
                    if kind_type == o.0 && kind == o.1 {
                        return false;
                    }
                }
            }
            if (i == 7) != (kind_type == 2) {
                return false;
            }
            if flags & 5 != 0 {
                return false;
            }
            if flags & 0x18 != 0x18
                && ((class & 1 != 0 && flags & 0x10 != 0) || (class & 2 != 0 && flags & 8 != 0))
            {
                return false;
            }
            cost += i32::from(slots);
        }
        if fill {
            extra = 7 - base;
            pass.extra_slots = extra;
        }
        extra + base >= cost
    }
}

/// `ITEMTBL_TYPE_SLOTS`: an item whose slots count towards the first total of `ItemSet_GetBonus`.
const ITEM_TYPE_SLOTS: u8 = 1;

/// One 0x28-byte entry of an item parameter table (`zitem_parameter`, the screens' own copy of common file 4's
/// section 2): what the menus read of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemParam {
    pub info: bt3_data::chara_data::ItemInfo,
    /// The four stat changes (+0xC).
    pub stat: [i16; 4],
}

/// The entries of a raw item parameter table.
pub fn item_params(raw: &[u8]) -> Vec<ItemParam> {
    raw.chunks_exact(0x28)
        .map(|e| ItemParam {
            info: bt3_data::chara_data::ItemInfo {
                kind: e[0],
                group: e[1],
                picture: e[2],
                slots: e[3],
                flags: i32::from_le_bytes(e[0x14..0x18].try_into().unwrap()),
            },
            stat: std::array::from_fn(|k| i16::from_le_bytes([e[0xC + k * 2], e[0xD + k * 2]])),
        })
        .collect()
}

/// `ItemSet_GetBonus`: the slots the slot items add (at most 7) and the four stat changes, each summed, kept
/// within -20..20 and divided by 5 (so -3..3 marks on the status page).
pub fn item_bonus(items: &[u16; 8], table: &[ItemParam]) -> [i32; 5] {
    let mut out = [0i32; 5];
    for &id in items.iter().filter(|&&id| id != 0) {
        let Some(e) = table.get(usize::from(id) - 1) else {
            continue;
        };
        if e.info.kind == ITEM_TYPE_SLOTS {
            out[0] += i32::from(e.info.slots);
        }
        for k in 0..4 {
            out[1 + k] += i32::from(e.stat[k]);
        }
    }
    if out[0] >= 8 {
        out[0] = 7;
    }
    for v in &mut out[1..] {
        *v = (*v).clamp(-20, 20) / 5;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_games_passwords_round_trip_and_catch_a_typo() {
        let mut libc = LibcRandom::new(7);
        let p = ChrPass {
            chara: 0x2A,
            items: [5, 0x101, 9, 0, 0, 0, 0, 0x88],
            extra_slots: 3,
            unk28: 1,
        };
        let text = p.encode(&mut libc);
        assert_eq!(text.len(), 34);
        assert!(text.bytes().all(|c| CHARSET.contains(&c)));
        assert_eq!(ChrPass::decode(&text), Some(p));
        // Another seed, another text, the same content.
        let again = p.encode(&mut libc);
        assert_ne!(again, text);
        assert_eq!(ChrPass::decode(&again), Some(p));
        let mut typo = text.into_bytes();
        typo[20] = if typo[20] == b'Q' { b'R' } else { b'Q' };
        assert_eq!(ChrPass::decode(&String::from_utf8(typo).unwrap()), None);
        assert_eq!(ChrPass::decode("QRST"), None);
    }

    #[test]
    fn the_previous_games_passwords_decode() {
        let p = OldPass {
            chara: 0x31,
            items: [1, 2, 0x100, 0, 0, 0, 0],
            unk20: 2,
            unk24: 1,
            unk28: 5,
            unk2c: 0,
        };
        let text = p.encode(0x1234_5678);
        assert_eq!(text.len(), 32);
        assert_eq!(OldPass::decode(&text), Some(p));
        // This game's alphabet differs: its digits are not in the old set.
        assert_eq!(ChrPass::decode(&text), None);
    }

    #[test]
    fn item_bonuses_are_summed_clamped_and_fifths() {
        let mut table = vec![ItemParam::default(); 4];
        table[0].info.kind = ITEM_TYPE_SLOTS;
        table[0].info.slots = 5;
        table[1].info.kind = ITEM_TYPE_SLOTS;
        table[1].info.slots = 4;
        table[2].stat = [15, -9, 30, 0];
        table[3].stat = [10, -4, 0, 4];
        let b = item_bonus(&[1, 2, 3, 4, 0, 0, 0, 0], &table);
        // Slots 9 -> 7; 25 -> 20 / 5; -13 / 5 = -2 (towards zero); 30 -> 20 / 5; 4 / 5 = 0.
        assert_eq!(b, [7, 4, -2, 4, 0]);
    }

    #[test]
    fn the_key_stream_is_mt19937s() {
        // init_genrand(5489)'s first output (index 0) is 3499211612 after a twist; here the state is not twisted,
        // so check the seeding against the reference's first state words instead.
        let k = KeyStream::seed(5489);
        assert_eq!(k.state[1], 1_301_868_182);
        assert_eq!(k.state[623], 79_981_964);
    }
}
