//! Battle tables of a character file (`BtlObj_BindTables`, src/battle/btl_obj_anim.c) and of common file 2
//! (`BtlChar_AllocAll`, src/battle/btl_char_mgr.c): the parameter block (section 0x12, `BtlParam`), the attack
//! records (section 0x13, `BtlAtkRecord`), the ki blast records (section 0x14, `BtlKiBlastData`), the movement
//! table (section 0x15, `BtlMoveData`) and the body volumes (section 2, `BobjBodyData`); the roster's motion flags,
//! attack actions and character table.

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
/// The character's camera cut table (`mdl->unk920`, object + 0x938: `ChrCam_RequestCut`'s table 0).
pub const CAM_CUT_SECTION: usize = 0x1C;
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
    /// `BtlParam_GetSlotId` (+0x98): transformation `i`'s character, 0xFF none.
    pub fn trans_target(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0x98 + i))
    }
    /// `BtlParam_GetSlotCost` (+0x9C): its blast cost (stocks * 100000).
    pub fn trans_cost(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0x9C + i)) * 100_000
    }
    /// `BtlParam_GetSlotA0` (+0xA0): its sequence 0..4 (actions 0xEC..0xF0).
    pub fn trans_seq(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xA0 + i))
    }
    /// `BtlParam_GetSlotA4` (+0xA4): its kind (3 needs the stage's moon).
    pub fn trans_kind(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xA4 + i))
    }
    /// `BtlParam_GetSlotA8` (+0xA8): the extra object of sequence 3.
    pub fn trans_object(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xA8 + i))
    }
    /// `BtlParam_GetDefaultSlot` (+0xAC, signed): the transformation of R3 without a direction.
    pub fn trans_default(&self) -> i32 {
        i32::from(self.u8_at(0xAC) as i8)
    }
    /// `BtlParam_GetFormFlags` (+0xAD): 1 / 2 + 5000 / 10000 health and 4 full ki after the change; 0x10 << i
    /// transformation `i` resets the damaged variant.
    pub fn form_flags(&self) -> u8 {
        self.u8_at(0xAD)
    }
    /// `BtlParam_GetFusionResult` (+0xB4): fusion `i`'s character, 0xFF none.
    pub fn fusion_result(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xB4 + i))
    }
    /// `BtlParam_GetCostAE` (+0xAE): its blast cost (stocks * 100000).
    pub fn fusion_cost(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xAE + i)) * 100_000
    }
    /// `BtlParam_GetFusionSequence` (+0xB1): 1 / 2 (actions 0xF1 / 0xF2).
    pub fn fusion_seq(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xB1 + i))
    }
    /// `BtlParam_GetUnkB7` (+0xB7): its extra object.
    pub fn fusion_object(&self, i: usize) -> i32 {
        i32::from(self.u8_at(0xB7 + i))
    }
    /// `BtlParam_GetFusionPartner` (+0xBA): the four characters fusion `i` takes as a partner.
    pub fn fusion_partner(&self, i: usize, n: usize) -> i32 {
        i32::from(self.u8_at(0xBA + i * 4 + n))
    }
    /// `BtlParam_CountSlots`: the transformations that are not 0xFF.
    pub fn trans_count(&self) -> usize {
        (0..4).filter(|&i| self.trans_target(i) != 0xFF).count()
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
    /// The camera cuts the character's moves ask for (`ChrCam_RequestCut(chr, 0, n)`).
    pub cam_cuts: Vec<CamCutDef>,
}

/// `ChrCamCutDef` (0x24 bytes): a camera cut the fighter camera blends over its time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CamCutDef {
    /// The model node the camera orbits, moving towards `node_a2`; the node looked at, moving towards `node_c2`.
    pub node_a: u8,
    pub node_a2: u8,
    pub node_c: u8,
    pub node_c2: u8,
    /// Degrees, relative to the fighter's facing, and what is added over the cut.
    pub yaw: f32,
    pub yaw_delta: f32,
    pub pitch: f32,
    pub pitch_delta: f32,
    /// In body scales.
    pub dist: f32,
    pub dist_delta: f32,
    pub seconds: f32,
    /// `CHRCUT_F_*`.
    pub cut_flags: u16,
    /// `CHRCUTDEF_*`.
    pub flags: u16,
}

