//! Battle tables of a character file (`BtlObj_BindTables`, src/battle/btl_obj_anim.c) and of common file 2
//! (`BtlChar_AllocAll`, src/battle/btl_char_mgr.c): the parameter block (section 0x12, `BtlParam`), the attack
//! records (section 0x13, `BtlAtkRecord`), the ki blast records (section 0x14, `BtlKiBlastData`), the movement
//! table (section 0x15, `BtlMoveData`) and the body volumes (section 2, `BobjBodyData`); the roster's motion flags,
//! attack actions and character table; and the ki blast emitter set headers of the character's effect pack
//! (`EftKiBlastMgr_Init`).

use std::io;

/// `BtlObj_BindTables`' sections.
pub const BODY_SECTION: usize = 2;
pub const PARAM_SECTION: usize = 0x12;
pub const ATTACK_SECTION: usize = 0x13;
pub const KI_BLAST_SECTION: usize = 0x14;
pub const MOVE_SECTION: usize = 0x15;
/// The technique table (`mdl->super`, object + 0x92C: the AI's "skill table").
pub const SUPER_SECTION: usize = 0x18;
/// The two-slot skill table (`mdl->skill`, object + 0x930: the AI's "move table").
pub const SKILL_SECTION: usize = 0x19;
/// The character's own AI parameters (`mdl->aiParam`, object + 0x934).
pub const AI_PARAM_SECTION: usize = 0x1B;
/// `BTL_ATK_COUNT`.
pub const ATTACK_COUNT: usize = 0xA3;
const ATTACK_SIZE: usize = 0x30;
/// `BTL_KIBLAST_COUNT`.
pub const KI_BLAST_COUNT: usize = 13;
const KI_BLAST_SIZE: usize = 0x34;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |w| u32::from_le_bytes(w.try_into().unwrap()))
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_at(b, o))
}

/// A section that ends where the next begins (`BObjFile_GetEntry`: equal offsets mean none).
fn bounded_section(file: &[u8], n: usize) -> Option<&[u8]> {
    let start = (u32_at(file, n * 4) & !3) as usize;
    let end = (u32_at(file, n * 4 + 4) & !3) as usize;
    (start < end).then(|| file.get(start..end)).flatten()
}

/// `BtlParam` (object + 0x91C), read by offset.
#[derive(Clone, Debug, Default)]
pub struct CharaParams(Vec<u8>);

impl CharaParams {
    pub fn u8_at(&self, o: usize) -> u8 {
        self.0.get(o).copied().unwrap_or(0)
    }
    pub fn u16_at(&self, o: usize) -> u16 {
        self.0
            .get(o..o + 2)
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn s32_at(&self, o: usize) -> i32 {
        u32_at(&self.0, o) as i32
    }
    pub fn f32_at(&self, o: usize) -> f32 {
        f32_at(&self.0, o)
    }
    /// `BobjParam.height` (+4): scale of the vertical root motion.
    pub fn height(&self) -> f32 {
        self.f32_at(4)
    }
    /// `BobjParam.radius` (+8): the body sphere radius, <= 0 for twice the model's size[3].
    pub fn radius(&self) -> f32 {
        self.f32_at(8)
    }
    /// `BtlParam_GetCharaFlags` (+0).
    pub fn chara_flags(&self) -> u16 {
        self.u16_at(0)
    }
    /// `BtlParam_GetSizeClass` (+2, signed).
    pub fn size_class(&self) -> i32 {
        i32::from(self.u8_at(2) as i8)
    }
    /// `BtlParam_GetFlags` (+0x10).
    pub fn flags(&self) -> u32 {
        self.s32_at(0x10) as u32
    }
    /// `BtlParam_GetFlags2` (+0x14).
    pub fn flags2(&self) -> u32 {
        self.s32_at(0x14) as u32
    }
    /// `BtlParam_GetFlags3` (+0x18).
    pub fn flags3(&self) -> u32 {
        self.s32_at(0x18) as u32
    }
}

/// `BtlAtkRecord`, 0x30 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackRecord(pub [u8; ATTACK_SIZE]);

impl AttackRecord {
    pub fn flags(&self) -> u32 {
        u32_at(&self.0, 0)
    }
    pub fn u16_at(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.0[o], self.0[o + 1]])
    }
    pub fn u8_at(&self, o: usize) -> u8 {
        self.0[o]
    }
    pub fn s8_at(&self, o: usize) -> i8 {
        self.0[o] as i8
    }
}

