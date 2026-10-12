//! `gSaveData` (src/sys/save.c): the fields the ported screens read, as `Save_SetDefaults` writes them.

/// `SAVE_CHARA_COUNT`, `SAVE_STAGE_COUNT`, `SAVE_BGM_COUNT`.
const CHARA_COUNT: usize = 161;
const STAGE_COUNT: u32 = 35;
const BGM_COUNT: u32 = 20;

/// Characters `Save_SetDefaults` leaves locked.
const LOCKED_CHARAS: [usize; 29] = [
    7, 8, 9, 10, 0x18, 0x26, 0x36, 0x45, 0x46, 0x47, 0x48, 0x4B, 0x4C, 0x61, 0x6E, 0x78, 0x91,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F, 0xA0,
];
/// Stages `Save_SetDefaults` leaves locked.
const LOCKED_STAGES: [u32; 8] = [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x1F, 0x20];

/// `Save_ResetRules`: time, COM level, referee, two transformation switches and destructible stages.
pub const DEFAULT_RULES: [i32; 6] = [3, 2, 2, 0, 0, 0];

#[derive(Clone, Debug)]
pub struct SaveData {
    /// `charaBits`: bit `id % 64` of word `id / 64`.
    pub chara_bits: [u64; 3],
    pub stage_bits: u64,
    pub bgm_bits: u32,
    /// `rule[6]`.
    pub rules: [i32; 6],
    /// `rec[14].chara`: the saved custom characters, -1 for none.
    pub rec_chara: [i32; 14],
    /// `rec[14].item` / `.level`: their item sets and levels.
    pub rec_items: [[u16; 8]; 14],
    pub rec_level: [u16; 14],
    /// `custom[97]`: per character slot of the grid the three item sets (`item[3][8]`, 1-based ids, empty in a new
    /// save: items are equipped in the customisation menu) and the level.
    pub custom_items: Vec<[[u16; 8]; 3]>,
    pub custom_level: Vec<u16>,
}

impl Default for SaveData {
    fn default() -> Self {
        let mut chara_bits = [0; 3];
        for id in (0..CHARA_COUNT).filter(|id| !LOCKED_CHARAS.contains(id)) {
            chara_bits[id / 64] |= 1 << (id % 64);
        }
        let stage_bits = (0..STAGE_COUNT)
            .filter(|i| !LOCKED_STAGES.contains(i))
            .fold(0, |bits, i| bits | 1 << i);
        Self {
            chara_bits,
            stage_bits,
            bgm_bits: (1 << BGM_COUNT) - 1,
            rules: DEFAULT_RULES,
            rec_chara: [-1; 14],
            rec_items: [[0; 8]; 14],
            rec_level: [0; 14],
            custom_items: vec![[[0; 8]; 3]; 97],
            custom_level: vec![0; 97],
        }
    }
}

impl SaveData {
    /// `CHARA_UNLOCKED(id)`.
    pub fn chara_unlocked(&self, id: i32) -> bool {
        let id = id as usize;
        self.chara_bits
            .get(id / 64)
            .is_some_and(|word| word >> (id % 64) & 1 != 0)
    }

    pub fn stage_unlocked(&self, id: i32) -> bool {
        (0..64).contains(&id) && self.stage_bits >> id & 1 != 0
    }

    pub fn bgm_unlocked(&self, id: i32) -> bool {
        (0..32).contains(&id) && self.bgm_bits & 1 << id != 0
    }
}

#[cfg(test)]
mod tests {
    use super::SaveData;

    #[test]
    fn defaults_lock_the_listed_entries() {
        let save = SaveData::default();
        assert!(save.chara_unlocked(0) && !save.chara_unlocked(7) && !save.chara_unlocked(0xA0));
        assert!(!save.chara_unlocked(0xA1));
        assert!(save.stage_unlocked(0) && !save.stage_unlocked(0x11) && !save.stage_unlocked(0x23));
        assert!(save.bgm_unlocked(19) && !save.bgm_unlocked(20));
    }
}