impl CamCutDef {
    /// Every whole 0x24-byte entry of a table.
    pub fn parse_table(b: &[u8]) -> Vec<Self> {
        b.chunks_exact(0x24)
            .map(|c| Self {
                node_a: c[0],
                node_a2: c[1],
                node_c: c[2],
                node_c2: c[3],
                yaw: f32_at(c, 4),
                yaw_delta: f32_at(c, 8),
                pitch: f32_at(c, 0xC),
                pitch_delta: f32_at(c, 0x10),
                dist: f32_at(c, 0x14),
                dist_delta: f32_at(c, 0x18),
                seconds: f32_at(c, 0x1C),
                cut_flags: u16::from_le_bytes([c[0x20], c[0x21]]),
                flags: u16::from_le_bytes([c[0x22], c[0x23]]),
            })
            .collect()
    }
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
            cam_cuts: bounded_section(file, CAM_CUT_SECTION)
                .map(CamCutDef::parse_table)
                .unwrap_or_default(),
        })
    }
}

/// The technique table (`BtlSuperData`, `mdl->super`): every field an array of three for the slots 2..4.
/// Out-of-range reads give 0.
#[derive(Clone, Copy, Debug)]
pub struct TechTable<'a>(pub &'a [u8]);

impl TechTable<'_> {
    fn at<const N: usize>(&self, base: usize, slot: i32, size: usize) -> Option<[u8; N]> {
        let i = usize::try_from(slot - 2).ok().filter(|&i| i < 3)?;
        let o = base + i * size;
        self.0.get(o..o + N)?.try_into().ok()
    }
    fn u32_of(&self, base: usize, slot: i32) -> u32 {
        self.at::<4>(base, slot, 4).map_or(0, u32::from_le_bytes)
    }
    fn f32_of(&self, base: usize, slot: i32) -> f32 {
        self.at::<4>(base, slot, 4).map_or(0.0, f32::from_le_bytes)
    }
    fn i16_of(&self, base: usize, slot: i32) -> i32 {
        self.at::<2>(base, slot, 2)
            .map_or(0, |b| i32::from(i16::from_le_bytes(b)))
    }
    fn s8_of(&self, base: usize, slot: i32) -> i32 {
        self.at::<1>(base, slot, 1)
            .map_or(0, |b| i32::from(b[0] as i8))
    }
    /// `BtlSuper_GetFlagsA` / `GetFlags`: the two flag words.
    pub fn flags_a(&self, slot: i32) -> u32 {
        self.u32_of(0, slot)
    }
    pub fn flags(&self, slot: i32) -> u32 {
        self.u32_of(0xC, slot)
    }
    /// `BtlSuper_GetId`.
    pub fn id(&self, slot: i32) -> i32 {
        self.i16_of(0x18, slot)
    }
    /// `BtlSuper_GetClashPower`.
    pub fn clash_power(&self, slot: i32) -> i32 {
        self.i16_of(0x1E, slot)
    }
    /// `BtlSuper_GetShotTime` (seconds).
    pub fn shot_time(&self, slot: i32) -> f32 {
        self.f32_of(0x24, slot)
    }
    /// `BtlSuper_GetShotSpeed` / `GetShotTurnRate` / `GetShotUnk6C` (raw: 10 km/h, degrees per second, the
    /// effect's scale).
    pub fn shot_speed(&self, slot: i32) -> f32 {
        self.f32_of(0x54, slot)
    }
    pub fn shot_turn(&self, slot: i32) -> f32 {
        self.f32_of(0x60, slot)
    }
    pub fn shot_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x6C, slot)
    }
    /// The impacts of the effect (`EftShot_BuildParam`): the effect id on a fighter (+0xA5) and on the stage
    /// (+0xA2), negative for none, and their scales (+0x84, +0x78).
    pub fn impact_fx(&self, slot: i32) -> i32 {
        self.s8_of(0xA5, slot)
    }
    pub fn ground_fx(&self, slot: i32) -> i32 {
        self.s8_of(0xA2, slot)
    }
    pub fn impact_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x84, slot)
    }
    pub fn ground_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x78, slot)
    }
    /// `shotLife` (+0x30): frames a blast object of effect type 2 lives before it is stopped.
    pub fn shot_life(&self, slot: i32) -> f32 {
        self.f32_of(0x30, slot)
    }
    /// `unk214` (`EftShotParam.shotSpeed`): the speed of effect type 2's target point (raw, per frame).
    pub fn item_speed(&self, slot: i32) -> f32 {
        self.f32_of(0x214, slot)
    }
    /// The struggle power (+0x3C, `EftShotParam.power`).
    pub fn power(&self, slot: i32) -> f32 {
        self.f32_of(0x3C, slot)
    }
    /// The hit records' scale (+0x48, `EftShotParam.hitScale`): the factor of their radius in the stage test.
    pub fn hit_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x48, slot)
    }
    /// The effect's hit shape (+0x99: 0 two spheres, 1 two capsules from the muzzle), raw hit limit (+0x9C, what
    /// `hits_c` adds 1 to) and frames between two hits (+0x9F) (`EftShot_BuildParam`).
    pub fn hit_shape(&self, slot: i32) -> i32 {
        self.s8_of(0x99, slot)
    }
    pub fn max_hits(&self, slot: i32) -> i32 {
        self.s8_of(0x9C, slot)
    }
    pub fn hit_interval(&self, slot: i32) -> i32 {
        self.s8_of(0x9F, slot)
    }
    /// The effect's node per event slot (+0xA8, stride 3).
    pub fn effect_nodes(&self, slot: i32) -> [i32; 6] {
        std::array::from_fn(|i| self.s8_of(0xA8 + i * 3, slot))
    }
    /// The effect type (+0x138: 0 the plain blast / beam, -1 none; `EftShot_BuildParam`).
    pub fn effect_type(&self, slot: i32) -> i32 {
        self.s8_of(0x138, slot)
    }
    /// `BtlSuper_GetGuardKind`.
    pub fn guard_kind(&self, slot: i32) -> i32 {
        self.s8_of(0x150, slot)
    }
    /// `BtlSuper_GetHitSound` (+0x153): the hit sound kind of the first / second (`alt`) hit variant.
    pub fn hit_sound(&self, slot: i32, alt: bool) -> i32 {
        self.s8_of(0x153 + usize::from(alt) * 3, slot)
    }
    /// The shot count and the shots per firing as stored (`EftShot_BuildParam`'s `shots` / `volley`).
    pub fn shots_raw(&self, slot: i32) -> i32 {
        self.s8_of(0x93, slot)
    }
    pub fn hits_b_raw(&self, slot: i32) -> i32 {
        self.s8_of(0x96, slot)
    }
    /// `BtlSuper_GetShots` / `GetHitsB` (at least 1) / `GetHitsC` (byte + 1).
    pub fn shots(&self, slot: i32) -> i32 {
        self.s8_of(0x93, slot).max(1)
    }
    pub fn hits_b(&self, slot: i32) -> i32 {
        self.s8_of(0x96, slot).max(1)
    }
    pub fn hits_c(&self, slot: i32) -> i32 {
        self.s8_of(0x9C, slot) + 1
    }
    /// `BtlSuper_GetHitCount`: the product of the three factors, at least 1.
    pub fn hit_count(&self, slot: i32) -> i32 {
        let n = self.shots(slot) * self.hits_b(slot) * self.hits_c(slot);
        if n > 0 { n } else { 1 }
    }
    /// `BtlSuper_GetHitDirKind`.
    pub fn hit_dir_kind(&self, slot: i32) -> i32 {
        self.s8_of(0x13B, slot)
    }
    /// `BtlSuper_GetType`: picks the action block.
    pub fn kind(&self, slot: i32) -> i32 {
        self.s8_of(0x13E, slot)
    }
    /// `BtlSuper_GetReact` / `GetReactAlt` / `GetReactS800` / `GetReactAltS800`.
    pub fn react(&self, slot: i32) -> i32 {
        self.s8_of(0x141, slot)
    }
    pub fn react_alt(&self, slot: i32) -> i32 {
        let v = self.s8_of(0x144, slot);
        if v > 0 { v } else { self.react(slot) }
    }
    pub fn react_s800(&self, slot: i32) -> i32 {
        self.s8_of(0x147, slot)
    }
    pub fn react_alt_s800(&self, slot: i32) -> i32 {
        let v = self.s8_of(0x14A, slot);
        if v > 0 { v } else { self.react_s800(slot) }
    }
    /// `BtlSuper_IsThrow`: one of the four reactions is 0x22.
    pub fn is_throw(&self, slot: i32) -> bool {
        [
            self.react(slot),
            self.react_alt(slot),
            self.react_s800(slot),
            self.react_alt_s800(slot),
        ]
        .contains(&0x22)
    }
    /// `BtlSuper_GetStepCount` (at least 1).
    pub fn steps(&self, slot: i32) -> i32 {
        self.s8_of(0x159, slot).max(1)
    }
    /// `BtlSuper_GetPush` / `GetGuardPush` (10 km/h, raw).
    pub fn push(&self, slot: i32) -> f32 {
        self.f32_of(0x16C, slot)
    }
    pub fn guard_push(&self, slot: i32) -> f32 {
        self.f32_of(0x178, slot)
    }
    /// The damage over all hits, plain and guarded (raw).
    pub fn damage(&self, slot: i32) -> i32 {
        self.u32_of(0x184, slot) as i32
    }
    pub fn guard_damage(&self, slot: i32) -> i32 {
        self.u32_of(0x190, slot) as i32
    }
    /// The ki cost (raw, before ability 0x2B).
    pub fn ki(&self, slot: i32) -> i32 {
        self.u32_of(0x19C, slot) as i32
    }
    /// `BtlSuper_GetRecoilDamage` / `GetDrain`.
    pub fn recoil(&self, slot: i32) -> i32 {
        self.u32_of(0x1A8, slot) as i32
    }
    pub fn drain(&self, slot: i32) -> i32 {
        self.u32_of(0x1B4, slot) as i32
    }
    /// `BtlSuper_GetCooldownFrames` / `GetChargeLimitFrames`: seconds * 30.
    pub fn cooldown_frames(&self, slot: i32) -> i32 {
        (self.f32_of(0x1C0, slot) * 30.0) as i32
    }
    pub fn charge_limit_frames(&self, slot: i32) -> i32 {
        (self.f32_of(0x1CC, slot) * 30.0) as i32
    }
    /// `BtlSuper_GetChargeScale`.
    pub fn charge_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x1D8, slot)
    }
    /// `BtlSuper_GetPromptRowIndex`.
    pub fn prompt_row(&self, slot: i32) -> i32 {
        self.s8_of(0x229, slot)
    }
    /// `BtlSuper_GetLandingKind`: what the victim of a rush lands in (`BtlAct_GetLandingAction`).
    pub fn landing_kind(&self, slot: i32) -> i32 {
        self.s8_of(0x14D, slot)
    }
    /// `BtlSuper_GetLastStep`: the rush step the victim's model is reloaded in (the last when negative).
    pub fn last_step(&self, slot: i32) -> i32 {
        let n = self.s8_of(0x168, slot);
        if n < 0 { self.steps(slot) - 1 } else { n }
    }
    /// `BtlSuper_GetThrowAngleA` / `GetThrowAngleB`: the victim's turn after a rush (radians).
    pub fn throw_angle_a(&self, slot: i32) -> f32 {
        self.f32_of(0x1E4, slot) * std::f32::consts::PI / 180.0
    }
    pub fn throw_angle_b(&self, slot: i32) -> f32 {
        self.f32_of(0x1F0, slot) * std::f32::consts::PI / 180.0
    }
    /// The rush dash speed (+0x1FC, 10 km/h, raw).
    pub fn rush_speed(&self, slot: i32) -> f32 {
        self.f32_of(0x1FC, slot)
    }
    /// The character a rush brings in and its alternate (+0x208 / +0x20E, raw; `BtlSuper_GetThrowChara`).
    pub fn throw_chara(&self, slot: i32) -> i32 {
        self.i16_of(0x208, slot)
    }
    pub fn throw_chara_alt(&self, slot: i32) -> i32 {
        self.i16_of(0x20E, slot)
    }
    /// `BtlSuper_GetThrowPartnerStep`.
    pub fn throw_partner_step(&self, slot: i32) -> i32 {
        self.s8_of(0x220, slot)
    }
    /// `BtlSuper_GetStageFxEndStep` (999 when negative).
    pub fn stage_fx_end_step(&self, slot: i32) -> i32 {
        let n = self.s8_of(0x223, slot);
        if n < 0 { 999 } else { n }
    }
    /// `BtlSuper_GetThrowObjectSlot`.
    pub fn throw_object_slot(&self, slot: i32) -> i32 {
        self.s8_of(0x226, slot)
    }
    /// `BtlSuper_GetFrames22C` / `GetFrames238`: seconds * 30.
    pub fn frames_22c(&self, slot: i32) -> i32 {
        (self.f32_of(0x22C, slot) * 30.0) as i32
    }
    pub fn frames_238(&self, slot: i32) -> i32 {
        (self.f32_of(0x238, slot) * 30.0) as i32
    }
}

