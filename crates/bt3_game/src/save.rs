//! `gSaveData` (src/sys/save.c): the fields the ported screens read, as `Save_SetDefaults` writes them, over the
//! whole 0x4000-byte block the memory card saves (`image`, laid out as include/sys/save.h has it).

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

/// `SAVE_SIZE`.
pub const SIZE: usize = 0x4000;
/// `ITEM_INFO_INITIAL`: an item owned in a new save.
const ITEM_INFO_INITIAL: i32 = 2;
/// `SAVE_ITEM_COUNT`.
const ITEM_COUNT: usize = 350;
const CUSTOM_COUNT: usize = 97;
const REC_COUNT: usize = 14;

// Offsets in the block.
const OFS_UNLOCK: usize = 0x8;
const OFS_LEVEL: usize = 0xC;
const OFS_SLOT: usize = 0x10;
const OFS_MISSION_PAGES: usize = 0x20C;
const OFS_RANK: usize = 0x77C;
const OFS_CHARA_BITS: usize = 0xC10;
const OFS_STAGE_BITS: usize = 0xC28;
const OFS_BGM_BITS: usize = 0xC30;
const OFS_RULE: usize = 0xC34;
const OFS_FLAGS: usize = 0x1608;
const OFS_KEY: usize = 0x160C;
const OFS_KEY_EDIT: usize = 0x164C;
const OFS_CAM_DIST_MODE: usize = 0x1694;
const OFS_CAM_SHAKE_OFF: usize = 0x1698;
const OFS_SCREEN_X: usize = 0x169C;
const OFS_SCREEN_Y: usize = 0x16A0;
const OFS_SOUND_MODE: usize = 0x16A4;
const OFS_BGM_VOLUME: usize = 0x16A8;
const OFS_SE_VOLUME: usize = 0x16AC;
const OFS_CUSTOM: usize = 0x1808;
const OFS_REC: usize = 0x2D40;
const OFS_ITEM: usize = 0x2EC8;
const OFS_MONEY: usize = 0x3028;
/// `trainClear[3]` (src/menu/training.h `MSaveTrain`): one word of "lesson cleared" bits per class.
const OFS_TRAIN_CLEAR: usize = 0xE0C;
/// `tourFlags`, `tourHour`, `tourLevel`: Dragon World Tour's invitations, clock and the open tournament's level.
const OFS_TOUR_FLAGS: usize = 0xA08;
const OFS_TOUR_HOUR: usize = 0xA0C;
const OFS_TOUR_LEVEL: usize = 0xA10;
const OFS_SHOP_FLAGS: usize = 0x1008;
const OFS_STOCK_LEVEL: usize = 0x100C;
/// `SAVE_MONEY_MAX`.
const MONEY_MAX: i32 = 9_999_999;
/// `SAVE_FLAG_DEFAULT` and `SAVE_VOLUME_DEFAULT`.
const FLAG_DEFAULT: i32 = 7;
const VOLUME_DEFAULT: i32 = 9;
/// Each pad's default key assignment.
const DEFAULT_KEYS: [i32; 8] = [2, 1, 0, 3, 4, 5, 6, 7];

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
    /// The whole block, with what the typed fields do not cover.
    pub image: Vec<u8>,
}

impl Default for SaveData {
    /// `Save_SetDefaults` without the item table: no item owned.
    fn default() -> Self {
        Self::new(&[])
    }
}