/// `BtlKiBlastData`, 0x34 bytes: one kind of ki blast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KiBlastRecord(pub [u8; KI_BLAST_SIZE]);

impl Default for KiBlastRecord {
    fn default() -> Self {
        Self([0; KI_BLAST_SIZE])
    }
}

impl KiBlastRecord {
    /// 1: cannot be guarded (except by guard kind 6); 2: cannot be deflected, reflected or absorbed; 0x10: no
    /// flinch against a tougher victim; 0x20: copied into the spawn request.
    pub fn flags(&self) -> u16 {
        u16::from_le_bytes([self.0[0], self.0[1]])
    }
    /// Spark module, sound class, deflect rules.
    pub fn kind_type(&self) -> i8 {
        self.0[2] as i8
    }
    pub fn unk3(&self) -> u8 {
        self.0[3]
    }
    /// Total over all hits.
    pub fn damage(&self) -> i32 {
        u32_at(&self.0, 4) as i32
    }
    pub fn guard_damage(&self) -> i32 {
        u32_at(&self.0, 8) as i32
    }
    pub fn ki(&self) -> i32 {
        u32_at(&self.0, 0xC) as i32
    }
    /// 10 km/h.
    pub fn speed(&self) -> f32 {
        f32_at(&self.0, 0x10)
    }
    /// Degrees per second.
    pub fn turn(&self) -> f32 {
        f32_at(&self.0, 0x14)
    }
    /// Seconds.
    pub fn time(&self) -> f32 {
        f32_at(&self.0, 0x18)
    }
    /// 10 km/h, total over all hits.
    pub fn push(&self) -> f32 {
        f32_at(&self.0, 0x1C)
    }
    pub fn guard_push(&self) -> f32 {
        f32_at(&self.0, 0x20)
    }
    /// Reaction ids: plain, against state bits 0x800 / 0x4000 / 0x200 (0: the plain one).
    pub fn react(&self) -> i8 {
        self.0[0x24] as i8
    }
    pub fn react_s800(&self) -> i8 {
        self.0[0x25] as i8
    }
    pub fn react_s4000(&self) -> i8 {
        self.0[0x26] as i8
    }
    pub fn react_s200(&self) -> i8 {
        self.0[0x27] as i8
    }
    pub fn radius(&self) -> f32 {
        f32_at(&self.0, 0x28)
    }
    pub fn unk2c(&self) -> f32 {
        f32_at(&self.0, 0x2C)
    }
    /// The hits the damage and the knock-back are spread over.
    pub fn hits(&self) -> i8 {
        self.0[0x30] as i8
    }
    pub fn spread_mode(&self) -> i8 {
        self.0[0x31] as i8
    }
}

/// `BtlMoveData` (object + 0x928), all floats except the ki costs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveTable {
    /// 10 km/h.
    pub speed: [f32; 20],
    pub accel: [f32; 2],
    pub angle58: f32,
    pub height: [f32; 2],
    pub ki_cost: [i32; 4],
    /// Degrees per second.
    pub turn_rate: [f32; 3],
    pub turn_accel: [f32; 4],
    pub turn_max: [f32; 4],
}

/// `BobjBodyPart`: an oriented box around a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyPart {
    pub node: u16,
    /// Centre in model space (minus the bone's origin it is in the node's space).
    pub offset: [f32; 3],
    /// Box half extents (`BobjObb.half`).
    pub half: [f32; 3],
    /// Radius of the part's sphere (+0x30), the volume projectiles are tested against.
    pub radius: f32,
}

/// `BobjBodyData`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyData {
    pub base_radius: f32,
    pub parts: Vec<BodyPart>,
}