/// The skill table (`BtlSkillData`, `mdl->skill`): every field an array of two for the slots 0..1.
/// Out-of-range reads give 0.
#[derive(Clone, Copy, Debug)]
pub struct SkillTable<'a>(pub &'a [u8]);

impl SkillTable<'_> {
    fn at<const N: usize>(&self, base: usize, slot: i32) -> Option<[u8; N]> {
        let i = usize::try_from(slot).ok().filter(|&i| i < 2)?;
        let o = base + i * N;
        self.0.get(o..o + N)?.try_into().ok()
    }
    fn u32_of(&self, base: usize, slot: i32) -> u32 {
        self.at::<4>(base, slot).map_or(0, u32::from_le_bytes)
    }
    fn s32_of(&self, base: usize, slot: i32) -> i32 {
        self.at::<4>(base, slot).map_or(0, i32::from_le_bytes)
    }
    fn f32_of(&self, base: usize, slot: i32) -> f32 {
        self.at::<4>(base, slot).map_or(0.0, f32::from_le_bytes)
    }
    fn s8_of(&self, base: usize, slot: i32) -> i32 {
        self.at::<1>(base, slot)
            .map_or(0, |b| i32::from(b[0] as i8))
    }
    /// `BtlSkill_GetHitSound` (+0x6C).
    pub fn hit_sound(&self, slot: i32) -> i32 {
        self.s8_of(0x6C, slot)
    }
    /// `BtlSkill_GetFlagsA` (+0x00): the effect definition's flags (`EftShot_BuildParam`).
    pub fn flags_a(&self, slot: i32) -> u32 {
        self.u32_of(0, slot)
    }
    /// `BtlSkill_GetFlags` (+0x08): the attribute bits.
    pub fn flags(&self, slot: i32) -> u32 {
        self.u32_of(0x8, slot)
    }
    /// The effect's clash level (+0x14, `EftShot_BuildParam`'s `level`).
    pub fn level(&self, slot: i32) -> i32 {
        self.at::<2>(0x14, slot)
            .map_or(0, |b| i32::from(u16::from_le_bytes(b)))
    }
    /// `BtlSkill_GetShotTime` (+0x18, seconds), `GetShotSpeed`'s raw 10 km/h (+0x20), `GetShotTurnRate`'s raw
    /// degrees per second (+0x28) and the effect's scale (+0x30).
    pub fn shot_time(&self, slot: i32) -> f32 {
        self.f32_of(0x18, slot)
    }
    pub fn shot_speed(&self, slot: i32) -> f32 {
        self.f32_of(0x20, slot)
    }
    pub fn shot_turn(&self, slot: i32) -> f32 {
        self.f32_of(0x28, slot)
    }
    pub fn shot_scale(&self, slot: i32) -> f32 {
        self.f32_of(0x30, slot)
    }
    /// The impacts of the effect (`EftShot_BuildParam`): the effect id on a fighter (+0x46) and on the stage
    /// (+0x44), negative for none.
    pub fn impact_fx(&self, slot: i32) -> i32 {
        self.s8_of(0x46, slot)
    }
    pub fn ground_fx(&self, slot: i32) -> i32 {
        self.s8_of(0x44, slot)
    }
    /// The hit factors as stored (+0x3A shots, +0x3C hits B, +0x40 hits C less 1), the hit shape (+0x3E) and the
    /// frames between two hits (+0x42).
    pub fn shots_raw(&self, slot: i32) -> i32 {
        self.s8_of(0x3A, slot)
    }
    pub fn hits_b_raw(&self, slot: i32) -> i32 {
        self.s8_of(0x3C, slot)
    }
    pub fn hit_shape(&self, slot: i32) -> i32 {
        self.s8_of(0x3E, slot)
    }
    pub fn max_hits(&self, slot: i32) -> i32 {
        self.s8_of(0x40, slot)
    }
    pub fn hit_interval(&self, slot: i32) -> i32 {
        self.s8_of(0x42, slot)
    }
    /// `BtlSkill_GetHitCount`: shots (at least 1) * hits B (at least 1) * (hits C + 1); at least 1 here.
    pub fn hit_count(&self, slot: i32) -> i32 {
        let n =
            self.shots_raw(slot).max(1) * self.hits_b_raw(slot).max(1) * (self.max_hits(slot) + 1);
        if n > 0 { n } else { 1 }
    }
    /// The effect's node per event slot (+0x48, stride 2) and its frame table (+0x54, -1 none).
    pub fn effect_nodes(&self, slot: i32) -> [i32; 6] {
        std::array::from_fn(|i| self.s8_of(0x48 + i * 2, slot))
    }
    pub fn effect_frames(&self, slot: i32) -> [i32; 6] {
        std::array::from_fn(|i| self.s8_of(0x54 + i * 2, slot))
    }
    /// The effect type (+0x60: 0 the plain blast / beam, -1 none).
    pub fn effect_type(&self, slot: i32) -> i32 {
        self.s8_of(0x60, slot)
    }
    /// `BtlSkill_GetHitDirKind` (+0x62).
    pub fn hit_dir_kind(&self, slot: i32) -> i32 {
        self.s8_of(0x62, slot)
    }
    /// `BtlSkill_GetReact` (+0x68) / `GetReactS800` (+0x6A, the first when not positive).
    pub fn react(&self, slot: i32) -> i32 {
        self.s8_of(0x68, slot)
    }
    pub fn react_s800(&self, slot: i32) -> i32 {
        let r = self.s8_of(0x6A, slot);
        if r > 0 { r } else { self.react(slot) }
    }
    /// `BtlSkill_GetPush` / `GetGuardPush` (+0x74 / +0x7C, as stored): of one hit.
    pub fn push(&self, slot: i32) -> f32 {
        self.f32_of(0x74, slot) / self.hit_count(slot) as f32
    }
    pub fn guard_push(&self, slot: i32) -> f32 {
        self.f32_of(0x7C, slot) / self.hit_count(slot) as f32
    }
    /// `BtlSkill_GetDamage` / `GetGuardDamage` (+0x84 / +0x8C) of one hit.
    pub fn damage(&self, slot: i32) -> i32 {
        self.s32_of(0x84, slot) / self.hit_count(slot)
    }
    pub fn guard_damage(&self, slot: i32) -> i32 {
        self.s32_of(0x8C, slot) / self.hit_count(slot)
    }
    /// `BtlSkill_GetId` (+0x10).
    pub fn id(&self, slot: i32) -> i32 {
        self.at::<2>(0x10, slot)
            .map_or(0, |b| i32::from(i16::from_le_bytes(b)))
    }
    /// `BtlSkill_GetSequence` (+0x64): the action block (0: 0xFD.., 1: 0xFF.., 2: 0x101..).
    pub fn sequence(&self, slot: i32) -> i32 {
        self.s8_of(0x64, slot)
    }
    /// `BtlSkill_GetStatKind` (+0x66): how long the stat levels last.
    pub fn stat_kind(&self, slot: i32) -> i32 {
        self.s8_of(0x66, slot)
    }
    /// `stocks` (+0x94): the blast stocks the skill costs (`BtlSkill_GetBlastCost` before its ability).
    pub fn stocks(&self, slot: i32) -> i32 {
        self.s8_of(0x94, slot)
    }
    /// `BtlSkill_GetStatLevel0..3` (+0x96, +0x9A, +0x98, +0x9C): the level the skill gives stat `stat`.
    pub fn stat_level(&self, slot: i32, stat: usize) -> i32 {
        match stat {
            0 => self.s8_of(0x96, slot),
            1 => self.s8_of(0x9A, slot),
            2 => self.s8_of(0x98, slot),
            3 => self.s8_of(0x9C, slot),
            _ => 0,
        }
    }
    /// `BtlSkill_GetFrames` (+0xA0, seconds).
    pub fn frames(&self, slot: i32) -> i32 {
        (self.f32_of(0xA0, slot) * 30.0) as i32
    }
    /// `healthPct` (+0xA8): percent of the maximum health given (negative: taken).
    pub fn health_pct(&self, slot: i32) -> i32 {
        self.s32_of(0xA8, slot)
    }
    /// `BtlSkill_GetKiChange` (+0xB0).
    pub fn ki(&self, slot: i32) -> i32 {
        self.s32_of(0xB0, slot)
    }
    /// `BtlSkill_GetValF` (+0xB8) / `GetValE` (+0xC0): the two timed values.
    pub fn val_f(&self, slot: i32) -> i32 {
        self.s32_of(0xB8, slot)
    }
    pub fn val_e(&self, slot: i32) -> i32 {
        self.s32_of(0xC0, slot)
    }
}