fn put(b: &mut [u8], at: usize, v: i32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

impl SaveData {
    /// `Save_SetDefaults`: a new save; the items flagged `ITEM_INFO_INITIAL` in the item table are owned (and new).
    pub fn new(items: &[bt3_data::chara_data::ItemInfo]) -> Self {
        let mut chara_bits = [0; 3];
        for id in (0..CHARA_COUNT).filter(|id| !LOCKED_CHARAS.contains(id)) {
            chara_bits[id / 64] |= 1 << (id % 64);
        }
        let stage_bits = (0..STAGE_COUNT)
            .filter(|i| !LOCKED_STAGES.contains(i))
            .fold(0, |bits, i| bits | 1 << i);
        let mut b = vec![0u8; SIZE];
        put(&mut b, OFS_SLOT, 3);
        put(&mut b, OFS_SLOT + 4, 1);
        put(&mut b, OFS_SLOT + 0xC, 1);
        put(&mut b, OFS_LEVEL, 1);
        for (i, item) in items.iter().take(ITEM_COUNT).enumerate() {
            if item.flags & ITEM_INFO_INITIAL != 0 {
                b[OFS_ITEM + i] |= 3;
            }
        }
        put(&mut b, OFS_MISSION_PAGES, 5);
        put(&mut b, OFS_RANK, 99);
        for pad in 0..2 {
            for (k, v) in DEFAULT_KEYS.iter().enumerate() {
                put(&mut b, OFS_KEY + pad * 0x20 + k * 4, *v);
                put(&mut b, OFS_KEY_EDIT + pad * 0x20 + k * 4, *v);
            }
        }
        put(&mut b, OFS_FLAGS, FLAG_DEFAULT);
        put(&mut b, OFS_BGM_VOLUME, VOLUME_DEFAULT);
        put(&mut b, OFS_SE_VOLUME, VOLUME_DEFAULT);
        let mut save = Self {
            chara_bits,
            stage_bits,
            bgm_bits: (1 << BGM_COUNT) - 1,
            rules: DEFAULT_RULES,
            rec_chara: [-1; REC_COUNT],
            rec_items: [[0; 8]; REC_COUNT],
            rec_level: [0; REC_COUNT],
            custom_items: vec![[[0; 8]; 3]; CUSTOM_COUNT],
            custom_level: vec![0; CUSTOM_COUNT],
            image: b,
        };
        save.image = save.to_image();
        save
    }

    fn word(&self, at: usize) -> i32 {
        self.image
            .get(at..at + 4)
            .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
    }

    fn set_word(&mut self, at: usize, v: i32) {
        if let Some(b) = self.image.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }

    /// `unlockFlags`: the dragon balls (bits 0..6) and the history menu's guide events (0x80, 0x100, 0x200).
    pub fn unlock_flags(&self) -> i32 {
        self.word(OFS_UNLOCK)
    }

    pub fn set_unlock_flags(&mut self, v: i32) {
        self.set_word(OFS_UNLOCK, v);
    }

    /// `level`: the history menu's difficulty, 0..2.
    pub fn level(&self) -> i32 {
        self.word(OFS_LEVEL)
    }

    pub fn set_level(&mut self, v: i32) {
        self.set_word(OFS_LEVEL, v);
    }

    /// `slot[n].flags` of a history sub menu (`MSLOT_*`); read as the original reads it past the nine slots.
    pub fn slot_flags(&self, n: usize) -> i32 {
        self.word(OFS_SLOT + n * 0x10)
    }

    pub fn set_slot_flags(&mut self, n: usize, v: i32) {
        self.set_word(OFS_SLOT + n * 0x10, v);
    }

    /// `slot[n].val[k]`: 0 the listed episodes, 1 the cleared ones, 2 the new ones (bit per episode).
    pub fn slot_val(&self, n: usize, k: usize) -> i32 {
        self.word(OFS_SLOT + n * 0x10 + 4 + k * 4)
    }

    /// `item[n]`: bit 0 owned, bit 1 new.
    pub fn item(&self, n: usize) -> u8 {
        self.image.get(OFS_ITEM + n).copied().unwrap_or(0)
    }

    /// `Save_AddItem`: an item not owned yet becomes owned and new.
    pub fn add_item(&mut self, n: usize) {
        if let Some(b) = self.image.get_mut(OFS_ITEM + n)
            && *b & 1 == 0
        {
            *b |= 3;
        }
    }

    /// `shopFlags`: bit 0 the stock has just grown (the shop's greeting says so), bit 1 the last raise of the
    /// stock level was made.
    pub fn shop_flags(&self) -> i32 {
        self.word(OFS_SHOP_FLAGS)
    }

    pub fn set_shop_flags(&mut self, v: i32) {
        self.set_word(OFS_SHOP_FLAGS, v);
    }

    /// `stockLevel`: what the shop sells (items of this level and below).
    pub fn stock_level(&self) -> i32 {
        self.word(OFS_STOCK_LEVEL)
    }

    pub fn set_stock_level(&mut self, v: i32) {
        self.set_word(OFS_STOCK_LEVEL, v);
    }

    /// `tourFlags`: bit n the open tournament, 0x20 the mode was visited, 0x40 the explanation was heard.
    pub fn tour_flags(&self) -> i32 {
        self.word(OFS_TOUR_FLAGS)
    }

    pub fn set_tour_flags(&mut self, v: i32) {
        self.set_word(OFS_TOUR_FLAGS, v);
    }

    /// `tourHour`: the mode's clock, 0..23.
    pub fn tour_hour(&self) -> i32 {
        self.word(OFS_TOUR_HOUR)
    }

    pub fn set_tour_hour(&mut self, v: i32) {
        self.set_word(OFS_TOUR_HOUR, v);
    }

    /// `tourLevel`: the open tournament's level, 0..2.
    pub fn tour_level(&self) -> i32 {
        self.word(OFS_TOUR_LEVEL)
    }

    pub fn set_tour_level(&mut self, v: i32) {
        self.set_word(OFS_TOUR_LEVEL, v);
    }

    /// `custom[cell].exp`: the experience paid into a customised character.
    pub fn custom_exp(&self, cell: usize) -> i32 {
        self.word(OFS_CUSTOM + cell * 0x38 + 0x30)
    }

    pub fn set_custom_exp(&mut self, cell: usize, v: i32) {
        self.set_word(OFS_CUSTOM + cell * 0x38 + 0x30, v);
    }

    /// `money` set as it is (the customising screen pays its points without the clamp of `Save_AddMoney`).
    pub fn set_money(&mut self, v: i32) {
        self.set_word(OFS_MONEY, v);
    }

    /// `item[n]` written whole (`SAVE_ITEM_OWNED` 1, `SAVE_ITEM_NEW` 2).
    pub fn set_item(&mut self, n: usize, v: u8) {
        if let Some(b) = self.image.get_mut(OFS_ITEM + n) {
            *b = v;
        }
    }

    /// `HistOutro_Init`'s count of a saga's first completion: `stockLevel` up by one, `shopFlags` bit 0.
    pub fn raise_stock_level(&mut self) {
        self.set_word(OFS_STOCK_LEVEL, self.word(OFS_STOCK_LEVEL) + 1);
        self.set_word(OFS_SHOP_FLAGS, self.word(OFS_SHOP_FLAGS) | 1);
    }

    /// `trainClear[class]`: the cleared lessons of a training class.
    pub fn train_clear(&self, class: usize) -> i32 {
        self.word(OFS_TRAIN_CLEAR + class * 4)
    }

    pub fn set_train_clear(&mut self, class: usize, v: i32) {
        self.set_word(OFS_TRAIN_CLEAR + class * 4, v);
    }

    /// `flags`: `SAVE_FLAG_VOICE` (1), each pad's vibration (2, 4) and own key assignment (8, 0x10).
    pub fn flags(&self) -> i32 {
        self.word(OFS_FLAGS)
    }

    pub fn set_flags(&mut self, v: i32) {
        self.set_word(OFS_FLAGS, v);
    }

    /// `camDistMode` (0..2) and `camShakeOff`: the options' screen page.
    pub fn cam_dist_mode(&self) -> i32 {
        self.word(OFS_CAM_DIST_MODE)
    }

    pub fn set_cam_dist_mode(&mut self, v: i32) {
        self.set_word(OFS_CAM_DIST_MODE, v);
    }

    pub fn cam_shake_off(&self) -> i32 {
        self.word(OFS_CAM_SHAKE_OFF)
    }

    pub fn set_cam_shake_off(&mut self, v: i32) {
        self.set_word(OFS_CAM_SHAKE_OFF, v);
    }

    /// `screenX` / `screenY`: the display offset, -16..16 each.
    pub fn screen_pos(&self) -> (i32, i32) {
        (self.word(OFS_SCREEN_X), self.word(OFS_SCREEN_Y))
    }

    pub fn set_screen_pos(&mut self, x: i32, y: i32) {
        self.set_word(OFS_SCREEN_X, x);
        self.set_word(OFS_SCREEN_Y, y);
    }

    /// `soundMode`: 0 stereo, 1 mono.
    pub fn sound_mode(&self) -> i32 {
        self.word(OFS_SOUND_MODE)
    }

    pub fn set_sound_mode(&mut self, v: i32) {
        self.set_word(OFS_SOUND_MODE, v);
    }

    /// `bgmVolume` / `seVolume`: 0..9.
    pub fn volumes(&self) -> (i32, i32) {
        (self.word(OFS_BGM_VOLUME), self.word(OFS_SE_VOLUME))
    }

    pub fn set_volumes(&mut self, bgm: i32, se: i32) {
        self.set_word(OFS_BGM_VOLUME, bgm);
        self.set_word(OFS_SE_VOLUME, se);
    }

    /// `key[pad][k]`: the key assignment in force, and `keyEdit[pad][k]`, the copy the options edit.
    #[cfg(test)]
    pub fn key(&self, pad: usize, k: usize) -> i32 {
        self.word(OFS_KEY + pad * 0x20 + k * 4)
    }

    pub fn set_key(&mut self, pad: usize, k: usize, v: i32) {
        self.set_word(OFS_KEY + pad * 0x20 + k * 4, v);
    }

    pub fn key_edit(&self, pad: usize, k: usize) -> i32 {
        self.word(OFS_KEY_EDIT + pad * 0x20 + k * 4)
    }

    pub fn set_key_edit(&mut self, pad: usize, k: usize, v: i32) {
        self.set_word(OFS_KEY_EDIT + pad * 0x20 + k * 4, v);
    }

    /// `money`.
    pub fn money(&self) -> i32 {
        self.word(OFS_MONEY)
    }

    /// `Save_AddMoney`: clamped to 0..9,999,999.
    pub fn add_money(&mut self, amount: i32) {
        let v = self.money().saturating_add(amount).clamp(0, MONEY_MAX);
        self.set_word(OFS_MONEY, v);
    }

    pub fn set_slot_val(&mut self, n: usize, k: usize, v: i32) {
        self.set_word(OFS_SLOT + n * 0x10 + 4 + k * 4, v);
    }

    /// `ZSAVE->rec[slot] = rec`: a custom character with its items and extra slots (the record's other two
    /// fields 0).
    pub fn store_rec(&mut self, slot: usize, chara: i32, items: [u16; 8], level: u16) {
        self.rec_chara[slot] = chara;
        self.rec_items[slot] = items;
        self.rec_level[slot] = level;
        let at = OFS_REC + slot * 0x1C;
        if let Some(b) = self.image.get_mut(at + 0x10..at + 0x18) {
            b[..4].fill(0);
            b[6..].fill(0);
        }
    }

    /// The block with the typed fields written in and its checksum (`McCard_Checksum(.., 1)`).
    pub fn to_image(&self) -> Vec<u8> {
        let mut b = self.image.clone();
        b.resize(SIZE, 0);
        for (i, w) in self.chara_bits.iter().enumerate() {
            let at = OFS_CHARA_BITS + i * 8;
            b[at..at + 8].copy_from_slice(&w.to_le_bytes());
        }
        b[OFS_STAGE_BITS..OFS_STAGE_BITS + 8].copy_from_slice(&self.stage_bits.to_le_bytes());
        b[OFS_BGM_BITS..OFS_BGM_BITS + 4].copy_from_slice(&self.bgm_bits.to_le_bytes());
        for (i, r) in self.rules.iter().enumerate() {
            put(&mut b, OFS_RULE + i * 4, *r);
        }
        for (i, (sets, level)) in self.custom_items.iter().zip(&self.custom_level).enumerate() {
            let at = OFS_CUSTOM + i * 0x38;
            for (k, id) in sets.iter().flatten().enumerate() {
                b[at + k * 2..at + k * 2 + 2].copy_from_slice(&id.to_le_bytes());
            }
            b[at + 0x34..at + 0x36].copy_from_slice(&level.to_le_bytes());
        }
        for i in 0..REC_COUNT {
            let at = OFS_REC + i * 0x1C;
            for (k, id) in self.rec_items[i].iter().enumerate() {
                b[at + k * 2..at + k * 2 + 2].copy_from_slice(&id.to_le_bytes());
            }
            b[at + 0x14..at + 0x16].copy_from_slice(&self.rec_level[i].to_le_bytes());
            put(&mut b, at + 0x18, self.rec_chara[i]);
        }
        crate::memcard::checksum(&mut b, true);
        b
    }

    /// The save of a block the card gave back.
    pub fn from_image(img: &[u8]) -> Self {
        let mut b = img.to_vec();
        b.resize(SIZE, 0);
        let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        let i32_at = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        Self {
            chara_bits: std::array::from_fn(|i| u64_at(OFS_CHARA_BITS + i * 8)),
            stage_bits: u64_at(OFS_STAGE_BITS),
            bgm_bits: i32_at(OFS_BGM_BITS) as u32,
            rules: std::array::from_fn(|i| i32_at(OFS_RULE + i * 4)),
            rec_chara: std::array::from_fn(|i| i32_at(OFS_REC + i * 0x1C + 0x18)),
            rec_items: std::array::from_fn(|i| {
                std::array::from_fn(|k| u16_at(OFS_REC + i * 0x1C + k * 2))
            }),
            rec_level: std::array::from_fn(|i| u16_at(OFS_REC + i * 0x1C + 0x14)),
            custom_items: (0..CUSTOM_COUNT)
                .map(|i| {
                    std::array::from_fn(|set| {
                        std::array::from_fn(|k| u16_at(OFS_CUSTOM + i * 0x38 + (set * 8 + k) * 2))
                    })
                })
                .collect(),
            custom_level: (0..CUSTOM_COUNT)
                .map(|i| u16_at(OFS_CUSTOM + i * 0x38 + 0x34))
                .collect(),
            image: b.clone(),
        }
    }

    /// The sound options the save holds (`flags` bit 0, `volume[3]` at 0x16A4).
    pub fn sound_options(&self) -> crate::sound::SoundOptions {
        let word = |at: usize| {
            self.image
                .get(at..at + 4)
                .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
        };
        crate::sound::SoundOptions {
            mono: word(0x16A4) != 0,
            bgm_volume: word(OFS_BGM_VOLUME),
            se_volume: word(OFS_SE_VOLUME),
            alt_voice: word(OFS_FLAGS) & 1 != 0,
        }
    }

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

    /// The block as the card stores it: the defaults at their offsets, a checksum that holds, and the fields
    /// back from it.
    #[test]
    fn the_block_round_trips_through_its_image() {
        let items = [
            bt3_data::chara_data::ItemInfo::default(),
            bt3_data::chara_data::ItemInfo {
                flags: 2,
                ..Default::default()
            },
        ];
        let mut save = SaveData::new(&items);
        save.rules[0] = 4;
        save.custom_items[5][1][2] = 0x33;
        save.rec_chara[3] = 12;
        let mut img = save.to_image();
        assert_eq!(img.len(), super::SIZE);
        assert!(!crate::memcard::checksum(&mut img, false));
        let word = |at: usize| i32::from_le_bytes(img[at..at + 4].try_into().unwrap());
        assert_eq!(
            (word(0x1608), word(0x16A8), word(0x77C), word(0xC34)),
            (7, 9, 99, 4)
        );
        assert_eq!((img[0x2EC8], img[0x2EC9]), (0, 3));
        let back = SaveData::from_image(&img);
        assert_eq!(back.rules, save.rules);
        assert_eq!(back.custom_items[5][1][2], 0x33);
        assert_eq!(back.rec_chara, save.rec_chara);
        assert_eq!(back.chara_bits, save.chara_bits);
        assert_eq!(back.to_image(), img);
    }

    /// A new save's options are a fresh save's; the block's own words are read.
    #[test]
    fn the_sound_options_come_from_the_block() {
        let mut save = SaveData::default();
        assert_eq!(save.sound_options(), crate::sound::SoundOptions::default());
        save.image[0x16A4] = 1;
        save.image[0x16A8] = 4;
        save.image[0x16AC] = 2;
        save.image[0x1608] = 6;
        let o = save.sound_options();
        assert!(o.mono && !o.alt_voice);
        assert_eq!((o.bgm_volume, o.se_volume), (4, 2));
    }
}