/// What a character file gives the battle code besides its model.
#[derive(Clone, Debug, Default)]
pub struct CharaTables {
    pub params: CharaParams,
    pub attacks: Vec<AttackRecord>,
    /// `KI_BLAST_COUNT` records; zeroed when the file has none.
    pub ki_blasts: Vec<KiBlastRecord>,
    pub moves: MoveTable,
    /// `None` when the model has no body volumes.
    pub body: Option<BodyData>,
    /// Raw technique / skill tables (empty when the file has none) and the AI parameters (`None`: no AI).
    pub super_table: Vec<u8>,
    pub skill_table: Vec<u8>,
    pub ai_param: Option<Vec<u8>>,
}

impl CharaTables {
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let params = CharaParams(
            bounded_section(file, PARAM_SECTION)
                .ok_or_else(|| invalid_data("character has no parameter block"))?
                .to_vec(),
        );
        let attacks = bounded_section(file, ATTACK_SECTION)
            .ok_or_else(|| invalid_data("character has no attack records"))?
            .chunks_exact(ATTACK_SIZE)
            .map(|c| AttackRecord(c.try_into().unwrap()))
            .collect::<Vec<_>>();
        if attacks.len() < ATTACK_COUNT {
            return Err(invalid_data("character has too few attack records"));
        }
        let mut ki_blasts = bounded_section(file, KI_BLAST_SECTION)
            .unwrap_or_default()
            .chunks_exact(KI_BLAST_SIZE)
            .take(KI_BLAST_COUNT)
            .map(|c| KiBlastRecord(c.try_into().unwrap()))
            .collect::<Vec<_>>();
        ki_blasts.resize(KI_BLAST_COUNT, KiBlastRecord::default());
        let mv = bounded_section(file, MOVE_SECTION)
            .filter(|s| s.len() >= 0xA0)
            .ok_or_else(|| invalid_data("character has no movement table"))?;
        let floats = |o: usize, out: &mut [f32]| {
            for (i, v) in out.iter_mut().enumerate() {
                *v = f32_at(mv, o + i * 4);
            }
        };
        let mut moves = MoveTable::default();
        floats(0, &mut moves.speed);
        floats(0x50, &mut moves.accel);
        moves.angle58 = f32_at(mv, 0x58);
        floats(0x5C, &mut moves.height);
        for (i, v) in moves.ki_cost.iter_mut().enumerate() {
            *v = u32_at(mv, 0x64 + i * 4) as i32;
        }
        floats(0x74, &mut moves.turn_rate);
        floats(0x80, &mut moves.turn_accel);
        floats(0x90, &mut moves.turn_max);
        let body = bounded_section(file, BODY_SECTION)
            .map(|b| {
                let mut parts = Vec::new();
                let mut o = 0x10;
                loop {
                    let rec = b
                        .get(o..o + 0x1D0)
                        .ok_or_else(|| invalid_data("body part list is not terminated"))?;
                    let flags = u32_at(rec, 0);
                    parts.push(BodyPart {
                        node: u32_at(rec, 4) as u16,
                        offset: [f32_at(rec, 0x10), f32_at(rec, 0x14), f32_at(rec, 0x18)],
                        half: [f32_at(rec, 0x50), f32_at(rec, 0x54), f32_at(rec, 0x58)],
                        radius: f32_at(rec, 0x30),
                    });
                    if flags & 1 != 0 {
                        break;
                    }
                    o += 0x1D0;
                }
                Ok::<_, io::Error>(BodyData {
                    base_radius: f32_at(b, 4),
                    parts,
                })
            })
            .transpose()?;
        let raw = |n: usize| bounded_section(file, n).map(<[u8]>::to_vec);
        Ok(Self {
            params,
            attacks,
            ki_blasts,
            moves,
            body,
            super_table: raw(SUPER_SECTION).unwrap_or_default(),
            skill_table: raw(SKILL_SECTION).unwrap_or_default(),
            ai_param: raw(AI_PARAM_SECTION),
        })
    }
}

/// `gBtlChars->paramTbl[chara]` (common file 2, table at header word 8, 0x10 bytes each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharaEntry {
    pub health_max: i32,
    pub ki_start: i32,
    pub blast_stocks: i32,
}