impl CharaTables {
    /// The technique table.
    pub fn tech(&self) -> TechTable<'_> {
        TechTable(&self.super_table)
    }

    /// The skill table.
    pub fn skill(&self) -> SkillTable<'_> {
        SkillTable(&self.skill_table)
    }
}

/// `gBtlChars->paramTbl[chara]` (common file 2, table at header word 8, 0x10 bytes each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharaEntry {
    pub health_max: i32,
    pub ki_start: i32,
    pub blast_stocks: i32,
}

/// One attack action's record (`BtlActAttackRec`, roster + 0x2C, by action id - 0x70).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AttackRec {
    /// +0x00: the advance speed in 10 km/h.
    pub speed: u8,
    /// +0x01: the lead-in actions `BtlDecide_QueueAttack` queues first (0..9); also the effect level 1..3.
    pub lead_in: u8,
    /// +0x02: the input that starts it (`BtlDecide_QueueAttack`: 1..9 -> inputs 0x4F..0x57, else none).
    pub input_kind: i8,
    /// +0x03: the hit record `BtlAtk_GetId` reads.
    pub hit_id: u8,
    /// +0x04: motions played one after another (motion, motion + 2, ...).
    pub parts: i8,
    /// +0x05: the opponent state the attack needs (0 none, 1..8 a reaction).
    pub kind: i8,
    /// +0x06.
    pub motion: u16,
    /// +0x08: the progress past which a follow-up starts.
    pub cancel_at: f32,
    /// +0x0C: the motion rate in tenths (0: 1.0).
    pub rate: i8,
    /// +0x0D / +0x0F: follow-up records (index from 0x70; < 0 none), the second pair for the powered-up mode.
    pub follow: [i8; 2],
    pub follow_alt: [i8; 2],
    /// +0x11 / +0x12: the camera cuts.
    pub cut: i8,
    pub cut_alt: i8,
    /// +0x13: the skill bit the attack needs (flags2 bits 0..31, flags3 32..63; < 0 none).
    pub need_skill: i8,
    /// +0x14.
    pub flags: u16,
}

impl AttackRec {
    pub fn parse(r: &[u8; 0x18]) -> Self {
        let s8 = |o: usize| r[o] as i8;
        Self {
            speed: r[0],
            lead_in: r[1],
            input_kind: s8(2),
            hit_id: r[3],
            parts: s8(4),
            kind: s8(5),
            motion: u16::from_le_bytes([r[6], r[7]]),
            cancel_at: f32::from_le_bytes([r[8], r[9], r[10], r[11]]),
            rate: s8(0xC),
            follow: [s8(0xD), s8(0xE)],
            follow_alt: [s8(0xF), s8(0x10)],
            cut: s8(0x11),
            cut_alt: s8(0x12),
            need_skill: s8(0x13),
            flags: u16::from_le_bytes([r[0x14], r[0x15]]),
        }
    }
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
    /// `tbl[5]` (word 9, roster + 0x34): the technique prompt rows, 0x14 bytes each (`BtlTechPromptRow`; byte 2 is
    /// the direction the technique's button command wants).
    pub prompt_rows: Vec<[u8; 0x14]>,
    /// `tbl[1]` (word 5, roster + 0x24): the common camera cuts (`ChrCam_RequestCut(chr, 1, n)`).
    pub cam_cuts: Vec<CamCutDef>,
    /// `tbl[2]` (word 6, roster + 0x28, `voiceTbl`): per voice kind its lines and cooldown.
    pub voices: Vec<VoiceEntry>,
}