/// The roster's tables of common file 2 (`BtlChar_AllocAll`).
#[derive(Clone, Debug, Default)]
pub struct CommonTables {
    /// `tbl[0]` (word 3): flags per motion id.
    pub motion_flags: Vec<u32>,
    /// `tbl[3]` (word 7): one row per attack action (0x70..), 0x18 bytes (`BtlActAttackRec` / `BtlTechAtkAction`).
    pub attack_actions: Vec<[u8; 0x18]>,
    /// `tbl[4]` (word 8).
    pub charas: Vec<CharaEntry>,
}

impl CommonTables {
    pub fn parse(common: &[u8]) -> io::Result<Self> {
        let table = |word: usize| {
            let start = (u32_at(common, word * 4) & !3) as usize;
            let end = (u32_at(common, word * 4 + 4) & !3) as usize;
            common
                .get(start..end.max(start))
                .ok_or_else(|| invalid_data("common table is outside the file"))
        };
        let motion_flags = table(3)?
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            .collect();
        let attack_actions = table(7)?
            .chunks_exact(0x18)
            .map(|c| c.try_into().unwrap())
            .collect();
        let charas = table(8)?
            .chunks_exact(0x10)
            .map(|c| CharaEntry {
                health_max: u32_at(c, 0) as i32,
                ki_start: u32_at(c, 4) as i32,
                blast_stocks: u32_at(c, 0xC) as i32,
            })
            .collect();
        Ok(Self {
            motion_flags,
            attack_actions,
            charas,
        })
    }
}

/// The header of an emitter set (`EftSetHead`, 0x40 bytes) as far as the simulation reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmitSetHead {
    /// `width0`: the trail width a ki blast's hit radius is scaled by (the blast never steps that animation).
    pub width0: f32,
    /// `endFrames`: frames the end animation runs.
    pub end_frames: u8,
    /// Bit 4: the second width animation is used (`EftEmit_HasWidth2`).
    pub flags: u8,
}

/// What the battle reads from a character's effect pack (entry 6 of the character's second animation file,
/// `BtlCharApi_GetPlayerCharPack`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectPack {
    /// The first word of entry 1 of entry 1 (`BtlScene_TestCharPackBit`): bit n - 1 creates the manager of kind n.
    pub kind_bits: u32,
    /// The set headers of the ki blast managers (`EftKiBlastMgr_Init`): kind 1 is entry 3, kind 2 entry 4; each
    /// header is entry 1 of its entry. `None` when it lies outside the file.
    pub blast_sets: [Option<EmitSetHead>; 2],
}

/// `BObjFile_GetEntry`: entry `n` of a pack, `None` when it is empty.
fn pack_entry(pack: &[u8], n: usize) -> Option<&[u8]> {
    let start = (u32_at(pack, n * 4) & !3) as usize;
    let end = (u32_at(pack, n * 4 + 4) & !3) as usize;
    (start != end).then(|| pack.get(start..)).flatten()
}

/// `BtlScene_GetPackEntry`: entry `n` without the emptiness test.
fn pack_entry_any(pack: &[u8], n: usize) -> Option<&[u8]> {
    pack.get((u32_at(pack, n * 4) & !3) as usize..)
}

impl EffectPack {
    /// From the character's second animation file (`chara * 10 + 0x599`); `None` without the pack.
    pub fn parse(anim_file: &[u8]) -> Option<Self> {
        let pack = pack_entry(anim_file, 6)?;
        let kind_bits = pack_entry_any(pack, 1)
            .and_then(|e| pack_entry_any(e, 1))
            .map_or(0, |w| u32_at(w, 0));
        let head = |n: usize| {
            let set = pack_entry_any(pack, n)?;
            let h = pack_entry_any(set, 1)?;
            Some(EmitSetHead {
                width0: f32_at(h, 8),
                end_frames: *h.get(0x20)?,
                flags: *h.get(0x21)?,
            })
        };
        Some(Self {
            kind_bits,
            blast_sets: [head(3), head(4)],
        })
    }
}