/// `BtlVoiceEntry`: a voice kind's first line, line count and cooldown in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoiceEntry {
    pub first: i16,
    pub count: i16,
    pub interval: f32,
}

impl CommonTables {
    /// The record of attack action `0x70 + index`.
    pub fn attack_rec(&self, index: usize) -> Option<AttackRec> {
        self.attack_actions.get(index).map(AttackRec::parse)
    }

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
        let prompt_rows = table(9)?
            .chunks_exact(0x14)
            .map(|c| c.try_into().unwrap())
            .collect();
        let cam_cuts = CamCutDef::parse_table(table(5)?);
        let voices = table(6)?
            .chunks_exact(8)
            .map(|v| VoiceEntry {
                first: i16::from_le_bytes([v[0], v[1]]),
                count: i16::from_le_bytes([v[2], v[3]]),
                interval: f32::from_le_bytes(v[4..8].try_into().unwrap()),
            })
            .collect();
        Ok(Self {
            motion_flags,
            attack_actions,
            charas,
            prompt_rows,
            cam_cuts,
            voices,
        })
    }
}

/// `ITEMTBL_TYPE_AI`: an item that sets the AI type to its id less `ITEM_AI_FIRST`.
const ITEM_TYPE_AI: u8 = 2;
const ITEM_AI_FIRST: i32 = 0x87;
const ITEM_SIZE: usize = 0x28;
/// `SAVE_ITEM_COUNT`: the item table's entries.
pub const ITEM_COUNT: usize = 350;
/// `ITEMSET_USABLE`: the ids of a set `ItemSet_Fit` looks at.
const ITEMSET_USABLE: usize = 7;

/// The menus' view of an item entry (`ItemTblEntry` / `VItemEntry`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemInfo {
    /// The column of its kind icon.
    pub kind: u8,
    pub group: u8,
    /// The details page's picture: its texture, and its cell of the icon sheet.
    pub picture: u8,
    /// How many of a character's item slots it takes.
    pub slots: u8,
    pub flags: i32,
}

impl ItemInfo {
    /// `ItemTbl_GetClass`: 0 / 1 for an item flagged 8 (without / with flag 0x10), else 2.
    pub fn class(&self) -> i32 {
        if self.flags & 8 != 0 {
            i32::from(self.flags & 0x10 != 0)
        } else {
            2
        }
    }
}

/// The character and item tables of common file 4 as the menus read them (`ChrTbl_*`, `ItemTbl_*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChrTable {
    /// Per character id: `costumes` (+0xA) and `baseSlots` (+0xE).
    pub costumes: Vec<i32>,
    pub base_slots: Vec<i32>,
    pub items: Vec<ItemInfo>,
}

impl ChrTable {
    /// The `count` characters and the item table of common file 4.
    pub fn parse(file: &[u8], count: usize) -> Self {
        let section = |n: usize| (u32_at(file, n * 4) & !3) as usize;
        let u16_at = |o: usize| {
            file.get(o..o + 2)
                .map(|b| i32::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let charas = section(1);
        let items = section(2);
        Self {
            costumes: (0..count)
                .map(|id| u16_at(charas + id * 0x3C + 0xA).unwrap_or(1))
                .collect(),
            base_slots: (0..count)
                .map(|id| u16_at(charas + id * 0x3C + 0xE).unwrap_or(0))
                .collect(),
            items: (0..ITEM_COUNT)
                .map(|i| {
                    let e = items + i * ITEM_SIZE;
                    let b = |k: usize| file.get(e + k).copied().unwrap_or(0);
                    ItemInfo {
                        kind: b(0),
                        group: b(1),
                        picture: b(2),
                        slots: b(3),
                        flags: u32_at(file, e + 0x14) as i32,
                    }
                })
                .collect(),
        }
    }

    /// `ChrTbl_GetItemSlots` without a saved level: the character's base slots plus `level`.
    pub fn item_slots(&self, chara: i32, level: i32) -> i32 {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.base_slots.get(c))
            .map_or(0, |&b| b + level)
    }

    /// `ItemSet_Fit`: the items of the first seven ids that fit in `capacity` slots (the rest of the set from the
    /// first that does not is emptied); returns the slots taken and, as `used`, the ids kept plus the slots left.
    pub fn item_set_fit(&self, ids: &mut [u16; 8], capacity: i32) -> (i32, i32) {
        let mut total = 0;
        let mut used = 0;
        for i in 0..ITEMSET_USABLE {
            let id = ids[i];
            if id == 0 {
                continue;
            }
            let slots = self
                .items
                .get(usize::from(id) - 1)
                .map_or(0, |e| i32::from(e.slots));
            let sum = total + slots;
            if sum <= capacity {
                total = sum;
                used = i as i32 + 1;
            } else {
                ids[i..ITEMSET_USABLE].fill(0);
                break;
            }
        }
        (total, used + capacity - total)
    }
}

/// What a member's items add up to in a battle (`ItemSet_GetStats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSetStats {
    /// The four stat changes.
    pub stat: [i32; 4],
    /// The AI type: an AI item's, else the character table's entry.
    pub ai_type: i32,
    /// The ability bits.
    pub ability: [u32; 4],
}

/// `ItemSet_GetStats` on common file 4 (`ChrTblFile`: header word 1 the character table, word 2 the 0x28-byte
/// item entries) for the 1-based item `ids`, as `BtlMember_ApplyItems` calls it: before the member's character is
/// set, so without an AI item the AI type is character 0's. Ids outside the table count as nothing.
pub fn item_set_stats(table: &[u8], ids: &[u16; 8]) -> ItemSetStats {
    let section = |n: usize| (u32_at(table, n * 4) & !3) as usize;
    let s16 = |o: usize| {
        table
            .get(o..o + 2)
            .map_or(0, |b| i32::from(i16::from_le_bytes([b[0], b[1]])))
    };
    let items = section(2);
    let mut out = ItemSetStats::default();
    for &id in ids.iter().filter(|&&id| id != 0) {
        let e = items + (usize::from(id) - 1) * ITEM_SIZE;
        let Some(&kind) = table.get(e) else {
            continue;
        };
        if kind == ITEM_TYPE_AI {
            out.ai_type = i32::from(id) - ITEM_AI_FIRST;
        } else {
            for k in 0..4 {
                out.stat[k] += s16(e + 0xC + k * 2);
                out.ability[k] |= u32_at(table, e + 0x18 + k * 4);
            }
        }
    }
    if out.ai_type == 0 {
        // ChrTblEntry.aiType: the first word of entry 0.
        out.ai_type = u32_at(table, section(1)) as i32;
    }
    out
}
