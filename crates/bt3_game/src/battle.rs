//! `Battle_Main` (src/battle/battle.c) up to the fight: `Battle_Load` reads the fighters' model and motion files
//! behind the loading screen, then the battle sequence (battle_seq.rs) runs the stage intro cuts, the entrance
//! poses with their lines and the ready wait. Each frame the fighters are posed and drawn the way
//! `BtlObjDraw_DrawModel` and VU1 program 0 draw them, after the stage, seen through the demo camera or side 0's
//! fighter camera (`ChrCam_CalcLockOn`), then the HUD (hud.rs).
//!
//! Not ported yet: the fighters' actions in the fight, the hair / cloth chains, the face animation, animated stage
//! objects and most of the HUD.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use crate::eft::{DrawCtx, EffectTri, FxWorld, OrderTable, SetAssets};
use bt3_data::ai_data::{AiData, AiTables};
use bt3_data::cam_anim::CamAnim;
use bt3_data::chara_data::{CharaEntry, CharaTables, CommonTables};
use bt3_data::eft_pack::EffectPack;
use bt3_data::elf::Elf;
use bt3_data::gs::{
    RgbaTexture, decode_clut256_upload, decode_sprite_texture, decode_sprite_texture_raw,
    psmt8_row_from_upload,
};
use bt3_data::model::{CharaModel, ModelPart, TEXTURE_SECTION, tex0_cbp, tex0_tbp};
use bt3_data::motion::{MOTION_COUNT, Motion};
use bt3_data::pack_section;
use bt3_data::stage::StageResource;
use bt3_data::stage_col::{StageCollision, StageNav};
use bt3_data::stage_model::StageModel;
use bt3_data::tex::TextureResource;

use crate::battle_seq::{DemoCam, ScrXfade, Seq, SeqInput};
use crate::btl_ai::{AiMgr, Ctx as AiCtx};
use crate::btl_chr::{Env, SideData};
use crate::btl_input::{InputGate, PadState};
use crate::btl_menu::{MenuDraw, MenuInput, MenuTexts, PauseMenu, Tree};
use crate::btl_mgr::Roster;
use crate::btl_obj::ObjData;
use crate::char_select::BattleSetup;
use crate::fade::Fade;
use crate::font::Font;
use crate::hud::{HUD_FILE, Hud, HudFrame, HudSheet, SHEET_WORDS};
use crate::hud_gauge::{HudTextures, SideGauge};
use crate::loading::Quad;
use crate::rng::{GameRandom, LibcRandom, Vu0Random};
use crate::stage_draw::{StageBatch, draw_stage};
use crate::title::TitleSound;

/// `BTL_FILE_CHARA`: `0x590 + chara * 10 + costume`.
pub const CHARA_FILE: u32 = 0x590;
/// The character's motion table (`slot->file[1]`): `0x598 + chara * 10`.
const MOTION_FILE: u32 = 0x598;
/// `BtlAct_GetIdleMotion` on the ground at full health.
#[cfg(test)]
const IDLE_MOTION: usize = 0;
/// `BtlObj_BindTables`: `mdl.camAnims[0]` (character file section 0x29) and `mdl.unk924`, the talk table
/// (section 0x16, read as `BtlSeqObj.talkTbl`).
const ENTRANCE_CUT_SECTION: usize = 0x29;
const TALK_TABLE_SECTION: usize = 0x16;
/// `BtlObj_BindTables`: `mdl.unkA4` is section 0x2D + `gProgress->language`, which the US game leaves at 0.
const SKILL_TEXT_SECTION: usize = 0x2D;
/// `mdl->unk38` (object + 0x50): the HUD's face pictures.
const FACE_SECTION: usize = 1;
/// `gBattleTimeLimitTbl`: the rule's time limit in seconds (s16, -1 for none), in the executable.
const TIME_LIMIT_TABLE: u32 = 0x2C_3480;
/// `gBtlStatCurve`: 13 rows {value at level 80, at 0, at -20}, in the executable.
const STAT_CURVE: u32 = 0x2E_E7C0;
/// `BtlStage_GetFileMember(idx)`: the stage's intro cuts are stage file sections 3..5.
const STAGE_CUT_SECTION: usize = 3;
/// `BTL_FILE_STAGE`: `0x171 + stage`.
pub const STAGE_FILE: u32 = 0x171;
/// `Battle_ResetWork`: `Bgm_Play(Battle_GetBgm() + 0x10B16)`.
pub const BGM_FIRST: u32 = 0x10B16;
/// `BtlObjLight_Init`: section 1 of common file 2 is the light's texture file.
const COMMON_LIGHT_FILE: u32 = 2;
/// Common file 4: the character table (`ChrTblEntry`, 0x3C bytes, header word 1) and the item table
/// (`ItemTblEntry`, 0x28 bytes, header word 2).
const CHARA_TABLE_FILE: u32 = 4;
/// `ITEMTBL_TYPE_AI` and `ITEMTBL_AI_FIRST`.
const ITEM_TYPE_AI: u8 = 2;
const ITEM_AI_FIRST: i32 = 0x87;
/// `ChrCam_KeepBehindHead` keeps the camera behind this node.
#[cfg(test)]
const HEAD_NODE: u16 = 0x30;
/// `BtlObjLight_Init`'s direction, used until the stage gives one.
const DEFAULT_LIGHT: [f32; 3] = [0.34, -0.006, 0.941];

/// The GS frame the battle draws into (`View_InitLayout`, full screen): XYOFFSET is (2048 - 256, 2048 - 224).
const OFFSET_X: f32 = 2048.0 - 256.0;
const OFFSET_Y: f32 = 2048.0 - 224.0;

/// A PS2 matrix: four rows, applied to row vectors (`v.x * m[0] + v.y * m[1] + v.z * m[2] + v.w * m[3]`).
pub(crate) type Mtx = [[f32; 4]; 4];

pub(crate) const IDENTITY: Mtx = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// What `Battle_Load` leaves for the draw: the fighters' models and their textures as GS colours.
pub struct BattleAssets {
    /// Material textures (bilinear, wrapping: TEX1 0x60, CLAMP_1 0).
    pub base_textures: Vec<RgbaTexture>,
    /// One 256 x 1 row per shading CLUT: the light ramp looked up through it (bilinear, clamped: CLAMP_2 5).
    pub shade_textures: Vec<RgbaTexture>,
    /// The stage texture file, decoded (`StgModel_SetTexture` uploads one at a time to block 0x2A00).
    pub stage_textures: Vec<RgbaTexture>,
    /// The effect textures of the fighters' emitter sets (raw GS colours; bilinear, clamped).
    pub effect_textures: Vec<RgbaTexture>,
    pub(crate) fighters: Vec<FighterModel>,
    stage: StageResource,
    stage_model: StageModel,
    stage_id: i32,
    pub(crate) stage_cuts: Vec<Option<CamAnim>>,
    /// `Battle_GetMode`.
    pub(crate) mode: i32,
    /// The five HUD sprite sheets, decoded one after the other (raw GS colours).
    pub hud_textures: Vec<RgbaTexture>,
    pub(crate) hud_sheets: Vec<HudSheet>,
    hud_cpu: HudTextures,
    /// `Battle_GetAnnouncer`.
    announcer: i32,
    /// `Battle_GetTimeLimit` in seconds, -1 for none.
    pub(crate) time_limit: i32,
    /// The roster's tables of common file 2.
    pub(crate) common: CommonTables,
    pub(crate) stat_curve: [[f32; 3]; 13],
    pub(crate) collision: StageCollision,
    /// `Battle_GetSetup()->side[n].cpu`.
    pub(crate) cpu: [bool; 2],
    /// The CPU player's data and the executable's rule tables, the stage's way points.
    pub(crate) ai: AiData,
    pub(crate) ai_tables: AiTables,
    pub(crate) nav: StageNav,
    /// The active members' `cpuLevel` and `aiType`.
    pub(crate) cpu_level: [i32; 2],
    pub(crate) ai_type: [usize; 2],
    /// `Battle_IsSplitScreen`.
    pub(crate) screen_mode: i32,
    /// `Battle_GetBgm`.
    pub(crate) bgm: i32,
    /// The versus pause and result menus (mode 0 only), the menu sheet (HUD file entry 7, in `hud_textures`)
    /// and the menu texts.
    pub(crate) menus: Option<(Tree, Tree)>,
    pub(crate) menu_sheet: HudSheet,
    pub(crate) menu_texts: MenuTexts,
}

pub(crate) struct FighterModel {
    pub(crate) chara: i32,
    model: CharaModel,
    /// Per part, per batch: the material texture and the shading row.
    batch_textures: Vec<Vec<[usize; 2]>>,
    /// Every motion of the character's table by id (`None` for an empty section), with `BtlAnim_Play`'s table
    /// flags applied.
    pub(crate) motions: Vec<Option<Motion>>,
    pub(crate) tables: CharaTables,
    /// `gBtlChars->paramTbl[chara]`.
    pub(crate) entry: CharaEntry,
    /// `mdl.camAnims`: character file sections 0x29..0x2B (entrance, win, lose).
    pub(crate) cam_cuts: [Option<CamAnim>; 3],
    pub(crate) talk: Vec<u8>,
    /// `BtlMember_LoadParams` at the start of the battle.
    pub(crate) gauge: SideGauge,
    /// The character's effect pack (`BtlCharApi_GetPlayerCharPack`) as far as the simulation reads it.
    pub(crate) effect: Option<EffectPack>,
    /// The textures and sprite packs of its two ki blast emitter sets.
    pub(crate) blast_assets: [Option<SetAssets>; 2],
    /// `mdl.unkA4` (object + 0xBC): the pause menu's skill-list script, section 0x2D + language (0 here).
    pub(crate) skill_text: Option<Vec<u16>>,
}

/// Reads what the battle draws so far. `read` returns a file of the AFS archives by id.
pub fn load(
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    elf: &Elf,
    setup: &BattleSetup,
) -> Result<BattleAssets, String> {
    let error = |e: std::io::Error| e.to_string();
    // BtlObjLight_FindRes0 uploads texture 0 of the light file to block 0x3D40; the models' TEX0_2 reads that
    // block as a 256-wide PSMT8 texture (TBW 4) on its first row.
    let common = read(COMMON_LIGHT_FILE)?;
    let light = TextureResource::parse(pack_section(&common, 1).map_err(error)?).map_err(error)?;
    let ramp_entry = light
        .entries
        .first()
        .ok_or("the light file has no textures")?;
    let ramp = psmt8_row_from_upload(ramp_entry, 4, 0, 256).map_err(error)?;
    let stage_file = read(STAGE_FILE + setup.stage.max(0) as u32)?;
    let stage = StageResource::parse(&stage_file).map_err(error)?;
    let object = StageModel::object(&stage_file).map_err(error)?;
    let stage_model = StageModel::parse(object).map_err(error)?;
    let stage_textures = TextureResource::parse_at(object, stage_model.texture_header)
        .map_err(error)?
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let stage_cuts = (0..3)
        .map(|i| {
            pack_section(&stage_file, STAGE_CUT_SECTION + i)
                .ok()
                .filter(|s| !s.is_empty())
                .map(CamAnim::parse)
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let hud_file = read(HUD_FILE)?;
    let (mut hud_textures, hud_sheets, hud_cpu) = load_hud(&hud_file).map_err(error)?;
    let menu_sheet = load_menu_sheet(&hud_file, &mut hud_textures).map_err(error)?;
    let menu_texts = MenuTexts::read(&hud_file, MENU_TEXT_COUNT);
    // PauseMenu_GetMenu: the versus trees without a replay; the other modes' menus are not ported.
    let menus = if setup.mode == 0 {
        Some((
            Tree::read(elf, crate::btl_menu::PAUSE_VERSUS).map_err(error)?,
            Tree::read(elf, crate::btl_menu::RESULT_VERSUS).map_err(error)?,
        ))
    } else {
        None
    };
    let common_tables = CommonTables::parse(&common).map_err(error)?;
    let collision = StageCollision::parse(&stage_file).map_err(error)?;
    let nav = StageNav::parse(&stage_file).map_err(error)?;
    let ai = AiData::parse(&common).map_err(error)?;
    let ai_tables = AiTables::read(elf).map_err(error)?;
    let chara_table = read(CHARA_TABLE_FILE)?;
    let ai_type = setup
        .sides
        .map(|side| member_ai_type(&chara_table, &side.items));
    let curve = elf.f32s(STAT_CURVE, 39).map_err(error)?;
    let stat_curve = std::array::from_fn(|r| [curve[r * 3], curve[r * 3 + 1], curve[r * 3 + 2]]);
    let limits = elf.read(TIME_LIMIT_TABLE, 16).map_err(error)?;
    let time_limit = usize::try_from(setup.time_limit)
        .ok()
        .and_then(|i| limits.get(i * 2..i * 2 + 2))
        .map_or(0, |b| i32::from(i16::from_le_bytes([b[0], b[1]])));
    let mut assets = BattleAssets {
        base_textures: Vec::new(),
        shade_textures: Vec::new(),
        stage_textures,
        effect_textures: Vec::new(),
        fighters: Vec::new(),
        stage,
        stage_model,
        stage_id: setup.stage,
        stage_cuts,
        mode: setup.mode,
        hud_textures,
        hud_sheets,
        hud_cpu,
        announcer: setup.announcer,
        time_limit,
        common: common_tables,
        stat_curve,
        collision,
        cpu: [setup.sides[0].control == 2, setup.sides[1].control == 2],
        ai,
        ai_tables,
        nav,
        cpu_level: setup.sides.map(|side| side.cpu_level),
        ai_type,
        screen_mode: setup.screen_mode,
        bgm: setup.bgm,
        menus,
        menu_sheet,
        menu_texts,
    };
    for side in setup.sides {
        let file_id = CHARA_FILE + side.chara.max(0) as u32 * 10 + side.costume.max(0) as u32;
        let file = read(file_id)?;
        let mut fighter = load_fighter(&file, &ramp, &mut assets)
            .map_err(|e| format!("character file 0x{file_id:X}: {e}"))?;
        fighter.chara = side.chara;
        for (i, cut) in fighter.cam_cuts.iter_mut().enumerate() {
            *cut = pack_section(&file, ENTRANCE_CUT_SECTION + i)
                .ok()
                .filter(|s| !s.is_empty())
                .map(CamAnim::parse)
                .transpose()
                .map_err(error)?;
        }
        fighter.talk = pack_section(&file, TALK_TABLE_SECTION)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        fighter.skill_text = pack_section(&file, SKILL_TEXT_SECTION).ok().map(|s| {
            s.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect()
        });
        fighter.gauge = start_gauge(&common, side.chara);
        // BtlObj_BindTables: object + 0x50 is section 1; HudGauge_DrawFace draws its texture `side`.
        let index = assets.fighters.len();
        assets.hud_cpu.faces[index] = pack_section(&file, FACE_SECTION)
            .ok()
            .and_then(|s| TextureResource::parse(s).ok())
            .and_then(|res| {
                let entry = res.entries.get(index)?;
                decode_sprite_texture_raw(entry, entry).ok()
            });
        fighter.tables = CharaTables::parse(&file).map_err(error)?;
        fighter.entry = assets
            .common
            .charas
            .get(side.chara.max(0) as usize)
            .copied()
            .unwrap_or_default();
        // BtlRes_Request: the second animation file is chara * 10 + 0x599.
        fighter.effect = EffectPack::parse(&read(MOTION_FILE + 1 + side.chara.max(0) as u32 * 10)?);
        if let Some(pack) = &fighter.effect {
            for (list, set) in pack.blast_sets.iter().enumerate() {
                if let Some(set) = set {
                    fighter.blast_assets[list] =
                        Some(SetAssets::load(set, &mut assets.effect_textures).map_err(error)?);
                }
            }
        }
        let motions = read(MOTION_FILE + side.chara.max(0) as u32 * 10)?;
        let word = |n: usize| {
            motions
                .get(n * 4..n * 4 + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) & !3)
        };
        for id in 0..MOTION_COUNT {
            // BObjFile_GetEntry: equal offsets mean the section is empty.
            let (Some(start), Some(end)) = (word(id + 1), word(id + 2)) else {
                fighter.motions.push(None);
                continue;
            };
            if start == end {
                fighter.motions.push(None);
                continue;
            }
            let packed = pack_section(&motions, id + 1).map_err(error)?;
            let raw = bt3_data::bpe::decode(packed).map_err(error)?;
            // Some sections are streams that decode to nothing (motions the character never plays).
            if raw.is_empty() {
                fighter.motions.push(None);
                continue;
            }
            let mut motion = Motion::parse(&raw).map_err(error)?;
            // BtlAnim_Play: BtlAnim_ApplyTableFlags and BtlAnim_HideModelNodes.
            motion.apply_table_flags(motion_flags(&common, id));
            motion.hide_model_nodes(side.chara);
            fighter.motions.push(Some(motion));
        }
        assets.fighters.push(fighter);
    }
    Ok(assets)
}

/// Entries of the HUD file's offset table read as menu texts (`BtlMenu_GetText`).
const MENU_TEXT_COUNT: usize = 0x40;
/// `BtlMenu_Init`: the menu sheet is the texture file at HUD file word 7.
const MENU_SHEET_WORD: usize = 7;

/// The menu sheet, appended to the HUD textures.
fn load_menu_sheet(file: &[u8], textures: &mut Vec<RgbaTexture>) -> std::io::Result<HudSheet> {
    let start = file
        .get(MENU_SHEET_WORD * 4..MENU_SHEET_WORD * 4 + 4)
        .map_or(0, |b| {
            u32::from_le_bytes(b.try_into().unwrap()) as usize & !3
        });
    let res = TextureResource::parse(file.get(start..).unwrap_or_default())?;
    let mut sheet = HudSheet {
        base: textures.len(),
        sizes: Vec::new(),
        views: Vec::new(),
    };
    for entry in &res.entries {
        let tex0 = entry.texture_register;
        sheet
            .sizes
            .push([1 << ((tex0 >> 26) & 0xF), 1 << ((tex0 >> 30) & 0xF)]);
        textures.push(decode_sprite_texture(entry).unwrap_or(RgbaTexture {
            width: 1,
            height: 1,
            pixels: vec![0; 4],
        }));
    }
    // The skill list's page tabs (entry 3) and page bar (entry 4) with the palettes of entries 0x26 + page.
    for tex in [3, 4] {
        for clut in 0x26..0x30 {
            if let (Some(t), Some(c)) = (res.entries.get(tex), res.entries.get(clut))
                && let Ok(texture) = bt3_data::gs::decode_sprite_texture_with_clut(t, c)
            {
                textures.push(texture);
                sheet.views.push(((tex, clut), textures.len() - 1));
            }
        }
    }
    Ok(sheet)
}

/// `BtlMember_ApplyItems`' AI type (`ItemSet_GetStats`): the last AI item's, else the character table's entry of
/// character 0 (the member's character is still 0 when the items are applied).
fn member_ai_type(table: &[u8], items: &[u16; 8]) -> usize {
    let word = |o: usize| {
        table
            .get(o..o + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let item_base = (word(8) & !3) as usize;
    let mut kind = 0;
    for &id in items.iter().filter(|&&id| id != 0) {
        let entry = item_base + (usize::from(id) - 1) * 0x28;
        if table.get(entry) == Some(&ITEM_TYPE_AI) {
            kind = i32::from(id) - ITEM_AI_FIRST;
        }
    }
    if kind == 0 {
        kind = word((word(4) & !3) as usize) as i32;
    }
    kind.max(0) as usize
}

/// `Hud_Init`'s sheets. Entries with no image (palettes only) become a transparent texel. Sheet 0 is also kept
/// with raw GS alpha for the CPU draw, with each image under the palettes of the entries after it.
fn load_hud(file: &[u8]) -> std::io::Result<(Vec<RgbaTexture>, Vec<HudSheet>, HudTextures)> {
    let word = |i: usize| {
        file.get(i * 4..i * 4 + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let mut textures = Vec::new();
    let mut sheets = Vec::new();
    let mut cpu = HudTextures::default();
    for (index, header) in SHEET_WORDS.into_iter().enumerate() {
        let start = (word(header) & !3) as usize;
        let res = TextureResource::parse(file.get(start..).unwrap_or_default())?;
        let mut sheet = HudSheet {
            base: textures.len(),
            sizes: Vec::new(),
            views: Vec::new(),
        };
        for (tex, entry) in res.entries.iter().enumerate() {
            let tex0 = entry.texture_register;
            sheet
                .sizes
                .push([1 << ((tex0 >> 26) & 0xF), 1 << ((tex0 >> 30) & 0xF)]);
            textures.push(decode_sprite_texture(entry).unwrap_or(RgbaTexture {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }));
            if index == 0 {
                for sub in 0..=HUD_PALETTE_SUBS {
                    let Some(clut) = res.entries.get(tex + usize::from(sub)) else {
                        break;
                    };
                    if let Ok(decoded) = decode_sprite_texture_raw(entry, clut) {
                        cpu.sheet0.insert((tex as i16, sub), decoded);
                    }
                }
            }
        }
        sheets.push(sheet);
    }
    Ok((textures, sheets, cpu))
}

/// The palette offsets the gauges and the clock use (`HudSprite_SetTex(spr, 8, 1..5)`).
const HUD_PALETTE_SUBS: u16 = 5;

/// `BtlChar_AllocAll`: `gBtlChars->tbl[4]`, the table at header word 8 of common file 2, four words per
/// character: health max, ki at the start, ?, blast stocks. `BtlMember_LoadParams` with 100 % health and no
/// abilities.
fn start_gauge(common: &[u8], chara: i32) -> SideGauge {
    let word = |o: usize| {
        common
            .get(o..o + 4)
            .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
    };
    let entry = (word(8 * 4) as usize & !3) + chara.max(0) as usize * 16;
    let health_max = word(entry).max(10000);
    SideGauge {
        hp: health_max,
        ki: word(entry + 4).min(100000),
        blast: 0,
        blast_max: word(entry + 12) * 100000,
        max_power: 0,
        powered_up: false,
        dead: false,
    }
}

/// `BtlAnim_GetFlags`: the roster's motion flag table is the table at header word 3 of common file 2.
fn motion_flags(common: &[u8], motion: usize) -> u32 {
    let word = |o: usize| {
        common
            .get(o..o + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    word((word(12) as usize & !3) + motion * 4)
}

/// `BtlObjMdl_BindFiles` and `ObjDraw_UploadTextures`: a batch's TEX0_1 is its material texture at
/// `tbp 0x3480 + tbpOfs`, `cbp 0x3C00 + cbpOfs` once `MdlTex_RebaseChain` moved it, and its TEX0_2 reads the
/// CLUT that `TexFile_UploadRange(clutFirst, clutCount)` put at `0x3D00 + tbpOfs`.
fn load_fighter(
    file: &[u8],
    ramp: &[u8],
    assets: &mut BattleAssets,
) -> Result<FighterModel, String> {
    let error = |e: std::io::Error| e.to_string();
    let model = CharaModel::from_file(file).map_err(error)?;
    let textures = TextureResource::parse(pack_section(file, TEXTURE_SECTION).map_err(error)?)
        .map_err(error)?;
    let header = &model.header;
    let entry = |i: u8| {
        textures
            .entries
            .get(usize::from(i))
            .ok_or_else(|| format!("texture {i} is missing"))
    };
    let mut materials = HashMap::new();
    for i in header.tex_first..header.tex_first.saturating_add(header.tex_count) {
        let e = entry(i)?;
        let mut texture = decode_sprite_texture(e).map_err(error)?;
        // TEX0_1 is MODULATE with alpha blending off (ALPHA_1 0x64 takes the source colour as it is).
        for pixel in texture.pixels.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        materials.insert(
            (e.tbp_offset as u32, e.cbp_offset as u32),
            assets.base_textures.len(),
        );
        assets.base_textures.push(texture);
    }
    let mut shades = HashMap::new();
    for i in header.clut_first..header.clut_first.saturating_add(header.clut_count) {
        let e = entry(i)?;
        let clut = decode_clut256_upload(e).map_err(error)?;
        let pixels = ramp
            .iter()
            .flat_map(|&index| {
                let [r, g, b, _] = clut[usize::from(index)];
                [r, g, b, 0xFF]
            })
            .collect();
        shades.insert(e.tbp_offset as u32, assets.shade_textures.len());
        assets.shade_textures.push(RgbaTexture {
            width: ramp.len() as u32,
            height: 1,
            pixels,
        });
    }
    let batch_textures = model
        .parts
        .iter()
        .map(|part| {
            part.batches
                .iter()
                .map(|batch| {
                    let key = (
                        tex0_tbp(batch.tex0[0]) - model.min_tbp,
                        tex0_cbp(batch.tex0[0]) - model.min_cbp,
                    );
                    let base = *materials
                        .get(&key)
                        .ok_or("a batch names no material texture")?;
                    let shade = *shades
                        .get(&tex0_cbp(batch.tex0[1]))
                        .ok_or("a batch names no shading CLUT")?;
                    Ok([base, shade])
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(FighterModel {
        chara: 0,
        model,
        batch_textures,
        motions: Vec::new(),
        tables: CharaTables::default(),
        entry: CharaEntry::default(),
        cam_cuts: [None, None, None],
        talk: Vec::new(),
        gauge: SideGauge::default(),
        effect: None,
        blast_assets: [None, None],
        skill_text: None,
    })
}

/// One vertex as the GS gets it from program 0: screen position (pixels of the 512 x 448 frame) and Z, the
/// model texture's (s, t, 1) * q and the ramp's u * q.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenVertex {
    pub position: [f32; 3],
    pub stq: [f32; 3],
    pub uq: f32,
}

/// Triangles drawn with one material texture and one shading row.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelBatch {
    pub textures: [usize; 2],
    /// Three vertices per triangle.
    pub vertices: Vec<ScreenVertex>,
}

/// What one battle frame draws, in `Battle_Draw`'s order: the stage, then the fighters, then the overlay pass
/// (the cross-fade) and the fade slot.
pub struct BattleFrame {
    pub stage: Vec<StageBatch>,
    pub models: Vec<ModelBatch>,
    /// `ScrXfade_PostDraw`: keep this frame to fade it out over the next ones.
    pub capture: bool,
    /// Alpha (0..128) of the kept frame over this one.
    pub xfade_alpha: i32,
    /// `Fade_DrawScreen` (slot 0).
    pub quads: Vec<Quad>,
    /// `BtlGame_Draw`: the HUD, drawn into the frame before the overlay pass.
    pub hud: HudFrame,
    pub sounds: Vec<TitleSound>,
    /// The battle is over: the result's reason bits (`BattleResult_GetReason`), which pick the next screen.
    pub exit: Option<u32>,
    /// `BtlScene_Draw` and `Ot_Draw`: the effects' triangles in the order the GS draws them, after the fighters.
    pub effects: Vec<EffectTri>,
}

/// The battle as far as it is ported.
pub struct Battle {
    pub assets: BattleAssets,
    /// `gBtlChars`.
    pub(crate) roster: Roster,
    light_dir: [f32; 3],
    pub(crate) seq: Seq,
    pub(crate) demo: DemoCam,
    pub(crate) xfade: ScrXfade,
    pub(crate) hud: Hud,
    /// `Fade_Start(0, 1, 1.0)` of `Battle_Restart`.
    pub(crate) fade: Fade,
    /// Node 0's world matrix of each fighter from the last pose (`DemoCam_SetBase` reads it before the new pose).
    roots: [Mtx; 2],
    started: bool,
    /// `gBtlAi`.
    ai: AiMgr,
    /// `gBtlMenu` with the versus menus.
    pub(crate) menu: Option<PauseMenu>,
    /// Last frame's buttons per pad, for the menus' pressed words.
    prev_held: [u32; 2],
    /// The VU0 R register: seeded at boot and shared by every battle (the game moves it in and out).
    pub vu0: Vu0Random,
    /// `Pad.gameRepeatTimer` / `gameRepeatLast` of both pads.
    pad_repeat: [(i32, u32); 2],
    /// The save's `charaBits` (the skill list's unlock tests); the game hands its save in.
    pub chara_bits: [u64; 3],
}

/// What a side's files give the fighter simulation.
pub(crate) fn side_data(f: &FighterModel) -> SideData<'_> {
    SideData {
        obj: ObjData {
            model: &f.model,
            motions: &f.motions,
            body: f.tables.body.as_ref(),
        },
        tables: &f.tables,
        entry: f.entry,
        effect: f.effect.as_ref(),
        blast_assets: std::array::from_fn(|i| f.blast_assets[i].as_ref()),
    }
}

impl BattleAssets {
    /// What the fighters share: the roster's tables, the stage, the sequence state and the input gate.
    pub(crate) fn env(&self, seq_state: i32, gate: InputGate) -> Env<'_> {
        Env {
            common: &self.common,
            stat_curve: &self.stat_curve,
            stage: &self.stage,
            col: &self.collision,
            mode: self.mode,
            stage_4_or_27: matches!(self.stage_id, 4 | 27),
            seq_state,
            gate,
        }
    }
}

impl Battle {
    pub fn new(assets: BattleAssets) -> Self {
        // BtlObj_UpdateAll: BtlObjLight_SetDir(BtlStage_GetLightVecB), which normalises it.
        let light_dir = normalize(assets.stage.light_dir.unwrap_or(DEFAULT_LIGHT));
        let seq = Seq::new(assets.mode, assets.time_limit);
        let roster = {
            let data = [
                side_data(&assets.fighters[0]),
                side_data(&assets.fighters[1]),
            ];
            let env = assets.env(seq.state_id(), seq.input_gate(assets.stage_id));
            Roster::new([&data[0], &data[1]], &env, assets.cpu)
        };
        let mut roster = roster;
        let ai = {
            let data = [
                side_data(&assets.fighters[0]),
                side_data(&assets.fighters[1]),
            ];
            let env = assets.env(seq.state_id(), seq.input_gate(assets.stage_id));
            // Resetting the sides draws no random number.
            let mut unused = GameRandom::from_key(&[0]);
            let mut no_blasts = Vec::new();
            let mut ctx = AiCtx {
                chars: &mut roster.chars,
                data: [&data[0], &data[1]],
                env: &env,
                ai: &assets.ai,
                tab: &assets.ai_tables,
                nav: &assets.nav,
                rng: &mut unused,
                blasts: &mut no_blasts,
            };
            AiMgr::new(&mut ctx, assets.cpu_level, assets.ai_type)
        };
        let roots = std::array::from_fn(|side| roster.chars[side].obj.mtx);
        let hud = Hud::new(&assets.hud_sheets, assets.hud_cpu.clone(), assets.announcer);
        // PauseMenu_Init; BtlGame_Reset closes both trees.
        let menu = assets.menus.clone().map(|(pause, result)| {
            let mut menu = PauseMenu::new(pause, result);
            menu.reset();
            menu.scripts = std::array::from_fn(|side| assets.fighters[side].skill_text.clone());
            menu
        });
        Self {
            assets,
            roster,
            light_dir,
            seq,
            demo: DemoCam::default(),
            xfade: ScrXfade::default(),
            hud,
            fade: Fade::start_in(1.0),
            roots,
            started: false,
            ai,
            menu,
            prev_held: [0; 2],
            vu0: Vu0Random::default(),
            pad_repeat: [(0, 0); 2],
            chara_bits: crate::save::SaveData::default().chara_bits,
        }
    }

    /// `Battle_Restart` for a rematch: `Battle_ResetWork` (sounds stopped, the battle music again, the result
    /// cleared), `StgFx_Reset` (the cross-fade), `BtlGame_Reset` (the menus, the HUD, the sequence),
    /// `BtlChar_ResetAll` and the 1 s fade-in. The AI is not reset, as in the original.
    fn restart(&mut self, sounds: &mut Vec<TitleSound>) {
        sounds.push(TitleSound::VoiceStop);
        sounds.push(TitleSound::Bgm(BGM_FIRST + self.assets.bgm.max(0) as u32));
        self.xfade = ScrXfade::default();
        if let Some(menu) = &mut self.menu {
            menu.reset();
        }
        self.hud = Hud::new(
            &self.assets.hud_sheets,
            self.assets.hud_cpu.clone(),
            self.assets.announcer,
        );
        self.seq = Seq::new(self.assets.mode, self.assets.time_limit);
        self.demo = DemoCam::default();
        self.roster = {
            let data = [
                side_data(&self.assets.fighters[0]),
                side_data(&self.assets.fighters[1]),
            ];
            let env = self.assets.env(
                self.seq.state_id(),
                self.seq.input_gate(self.assets.stage_id),
            );
            Roster::new([&data[0], &data[1]], &env, self.assets.cpu)
        };
        self.roots = std::array::from_fn(|side| self.roster.chars[side].obj.mtx);
        self.fade = Fade::start_in(1.0);
        self.started = false;
    }

    /// The fighter phases with both sides' data and the shared state.
    fn with_roster(&mut self, f: impl FnOnce(&mut Roster, [&SideData; 2], &Env) -> bool) -> bool {
        let data = [
            side_data(&self.assets.fighters[0]),
            side_data(&self.assets.fighters[1]),
        ];
        let env = self.assets.env(
            self.seq.state_id(),
            self.seq.input_gate(self.assets.stage_id),
        );
        f(&mut self.roster, [&data[0], &data[1]], &env)
    }

    /// One iteration of `Battle_Loop`: `Gfx_BeginFrame` (the fade), `BtlChars_CheckStart`, the sequence's
    /// preUpdate, `BtlChars_SampleInput`, `Battle_Update` (the fighters' phases, the cameras with the demo camera
    /// last), the sequence's update, then `Battle_Draw`: the stage, the fighters, the HUD, the overlay pass and the
    /// fade. `start` is START going down on a pad; `pads` the two pads; `voice_stat` the voice player's status;
    /// `shown` whether the frame is the one presented (an unseen frame skips the HUD's pixels).
    pub fn run_frame(
        &mut self,
        start: bool,
        pads: [PadState; 2],
        voice_stat: i32,
        libc: &mut LibcRandom,
        random: &mut GameRandom,
        mut font: Option<&mut Font>,
        shown: bool,
    ) -> BattleFrame {
        let mut sounds = Vec::new();
        // Battle_Loop: a restart asked for last frame happens first.
        if std::mem::take(&mut self.seq.restart) {
            self.restart(&mut sounds);
        }
        let menu = std::array::from_fn(|i| {
            use crate::btl_input::pad::{CROSS, DOWN, LEFT, RIGHT, START, TRIANGLE, UP};
            let (held, prev) = (pads[i].held, self.prev_held[i]);
            let down = held & !prev;
            // Pad_CalcRepeat with the battle's Pad_SetRepeat(0x14, 1).
            let (timer, last) = &mut self.pad_repeat[i];
            let mut repeat = 0;
            if held == *last && held != 0 {
                *timer -= 1;
                if *timer < 0 {
                    *timer = 1;
                    repeat = held;
                }
            } else {
                *last = held;
                *timer = 0x14;
            }
            if down != 0 {
                repeat = down;
            }
            MenuInput {
                up: down & UP != 0,
                down: down & DOWN != 0,
                cross: down & CROSS != 0,
                triangle: down & TRIANGLE != 0,
                start: down & START != 0,
                rep_up: repeat & UP != 0,
                rep_down: repeat & DOWN != 0,
                rep_left: repeat & LEFT != 0,
                rep_right: repeat & RIGHT != 0,
            }
        });
        self.prev_held = pads.map(|p| p.held);
        let mut input = SeqInput {
            start,
            voice_stat,
            libc: &mut *libc,
            sounds: &mut sounds,
            menu,
            font: font.as_deref(),
        };
        if !std::mem::replace(&mut self.started, true) {
            self.seq_start(&mut input);
        }
        self.fade.update();
        if self.with_roster(|roster, data, env| roster.check_start(data, env)) {
            self.seq.started = true;
        }
        let sides = [
            self.roster.chars[0].hud_gauge(),
            self.roster.chars[1].hud_gauge(),
        ];
        self.hud.pre_update(self.seq.time_left(), &sides);
        self.seq_pre_update(&mut input);
        // BATTLE_FLAG_PAUSE as the sequence left it last frame.
        let paused = self.seq.paused;
        if !paused {
            // BtlAiMgr_Update, after Battle_UpdateWork and before the fighters sample their input.
            let data = [
                side_data(&self.assets.fighters[0]),
                side_data(&self.assets.fighters[1]),
            ];
            let env = self.assets.env(
                self.seq.state_id(),
                self.seq.input_gate(self.assets.stage_id),
            );
            let mut ctx = AiCtx {
                chars: &mut self.roster.chars,
                data: [&data[0], &data[1]],
                env: &env,
                ai: &self.assets.ai,
                tab: &self.assets.ai_tables,
                nav: &self.assets.nav,
                rng: &mut *random,
                blasts: &mut self.roster.scene.records,
            };
            self.ai.update(&mut ctx);
        }
        let mut vu0 = self.vu0;
        self.with_roster(|roster, data, env| {
            // BtlChars_SampleInput, UpdateInput, UpdateMain and PostScene do nothing while paused; the effect
            // scene runs with its time stopped.
            if !paused {
                roster.sample_input(&pads, env);
                roster.update_input(data, env);
                roster.update_main(data, env);
            }
            roster.update_scene(data, env, &mut *input.libc, &mut vu0, paused);
            if !paused {
                roster.post_scene(data, env);
            }
            false
        });
        self.vu0 = vu0;
        let cam = self.roster.chars[0].cam;
        let mut view = View::new(cam.eye, cam.rot);
        if self.demo.is_active() {
            let base = self.demo.attached().map(|side| self.roots[side]);
            if let Some(demo) = self.demo.update(base, paused) {
                view = demo;
            }
        }
        if !paused {
            self.roster.end_frame();
        }
        for side in 0..2 {
            if let Some(n) = self.roster.chars[side].obj.node(0) {
                self.roots[side] = n.mtx;
            }
        }
        // What the skill lists' line tests read (BtlText_CheckUnlock).
        if let Some(menu) = &mut self.menu {
            for side in 0..2 {
                menu.text_ctx[side] = crate::btl_text::TextCtx {
                    pad_type: 0,
                    chara_bits: self.chara_bits,
                    body_changed: self.roster.chars[side].gauge.body_changed,
                };
            }
        }
        self.seq_update(&mut input);
        // FontIcon_ResetAnim from the skill list's cursor.
        if let Some(menu) = &mut self.menu
            && std::mem::take(&mut menu.reset_anim)
            && let Some(font) = font.as_deref_mut()
        {
            font.reset_icon_anim();
        }
        let stage = draw_stage(&mut self.assets.stage_model, self.assets.stage_id, &view);
        let mut groups: HashMap<[usize; 2], Vec<ScreenVertex>> = HashMap::new();
        for side in 0..2 {
            let model = &self.assets.fighters[side];
            let nodes = &self.roster.chars[side].obj.nodes;
            let parts = model
                .model
                .parts
                .iter()
                .zip(&model.batch_textures)
                .zip(nodes);
            for ((part, textures), node) in parts {
                if part.enabled && node_shown(model.model.header.flags, part.node) {
                    draw_part(
                        part,
                        textures,
                        &node.mtx,
                        &node.parent,
                        self.light_dir,
                        &view,
                        &mut groups,
                    );
                }
            }
        }
        let mut batches: Vec<ModelBatch> = groups
            .into_iter()
            .map(|(textures, vertices)| ModelBatch { textures, vertices })
            .collect();
        batches.sort_by_key(|batch| batch.textures);
        // BtlScene_Draw(1), then Ot_Draw.
        let effects = {
            let ctx = DrawCtx::new(view.world2screen, view.world2view);
            let mut ot = OrderTable::new();
            let fxw = FxWorld {
                sets: std::array::from_fn(|s| {
                    std::array::from_fn(|l| self.assets.fighters[s].blast_assets[l].as_ref())
                }),
                any_frozen: self.roster.chars.iter().any(|c| c.is_frozen()),
            };
            // BtlCharApi_IsCamShown without camera priorities: the demo camera shows both, else side 0's view.
            let demo = self.demo.is_active();
            self.roster
                .scene
                .fx
                .draw(&fxw, &mut ot, &ctx, |chr| demo || chr == 0);
            ot.into_list()
        };
        self.hud.set_paused(self.seq.paused);
        let mut hud = self.hud.draw(libc, shown);
        // BtlGame_Draw: the menu over the HUD.
        if let Some(menu) = &self.menu {
            menu.draw(
                &mut MenuDraw {
                    font: font.as_deref_mut(),
                    sheet: &self.assets.menu_sheet,
                    texts: &self.assets.menu_texts,
                },
                &mut hud.prims,
            );
        }
        let (capture, xfade_alpha) = self.xfade.draw_pass();
        BattleFrame {
            stage,
            models: batches,
            capture,
            xfade_alpha,
            quads: self.fade.quad().into_iter().collect(),
            hud,
            sounds,
            exit: self.seq.left.then_some(self.seq.reason),
            effects,
        }
    }
}

/// `BtlObj_IsNodeShown` with all of the object's part flags clear (`visible && partN` never holds): the
/// alternative parts that need a flag stay hidden.
fn node_shown(model_flags: u32, node: u16) -> bool {
    const PARTS_A: u32 = 0x1_0000;
    const PARTS_B: u32 = 0x2_0000;
    const PARTS_C: u32 = 0x4_0000;
    if model_flags & PARTS_A != 0 {
        !matches!(node, 0x42 | 0x43)
    } else if model_flags & PARTS_B != 0 {
        node != 0x40
    } else if model_flags & PARTS_C != 0 {
        !matches!(node, 0x40 | 0x41)
    } else {
        !matches!(node, 0x40 | 0x41 | 0x6F)
    }
}

/// What `ChrCam_CalcLockOn` reads.
pub(crate) struct LockOn {
    pub pos: [f32; 3],
    pub height: f32,
    pub side_slope: f32,
    pub opp_pos: [f32; 3],
    pub opp_height: f32,
    pub stage_bottom: f32,
    pub yaw: f32,
    pub side: f32,
}

/// `ChrCam_GetOffset` for distance preset 0: the offset behind the fighter and the pitch.
pub(crate) fn camera_offset(height: f32) -> ([f32; 3], f32) {
    let dist = (height * 0.5 + 1.0).max(10.0);
    ([0.0, -dist * 1.2, -dist * 2.4], height * -0.001 + -0.05)
}

/// `ChrCam_CalcLockOn`: returns the eye, the rotation (pitch, yaw, 0) and the target.
pub(crate) fn lock_on_camera(cam: &LockOn) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let scale = cam.height;
    let mut pos = cam.pos;
    let floor_y = cam.stage_bottom + scale * 0.5;
    if floor_y < pos[1] {
        pos[1] = floor_y;
    }
    let mut to_opp = sub(cam.opp_pos, pos);
    to_opp[1] += cam.height * 0.5 - cam.opp_height * 0.5;
    let len = length(to_opp);
    let dir = if len < 0.01 {
        [0.0, 0.0, 1.0]
    } else {
        scale3(to_opp, 1.0 / len)
    };
    let mut target = [0.0, -(scale * 0.7), 0.0];
    let (ofs, mut base) = camera_offset(scale);
    let mut ofs = sub(ofs, target);
    let mut u = if len < 0.0 {
        0.0
    } else if len < 100.0 {
        len / 100.0
    } else {
        1.0
    };
    u = u * 2.0 - 1.0;
    u = -u * u.abs() + u * 2.0;
    u = u * 0.5 + 0.5;
    u = u * 0.2 + 0.8;
    ofs = scale3(ofs, u);
    target = scale3(target, 1.0 - (1.0 - u) * 0.5);
    let max_pitch = PI * 0.35;
    let mut a = (-dir[1].clamp(-1.0, 1.0).asin() / max_pitch).clamp(-1.0, 1.0);
    let mut b = a.max(0.0);
    b = b * 2.0 - 1.0;
    a = -a * a.abs() + a * 2.0;
    b = -b * b.abs() + b * 2.0;
    let pitch = max_pitch * a * 0.4;
    b = b * 0.5 + 0.5;
    ofs = rotate_x(ofs, pitch);
    ofs = scale3(ofs, 1.0 - b * 0.3);
    base += pitch - base * b;
    let limit = cam.side_slope.atan().max(0.785_398_2);
    let horiz = to_opp[0].hypot(to_opp[2]);
    let ofs_len = ofs[0].hypot(ofs[2]);
    let fov = (512.0f32 * 0.5 / 433.0).atan() * 2.0;
    let half = fov * 0.7;
    let tn = half.tan();
    let spread = if 0.01 < horiz {
        (tn * (ofs_len / horiz)).atan()
    } else if 0.0 < tn * ofs_len * horiz {
        FRAC_PI_2
    } else {
        -FRAC_PI_2
    };
    let side = cam.side / limit;
    let mut turn = half + spread * (limit - fov) / limit;
    ofs = rotate_y(ofs, turn * side);
    turn += spread * (limit / FRAC_PI_2);
    turn *= 0.5;
    turn *= side;
    let yaw = wrap_angle(cam.yaw - turn);
    target = add(target, pos);
    let (s, c) = cam.yaw.sin_cos();
    let out = [-c * ofs[0] + s * ofs[2], ofs[1], s * ofs[0] + c * ofs[2]];
    (add(target, out), [base, yaw, 0.0], target)
}

/// `ChrCam_KeepBehindHead`: pushes the eye back onto a vertical plane behind the head.
pub(crate) fn keep_behind_head(node: [f32; 3], height: f32, eye: &mut [f32; 3], rot: [f32; 3]) {
    let margin = height * 0.15;
    let dir = [rot[1].sin(), 0.0, rot[1].cos()];
    let w = -dot(dir, node) + margin * (rot[0].cos() + 1.0) + 10.0;
    let d = dot(dir, *eye) + w;
    if 0.0 < d {
        *eye = sub(*eye, scale3(dir, d));
    }
}

/// `View_SetTransform` + `View_UpdateMatrices` with `View_InitLayout(VIEW_LAYOUT_FULL)`'s projection.
pub(crate) struct View {
    /// `view->view` (+0x40): world to view.
    pub(crate) world2view: Mtx,
    pub(crate) world2screen: Mtx,
    pub(crate) world2clip: Mtx,
    /// `view->clip2screen` (+0x100), what `StgModel_Draw` loads program 4 with.
    pub(crate) clip2screen: Mtx,
}

impl View {
    fn new(pos: [f32; 3], rot: [f32; 3]) -> Self {
        Self::from_world_to_view(view_matrix(pos, rot))
    }

    /// `View_UpdateMatrices` for a given world-to-view matrix, with the full-screen projection.
    pub(crate) fn from_world_to_view(world2view: Mtx) -> Self {
        let (view2screen, view2clip, clip2screen) = projection();
        Self {
            world2view,
            world2screen: mul(&world2view, &view2screen),
            world2clip: mul(&world2view, &view2clip),
            clip2screen,
        }
    }
}

/// `Mtx_InverseRT`: the inverse of a rotation + translation matrix.
pub(crate) fn inverse_rt(m: &Mtx) -> Mtx {
    let mut out = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = m[j][i];
        }
    }
    for k in 0..3 {
        out[3][k] = -(m[3][0] * out[0][k] + m[3][1] * out[1][k] + m[3][2] * out[2][k]);
    }
    out
}

/// `Vu0View_SetRotTrans(rot, -pos)`: rotation about X, then Z, then Y (inverse angles), translation first.
fn view_matrix(pos: [f32; 3], rot: [f32; 3]) -> Mtx {
    let mut m = IDENTITY;
    let (s, c) = rot[0].sin_cos();
    m[1] = [0.0, c, -s, 0.0];
    m[2] = [0.0, s, c, 0.0];
    let (s, c) = rot[2].sin_cos();
    let (m0, m1) = (m[0], m[1]);
    m[0] = lerp_rows(m0, c, m1, -s);
    m[1] = lerp_rows(m0, s, m1, c);
    let (s, c) = rot[1].sin_cos();
    let (m0, m2) = (m[0], m[2]);
    m[0] = lerp_rows(m0, c, m2, s);
    m[2] = lerp_rows(m0, -s, m2, c);
    let t = [-pos[0], -pos[1], -pos[2]];
    for k in 0..3 {
        m[3][k] = t[0] * m[0][k] + t[1] * m[1][k] + t[2] * m[2][k];
    }
    m[3][3] = 1.0;
    m
}

fn lerp_rows(a: [f32; 4], ka: f32, b: [f32; 4], kb: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] * ka + b[i] * kb)
}

/// `View_BuildProjection` for the full-screen layout: screen size 2047, distance 433, aspect (1 / 1.1666667, 1),
/// centre 2048, Z from 10877000 (near 0.3) to 1 (far 65536), scale 1.
fn projection() -> (Mtx, Mtx, Mtx) {
    let (size, dist, scale) = (2047.0f32, 433.0f32, 1.0f32);
    let (ax, ay) = (1.0 / 1.166_666_7f32, 1.0f32);
    let (center_x, center_y) = (2048.0f32, 2048.0f32);
    let (z_min, z_max, near, far) = (1.0f32, 10_877_000.0f32, 0.3f32, 65536.0f32);
    let z_ofs = (-z_max * near + z_min * far) / (far - near);
    let hw = near * size / dist;
    let hh = near * size / dist;
    let z_scale = far * near * (z_max - z_min) / (far - near);
    let ox = (center_x - 2048.0) / (size * ax);
    let oy = (center_y - 2048.0) / (size * ay);
    let mut persp = IDENTITY;
    persp[0][0] = dist * scale;
    persp[1][1] = dist * scale;
    persp[2][2] = 0.0;
    persp[3][2] = 1.0;
    persp[2][3] = 1.0;
    persp[3][3] = 0.0;
    let mut screen = IDENTITY;
    screen[0][0] = ax;
    screen[1][1] = ay;
    screen[2][2] = z_scale;
    screen[3] = [center_x, center_y, z_ofs, 1.0];
    let mut clip = IDENTITY;
    clip[0][0] = 2.0 * near / (2.0 * hw) * scale;
    clip[1][1] = 2.0 * near / (2.0 * hh) * scale;
    clip[2][2] = (far + near) / (far - near);
    clip[2][3] = 1.0;
    clip[3][2] = far * near * -2.0 / (far - near);
    clip[3][3] = 0.0;
    let mut shift = IDENTITY;
    shift[3][0] = ox;
    shift[3][1] = oy;
    let mut clip2screen = IDENTITY;
    clip2screen[0][0] = dist * ax * hw / near;
    clip2screen[1][1] = dist * ay * hh / near;
    clip2screen[2][2] = (z_min - z_max) * 0.5;
    clip2screen[3] = [2048.0, 2048.0, (z_max + z_min) * 0.5, 1.0];
    (mul(&persp, &screen), mul(&clip, &shift), clip2screen)
}

/// `ObjDraw_DrawParts` mode 0 for one part and VU1 program 0 on its batches: bone A is the part's node, bone B
/// its parent; `offset_a` / `offset_b` are their rest positions in model space.
fn draw_part(
    part: &ModelPart,
    textures: &[[usize; 2]],
    mtx_a: &Mtx,
    mtx_b: &Mtx,
    light_dir: [f32; 3],
    view: &View,
    groups: &mut HashMap<[usize; 2], Vec<ScreenVertex>>,
) {
    // Mtx_MakeNormalLight(light, lightDir, 0, 0): x of light * n is -L . n; Vec3_Normalize clears w.
    let neg_light = scale3(normalize(light_dir), -1.0);
    for (batch, &tex) in part.batches.iter().zip(textures) {
        let mut flags = [false; 2];
        let mut previous: [Option<ScreenVertex>; 2] = [None, None];
        let out = groups.entry(tex).or_default();
        for vertex in &batch.vertices {
            let [x, y, z, w] = vertex.position;
            let pa = apply(
                &mtx_a,
                [
                    x - part.offset_a[0],
                    y - part.offset_a[1],
                    z - part.offset_a[2],
                    1.0,
                ],
            );
            let pb = apply(
                &mtx_b,
                [
                    x - part.offset_b[0],
                    y - part.offset_b[1],
                    z - part.offset_b[2],
                    1.0,
                ],
            );
            let p: [f32; 4] = std::array::from_fn(|i| pb[i] + (pa[i] - pb[i]) * w);
            let [nx, ny, nz] = vertex.normal;
            let la = dot(neg_light, rotate(&mtx_a, [nx, ny, nz]));
            let lb = dot(neg_light, rotate(&mtx_b, [nx, ny, nz]));
            let u = 0.5 + 0.5 * (lb + (la - lb) * w);
            let s = apply(&view.world2screen, [p[0], p[1], p[2], 1.0]);
            let c = apply(&view.world2clip, [p[0], p[1], p[2], 1.0]);
            let q = 1.0 / s[3];
            // ftoi4 for x / y; the GS takes Z from bits 4..27 of the packed word, so whole units.
            let sx = (s[0] * q * 16.0).trunc() / 16.0;
            let sy = (s[1] * q * 16.0).trunc() / 16.0;
            let sz = (s[2] * q).trunc();
            let cw = c[3].abs();
            let clipped = c[0].abs() > cw || c[1].abs() > cw || c[2].abs() > cw;
            let screen = ScreenVertex {
                position: [sx - OFFSET_X, sy - OFFSET_Y, sz],
                stq: [vertex.st[0] * q, vertex.st[1] * q, q],
                uq: u * q,
            };
            // ADC: a vertex closes no triangle when it or one of the two before it is outside the clip volume.
            if let [Some(a), Some(b)] = previous
                && !(clipped || flags[0] || flags[1])
            {
                out.extend([a, b, screen]);
            }
            flags = [flags[1], clipped];
            previous = [previous[1], Some(screen)];
        }
    }
}

pub(crate) fn apply(m: &Mtx, v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|k| v[0] * m[0][k] + v[1] * m[1][k] + v[2] * m[2][k] + v[3] * m[3][k])
}

fn rotate(m: &Mtx, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| v[0] * m[0][k] + v[1] * m[1][k] + v[2] * m[2][k])
}

/// `a` then `b` for row vectors.
pub(crate) fn mul(a: &Mtx, b: &Mtx) -> Mtx {
    std::array::from_fn(|i| apply(b, a[i]))
}

/// `Vec4_RotateEuler` with only an X angle (`Vu0Cur_RotateX`).
fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

/// `Vec4_RotateEuler` with only a Y angle (`Vu0Cur_RotateY`).
fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] + s * v[2], v[1], -s * v[0] + c * v[2]]
}

/// `BtlUtil_WrapAngle`: into [-pi, pi).
fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    scale3(a, 1.0 / length(a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle_seq::SeqState;

    #[test]
    fn projection_puts_the_view_axis_at_the_screen_centre_and_near_at_z_max() {
        let (screen, _, _) = projection();
        let s = apply(&screen, [0.0, 0.0, 0.3, 1.0]);
        assert_eq!(s[0] / s[3], 2048.0);
        assert_eq!(s[1] / s[3], 2048.0);
        assert!((s[2] / s[3] - 10_877_000.0).abs() < 4.0);
        let far = apply(&screen, [0.0, 0.0, 65536.0, 1.0]);
        assert!((far[2] / far[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn view_matrix_looks_along_the_yaw() {
        let m = view_matrix([1.0, 2.0, 3.0], [0.0, FRAC_PI_2, 0.0]);
        // Yaw pi/2 looks along +x: a point ahead of the camera ends up on the view's +z axis.
        let v = apply(&m, [11.0, 2.0, 3.0, 1.0]);
        assert!(v[0].abs() < 1e-5 && v[1].abs() < 1e-5 && (v[2] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn lock_on_camera_sits_behind_and_above_the_fighter() {
        let cam = LockOn {
            pos: [0.0, 0.0, 0.0],
            height: 20.0,
            side_slope: 3.0,
            opp_pos: [0.0, 0.0, 100.0],
            opp_height: 20.0,
            stage_bottom: 1000.0,
            yaw: 0.0,
            side: 0.0,
        };
        let (eye, rot, target) = lock_on_camera(&cam);
        assert!(eye[2] < 0.0 && eye[1] < 0.0 && eye[0].abs() < 1e-5);
        assert_eq!(target, [0.0, -14.0, 0.0]);
        assert_eq!(rot[1], 0.0);
        assert!(rot[0] < 0.0);
    }

    #[test]
    fn real_fighters_load_and_project_onto_the_screen() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = bt3_data::Iso9660::open(&path) else {
            return;
        };
        let cnf = image.read_file("SYSTEM.CNF").unwrap();
        let exe = bt3_data::elf::boot_path(&cnf).unwrap();
        let elf = Elf::parse(image.read_file(&exe).unwrap()).unwrap();
        let mut read = |id: u32| -> Result<Vec<u8>, String> {
            let location = bt3_data::afs::partition_entry_for_id(id);
            let archive = bt3_data::afs::AfsArchive::open(
                &mut image,
                format!("DATA/PZS3US{}.AFS", location.partition),
            )
            .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        };
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 0;
        setup.sides[1].chara = 1;
        setup.stage = 0;
        let assets = load(&mut read, &elf, &setup).unwrap();
        let mut battle = Battle::new(assets);
        assert_eq!(
            battle.assets.fighters[0].motions[IDLE_MOTION]
                .as_ref()
                .unwrap()
                .length,
            67
        );
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        // Mode 0 opens on the stage intro: cut 0 films the stage through the demo camera, under the fade-in.
        let frame = battle.run_frame(
            false,
            [PadState::default(); 2],
            0,
            &mut libc,
            &mut random,
            None,
            true,
        );
        assert_eq!(battle.seq.state, SeqState::StageIntro);
        assert!(battle.demo.is_active());
        assert_eq!(frame.quads.len(), 1);
        let stage_vertices: usize = frame.stage.iter().map(|b| b.vertices.len()).sum();
        assert!(stage_vertices > 1000, "{stage_vertices} stage vertices");
        // Mode 5 starts at Ready, in side 0's camera.
        setup.mode = 5;
        let mut battle = Battle::new(load(&mut read, &elf, &setup).unwrap());
        let frame = battle.run_frame(
            false,
            [PadState::default(); 2],
            0,
            &mut libc,
            &mut random,
            None,
            true,
        );
        assert_eq!(battle.seq.state, SeqState::Ready);
        assert!(!battle.demo.is_active());
        // Stage 0's ground, sky and buildings are in view behind the fighters.
        let stage_vertices: usize = frame.stage.iter().map(|b| b.vertices.len()).sum();
        assert!(stage_vertices > 3000, "{stage_vertices} stage vertices");
        assert!(
            frame.stage.iter().any(|b| b.group == 0) && frame.stage.iter().any(|b| b.group == 2)
        );
        let batches = frame.models;
        let vertices: Vec<_> = batches.iter().flat_map(|b| &b.vertices).collect();
        assert!(vertices.len() > 3000);
        // The two fighters stand in front of the camera: most of what is drawn is on screen.
        let on_screen = vertices
            .iter()
            .filter(|v| {
                (0.0..512.0).contains(&v.position[0]) && (0.0..448.0).contains(&v.position[1])
            })
            .count();
        assert!(on_screen * 2 > vertices.len());
        // The idle stance: ankles (node 11) on the ground, hips (node 2) lowered, the head (node 0x30) above them
        // and leaning forward (-Z is the model's front); y points down.
        let fighter = &battle.assets.fighters[0];
        let data = side_data(fighter);
        let mut obj = crate::btl_obj::Obj::new(&data.obj, 0.0);
        obj.play(&data.obj, 0, IDLE_MOTION, true);
        obj.frame = 10.0;
        obj.sample_pose(&data.obj);
        obj.calc_matrices(&data.obj);
        let at = |id: u32| {
            let n = obj.node(id).unwrap();
            n.mtx[3]
        };
        assert!((at(11)[1] + 1.2).abs() < 0.5);
        assert!(at(2)[1] > -8.0 && at(2)[1] < -5.0);
        assert!(at(u32::from(HEAD_NODE))[1] < at(2)[1] - 3.0 && at(u32::from(HEAD_NODE))[2] < -2.0);
        // 0.8 s into Ready: READY with announcer 0's line 0, drawn from sheet 1 of the HUD file.
        let mut sounds = Vec::new();
        let mut hud_frames = 0;
        let mut gauges = false;
        for _ in 0..40 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                true,
            );
            sounds.extend(frame.sounds);
            if frame.hud.prims.iter().any(|p| p.texture.is_some()) {
                hud_frames += 1;
            }
            gauges |= frame.hud.layer.is_some();
        }
        assert!(
            sounds.contains(&TitleSound::StreamSe(0x10B6C)),
            "{sounds:?}"
        );
        assert!(hud_frames > 10);
        assert!(gauges);
        assert_eq!(battle.assets.hud_sheets[1].sizes[0], [512, 64]);
        // Goku: 40000 health, 40000 ki, 6 blast stocks; Hud_ShowAll(1) came with Ready.
        assert_eq!(battle.assets.fighters[0].gauge.hp, 40000);
        assert_eq!(battle.assets.fighters[0].gauge.blast_max, 600000);
        // An unseen frame updates the HUD but draws nothing.
        assert!(
            battle
                .run_frame(
                    false,
                    [PadState::default(); 2],
                    0,
                    &mut libc,
                    &mut random,
                    None,
                    false
                )
                .hud
                .layer
                .is_none()
        );
        let layer = battle
            .run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                true,
            )
            .hud
            .layer
            .unwrap();
        let covered = |x0: usize, x1: usize, y0: usize, y1: usize| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (y * 512 + x) * 4 + 3))
                .filter(|&i| layer[i] > 128)
                .count()
        };
        // Both panels at the top corners, mirrored; the infinite clock between them.
        assert!(covered(0, 200, 0, 80) > 2000);
        assert!(covered(312, 512, 0, 80) > 2000);
        assert!(covered(230, 282, 20, 50) > 200);
        assert!(covered(0, 512, 120, 448) < 100);
    }

    /// The battle of a setup read from the ISO, `None` without it.
    fn real_battle(setup: &BattleSetup) -> Option<Battle> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let mut image = bt3_data::Iso9660::open(&path).ok()?;
        let cnf = image.read_file("SYSTEM.CNF").unwrap();
        let exe = bt3_data::elf::boot_path(&cnf).unwrap();
        let elf = Elf::parse(image.read_file(&exe).unwrap()).unwrap();
        let mut read = |id: u32| -> Result<Vec<u8>, String> {
            let location = bt3_data::afs::partition_entry_for_id(id);
            let archive = bt3_data::afs::AfsArchive::open(
                &mut image,
                format!("DATA/PZS3US{}.AFS", location.partition),
            )
            .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        };
        Some(Battle::new(load(&mut read, &elf, setup).unwrap()))
    }

    #[test]
    fn real_fighters_walk_and_rush_in_the_fight() {
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        // An idle second player: these check the player side, the CPU has its own test.
        setup.sides[1].pad = 1;
        setup.mode = 5;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let idle = [PadState::default(); 2];
        let pad = |held: u32| {
            [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        // Ready: 0.8 s, then READY for 2 s; the fighters stand on the ground in the neutral action.
        for _ in 0..90 {
            battle.run_frame(false, idle, 0, &mut libc, &mut random, None, false);
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        for chr in &battle.roster.chars {
            assert_eq!(chr.act.current, 0xB);
            assert!(chr.flags.test(0xF), "on the ground");
            assert!(chr.flags.test(3) && chr.flags.test(5));
        }
        let gap = |b: &Battle| crate::btl_chr::opp_distance(&b.roster.chars[0], &b.roster.chars[1]);
        let start = gap(&battle);
        // Up walks towards the opponent (the camera looks at it): action 0xD, or 0xE once close.
        for _ in 0..20 {
            battle.run_frame(
                false,
                pad(crate::btl_input::pad::UP),
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
        }
        let current = battle.roster.chars[0].act.current;
        assert!(matches!(current, 0xD | 0xE), "action 0x{current:X}");
        assert!(gap(&battle) < start - 10.0, "{start} -> {}", gap(&battle));
        // Close in, then Square taps: rush steps that hit and take health.
        for _ in 0..60 {
            if gap(&battle) < 25.0 {
                break;
            }
            battle.run_frame(
                false,
                pad(crate::btl_input::pad::UP),
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
        }
        let health = battle.roster.chars[1].gauge.health;
        let mut rushed = false;
        let mut reacted = false;
        for i in 0..90 {
            let held = if i % 6 < 2 {
                crate::btl_input::pad::SQUARE
            } else {
                0
            };
            battle.run_frame(false, pad(held), 0, &mut libc, &mut random, None, false);
            rushed |= battle.roster.chars[0].act.current == 0x44;
            reacted |= matches!(battle.roster.chars[1].act.current, 0xC0..=0xC6 | 0xD0);
        }
        assert!(rushed, "no rush");
        assert!(reacted, "no hit reaction");
        assert!(
            battle.roster.chars[1].gauge.health < health,
            "{health} -> {}",
            battle.roster.chars[1].gauge.health
        );
    }

    #[test]
    fn real_battle_ends_by_ko_and_by_time_up() {
        use crate::battle_seq::{REASON_KO, REASON_TIME_UP, WIN_P1};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        // An idle second player: these check the player side, the CPU has its own test.
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pad = |held: u32| {
            [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        // START skips the stage intro and the talk.
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pad(0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        // One rush step from close by on an opponent with 100 health left: K.O.
        battle.roster.chars[1].gauge.health = 100;
        for _ in 0..120 {
            let near =
                crate::btl_chr::opp_distance(&battle.roster.chars[0], &battle.roster.chars[1])
                    < 25.0;
            let held = if near {
                crate::btl_input::pad::SQUARE
            } else {
                crate::btl_input::pad::UP
            };
            battle.run_frame(false, pad(held), 0, &mut libc, &mut random, None, false);
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state != SeqState::Fight {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::Finish);
        assert_eq!((battle.seq.winner, battle.seq.reason), (WIN_P1, REASON_KO));
        // The defeated fighter goes down; 3.5 s later the winner poses (action 2) under its camera, then the end.
        let mut posed = false;
        let mut downed = false;
        for _ in 0..600 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            posed |= battle.roster.chars[0].act.current == 2;
            downed |= matches!(
                battle.roster.chars[1].act.current,
                0xCF | 0xD8 | 0xD9 | 0xDB | 0xDC
            );
            if battle.seq.state == SeqState::End {
                break;
            }
        }
        assert!(downed && posed, "downed {downed} posed {posed}");
        assert_eq!(battle.seq.state, SeqState::End);
        // Time up: 45 s with both standing, the healthier side wins.
        setup.time_limit = 5;
        setup.mode = 5;
        let mut battle = real_battle(&setup).unwrap();
        for _ in 0..90 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        battle.roster.chars[0].gauge.health -= 1000;
        let mut frames = 0;
        while battle.seq.state == SeqState::Fight && frames < 2000 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            frames += 1;
        }
        assert_eq!(battle.seq.state, SeqState::Finish);
        assert_eq!(battle.seq.reason, REASON_TIME_UP);
        assert_eq!(battle.seq.winner, crate::battle_seq::WIN_P2);
        // 45 s at 30 frames a second, give or take the clock's 34 / 32 ms steps.
        assert!((1340..=1360).contains(&frames), "{frames}");
    }

    #[test]
    fn real_cpu_closes_in_rushes_and_knocks_out_an_idle_player() {
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].control = 2;
        setup.sides[1].cpu_level = 13;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let idle = [PadState::default(); 2];
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, idle, 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        let gap = |b: &Battle| crate::btl_chr::opp_distance(&b.roster.chars[0], &b.roster.chars[1]);
        let start_gap = gap(&battle);
        let health = battle.roster.chars[0].gauge.health;
        let mut actions = std::collections::BTreeSet::new();
        let mut pressed = 0u32;
        let mut min_gap = start_gap;
        for _ in 0..5400 {
            battle.run_frame(false, idle, 0, &mut libc, &mut random, None, false);
            actions.insert(battle.roster.chars[1].act.current);
            pressed |= battle.roster.chars[1].input.inject_buttons;
            min_gap = min_gap.min(gap(&battle));
            if battle.seq.state != SeqState::Fight {
                break;
            }
        }
        eprintln!(
            "cpu actions {:X?} buttons {pressed:X} gap {start_gap} -> min {min_gap}; p1 health {health} -> {}",
            actions, battle.roster.chars[0].gauge.health
        );
        assert!(pressed != 0, "the CPU pressed nothing");
        assert!(min_gap < start_gap - 10.0, "the CPU did not close in");
        assert!(
            battle.roster.chars[0].gauge.health < health,
            "the CPU never hit"
        );
        // The idle player is knocked out: the round ends for the CPU side.
        assert!(actions.contains(&0x44), "no rush step");
        assert_eq!(battle.seq.state, SeqState::Finish);
        assert_eq!(
            (battle.seq.winner, battle.seq.reason),
            (crate::battle_seq::WIN_P2, crate::battle_seq::REASON_KO)
        );
    }

    /// From the start to the result menu: START skips the intro, the CPU side loses its last health, then the
    /// finish and winner scenes run to End.
    fn run_to_result_menu(battle: &mut Battle, libc: &mut LibcRandom, random: &mut GameRandom) {
        let pad = |held: u32| {
            [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pad(0), 0, libc, random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        // The CPU side has no health left: K.O.
        battle.roster.chars[1].gauge.health = 0;
        for _ in 0..800 {
            battle.run_frame(false, pad(0), 0, libc, random, None, false);
            if battle.seq.state == SeqState::End {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::End);
    }

    #[test]
    fn real_result_menu_restarts_the_match_or_leaves_for_the_character_select() {
        use crate::btl_input::pad::{CROSS, DOWN, UP};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].control = 2;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut tap = |battle: &mut Battle, held: u32| {
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            let frame = battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
            let idle = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            frame.exit.or(idle.exit)
        };
        run_to_result_menu(
            &mut battle,
            &mut LibcRandom::new(2),
            &mut GameRandom::from_key(&[2]),
        );
        // The box opens over six frames; the cursor starts on "Fight Again" (the title is skipped).
        for _ in 0..4 {
            assert_eq!(tap(&mut battle, 0), None);
        }
        assert!(battle.menu.as_ref().unwrap().active);
        assert_eq!(tap(&mut battle, CROSS), None);
        // Fight Again: the fade out, then the restart into a fresh match.
        for _ in 0..40 {
            assert_eq!(tap(&mut battle, 0), None);
            if battle.seq.state == SeqState::StageIntro {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::StageIntro);
        assert_eq!(
            battle.roster.chars[1].gauge.health,
            battle.roster.chars[1].gauge.health_max
        );
        assert_eq!(battle.seq.winner, 0);

        // Again, now "Return to Character Select", then "Yes" in the box (it opens on "No").
        run_to_result_menu(
            &mut battle,
            &mut LibcRandom::new(3),
            &mut GameRandom::from_key(&[3]),
        );
        for _ in 0..4 {
            tap(&mut battle, 0);
        }
        tap(&mut battle, DOWN);
        tap(&mut battle, CROSS);
        for _ in 0..4 {
            tap(&mut battle, 0);
        }
        tap(&mut battle, UP);
        assert_eq!(tap(&mut battle, CROSS), None);
        let mut exit = None;
        for _ in 0..40 {
            exit = tap(&mut battle, 0);
            if exit.is_some() {
                break;
            }
        }
        assert_eq!(exit, Some(0x10));
    }

    #[test]
    fn real_dash_closes_in_and_guard_blocks_a_rush() {
        use crate::btl_input::pad::{CIRCLE, CROSS, SQUARE, UP};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32, b: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState {
                    held: b,
                    ..PadState::default()
                },
            ]
        };
        // START skips the stage intro and the talk.
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pads(0, 0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        let gap = |b: &Battle| crate::btl_chr::opp_distance(&b.roster.chars[0], &b.roster.chars[1]);
        // Cross held: the dash (0xF) runs at the dash speed, much faster than walking.
        let start = gap(&battle);
        let mut dashed = false;
        let mut frames = 0;
        while gap(&battle) > 30.0 && frames < 60 {
            battle.run_frame(
                false,
                pads(CROSS, 0),
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            dashed |= battle.roster.chars[0].act.current == 0xF;
            frames += 1;
        }
        assert!(dashed, "no dash");
        assert!(
            gap(&battle) <= 30.0,
            "{start} -> {} in {frames} frames",
            gap(&battle)
        );
        assert!(
            frames < 20,
            "{start} -> {} took {frames} frames",
            gap(&battle)
        );
        // Let the dash stop, then P1 guards (0x38) while P2 rushes into it.
        for _ in 0..30 {
            battle.run_frame(
                false,
                pads(CIRCLE, 0),
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
        }
        assert_eq!(battle.roster.chars[0].act.current, 0x38);
        let health = battle.roster.chars[0].gauge.health;
        let mut recoil = false;
        let mut rushed = false;
        for i in 0..120 {
            let near = gap(&battle) < 25.0;
            let p2 = if !near {
                UP
            } else if i % 6 < 2 {
                SQUARE
            } else {
                0
            };
            battle.run_frame(
                false,
                pads(CIRCLE, p2),
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            rushed |= battle.roster.chars[1].act.current == 0x44;
            recoil |= matches!(battle.roster.chars[0].anim.cur, 0xF0 | 0xF2 | 0xF4 | 0xF5);
            assert_eq!(
                battle.roster.chars[0].act.current, 0x38,
                "guard broken at frame {i}"
            );
        }
        assert!(rushed, "P2 never rushed");
        assert!(recoil, "no guard recoil");
        eprintln!(
            "guarded: health {health} -> {}",
            battle.roster.chars[0].gauge.health
        );
        assert!(
            health - battle.roster.chars[0].gauge.health < 300,
            "guarded hits took full damage"
        );
    }

    #[test]
    fn real_ascend_takes_off_into_flight_and_descend_lands() {
        use crate::btl_input::pad::{R1, R2};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        setup.mode = 5;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pad = |held: u32| {
            [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        for _ in 0..90 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        let ground = battle.roster.chars[0].pose.pos[1];
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..60 {
            battle.run_frame(false, pad(R1), 0, &mut libc, &mut random, None, false);
            seen.insert(battle.roster.chars[0].act.current);
        }
        let high = battle.roster.chars[0].pose.pos[1];
        eprintln!("ascend actions {seen:X?}, y {ground} -> {high}");
        assert!(seen.contains(&0x10) && seen.contains(&0x14), "{seen:X?}");
        // +Y is down: flying 60 frames takes the fighter well above the ground, in flight (flag 0xE).
        assert!(high < ground - 30.0, "{ground} -> {high}");
        assert!(battle.roster.chars[0].flags.test(0xE));
        let mut seen = std::collections::BTreeSet::new();
        let mut landed = false;
        for _ in 0..240 {
            battle.run_frame(false, pad(R2), 0, &mut libc, &mut random, None, false);
            seen.insert(battle.roster.chars[0].act.current);
            if battle.roster.chars[0].flags.test(0xF) {
                landed = true;
                break;
            }
        }
        eprintln!("descend actions {seen:X?}");
        assert!(seen.contains(&0x15), "{seen:X?}");
        assert!(landed, "never landed");
    }

    #[test]
    fn real_steps_and_dashes() {
        use crate::btl_input::pad::{CROSS, DOWN, L2, LEFT, UP};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        setup.mode = 5;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut run =
            |battle: &mut Battle, held: u32, frames: usize| -> std::collections::BTreeSet<i32> {
                let mut seen = std::collections::BTreeSet::new();
                for _ in 0..frames {
                    let pads = [
                        PadState {
                            held,
                            ..PadState::default()
                        },
                        PadState::default(),
                    ];
                    battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
                    seen.insert(battle.roster.chars[0].act.current);
                }
                seen
            };
        run(&mut battle, 0, 90);
        assert_eq!(battle.seq.state, SeqState::Fight);
        let gap = |b: &Battle| crate::btl_chr::opp_distance(&b.roster.chars[0], &b.roster.chars[1]);
        while gap(&battle) > 25.0 {
            run(&mut battle, UP, 1);
        }
        run(&mut battle, 0, 30);
        // Back step: Down held, Cross tapped while close.
        let before = gap(&battle);
        let mut seen = run(&mut battle, DOWN, 2);
        seen.extend(run(&mut battle, DOWN | CROSS, 2));
        seen.extend(run(&mut battle, 0, 40));
        assert!(seen.contains(&0x1B), "{seen:X?}");
        assert!(gap(&battle) > before + 10.0, "{before} -> {}", gap(&battle));
        // Side step: Left held, Cross tapped.
        let mut seen = run(&mut battle, LEFT, 2);
        seen.extend(run(&mut battle, LEFT | CROSS, 2));
        seen.extend(run(&mut battle, 0, 40));
        assert!(seen.contains(&0x1C), "{seen:X?}");
        // Directional dash: L2 (charge) and Down held, Cross tapped; it runs away until released.
        let before = gap(&battle);
        let mut seen = run(&mut battle, L2 | DOWN, 2);
        seen.extend(run(&mut battle, L2 | DOWN | CROSS, 2));
        seen.extend(run(&mut battle, DOWN, 20));
        assert!(seen.contains(&0x1A), "{seen:X?}");
        assert!(gap(&battle) > before + 50.0, "{before} -> {}", gap(&battle));
        run(&mut battle, 0, 60);
        // Homing dash: L2 held, Cross tapped with no direction; it closes in on the opponent.
        let ki = battle.roster.chars[0].gauge.ki;
        let before = gap(&battle);
        let mut seen = run(&mut battle, L2, 2);
        seen.extend(run(&mut battle, L2 | CROSS, 2));
        let mut min = before;
        for _ in 0..150 {
            seen.extend(run(&mut battle, 0, 1));
            min = min.min(gap(&battle));
        }
        eprintln!(
            "homing: {seen:X?} gap {before} -> min {min}, ki {ki} -> {}",
            battle.roster.chars[0].gauge.ki
        );
        assert!(seen.contains(&0x19), "{seen:X?}");
        assert!(min < 30.0, "{before} -> {min}");
    }

    #[test]
    fn real_fighters_cannot_dash_through_the_terrain() {
        use crate::btl_input::pad::{CROSS, DOWN, L2, R1, R2, UP};
        use bt3_data::stage_col::ColBox;
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        setup.mode = 5;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        // The topmost stage surface under a point (the ground probe from far above).
        let surface = |b: &Battle, p: [f32; 3]| {
            let col = &b.assets.collision;
            let zone = col.find_zone_near(-1, p) as i32;
            let probe = ColBox::from_center_half([p[0], -5000.0, p[2]], [0.1, 1.0, 0.1]);
            col.ground_probe(zone, &probe, p[0], p[2], -5000.0).y
        };
        let mut worst = f32::MIN;
        let mut run = |battle: &mut Battle, held: u32, frames: usize, worst: &mut f32| {
            for _ in 0..frames {
                let pads = [
                    PadState {
                        held,
                        ..PadState::default()
                    },
                    PadState::default(),
                ];
                battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
                let p = battle.roster.chars[0].pose.pos;
                let g = surface(battle, p);
                if g < 100_000.0 {
                    // +Y is down: how far below the surface the fighter's feet are.
                    *worst = worst.max(p[1] - g);
                }
            }
        };
        run(&mut battle, 0, 90, &mut worst);
        // Fly up, then dash away and downwards (directional dash with DESCEND) for a long time.
        run(&mut battle, R1, 40, &mut worst);
        run(&mut battle, L2 | DOWN, 2, &mut worst);
        run(&mut battle, L2 | DOWN | CROSS, 2, &mut worst);
        run(&mut battle, DOWN | R2, 150, &mut worst);
        // And the fast descent straight down.
        run(&mut battle, 0, 30, &mut worst);
        run(&mut battle, R1, 40, &mut worst);
        run(&mut battle, UP | R2, 120, &mut worst);
        eprintln!("deepest below the surface: {worst}");
        assert!(worst < 1.0, "the fighter went {worst} below the terrain");
    }

    #[test]
    fn real_ki_blasts_fly_hit_and_are_guarded() {
        use crate::btl_input::pad::{CIRCLE, TRIANGLE};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32, b: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState {
                    held: b,
                    ..PadState::default()
                },
            ]
        };
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pads(0, 0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        assert_eq!(battle.seq.state, SeqState::Fight);
        let mut frame = |b: &mut Battle, p1: u32, p2: u32| {
            b.run_frame(false, pads(p1, p2), 0, &mut libc, &mut random, None, false);
        };
        // A tap of the blast button: action 0xAE fires on its motion's event, the shot flies and hits the idle P2.
        let (ki, health) = (
            battle.roster.chars[0].gauge.ki,
            battle.roster.chars[1].gauge.health,
        );
        let mut fired = false;
        let mut spent = false;
        let mut flew = 0;
        let mut hit_at = None;
        for i in 0..90 {
            let p1 = if i < 2 { TRIANGLE } else { 0 };
            let before = battle.roster.chars[0].gauge.ki;
            frame(&mut battle, p1, 0);
            fired |= battle.roster.chars[0].act.current == 0xAE;
            spent |= battle.roster.chars[0].gauge.ki < before;
            let live = battle.roster.scene.records.len();
            flew = flew.max(live);
            if hit_at.is_none() && battle.roster.chars[1].gauge.health < health {
                hit_at = Some(i);
            }
        }
        let gap = crate::btl_chr::opp_distance(&battle.roster.chars[0], &battle.roster.chars[1]);
        eprintln!(
            "ki {ki} -> {}, P2 health {health} -> {} at {hit_at:?}, reaction {}, gap {gap}",
            battle.roster.chars[0].gauge.ki,
            battle.roster.chars[1].gauge.health,
            battle.roster.chars[1].react.reaction
        );
        assert!(fired, "no ki blast action");
        assert!(spent, "no ki spent");
        assert!(flew > 0, "no blast record");
        assert!(hit_at.is_some(), "the blast never hit");
        assert!(
            battle.roster.scene.records.is_empty(),
            "a blast is still flying"
        );
        let unguarded = health - battle.roster.chars[1].gauge.health;
        // Let P2 recover, then it guards (0x38) while P1 fires again.
        for _ in 0..90 {
            frame(&mut battle, 0, 0);
        }
        let health = battle.roster.chars[1].gauge.health;
        let mut guarded = false;
        for i in 0..90 {
            let p1 = if i % 20 < 2 { TRIANGLE } else { 0 };
            frame(&mut battle, p1, CIRCLE);
            guarded |= battle.roster.chars[1].flags.test(0x66);
        }
        let took = health - battle.roster.chars[1].gauge.health;
        eprintln!("unguarded {unguarded}, guarded {took}");
        assert!(guarded, "no blast was guarded");
        assert!(took < unguarded, "guarding did not reduce the damage");
        // Holding the button charges (0xAF); the release fires a charged kind.
        for _ in 0..60 {
            frame(&mut battle, 0, 0);
        }
        let mut charged = false;
        let mut kinds = Vec::new();
        for i in 0..90 {
            let p1 = if i < 40 { TRIANGLE } else { 0 };
            frame(&mut battle, p1, 0);
            charged |= battle.roster.chars[0].act.current == 0xAF;
            for r in &battle.roster.scene.records {
                if !kinds.contains(&r.atk.kind) {
                    kinds.push(r.atk.kind);
                }
            }
        }
        eprintln!("charged kinds {kinds:?}");
        assert!(charged, "no charged ki blast");
        assert!(kinds.iter().any(|k| (1..=3).contains(k)), "no charged shot");
    }

    #[test]
    fn real_ki_blast_draws_its_effects() {
        ki_blast_draws_its_effects(0);
    }

    #[test]
    fn real_ki_blast_of_character_2_draws_its_sprites() {
        ki_blast_draws_its_effects(2);
    }

    #[test]
    fn real_step_in_and_vanishing_step() {
        use crate::btl_input::pad::{CIRCLE, CROSS, LEFT, SQUARE, START, UP};
        let mut setup = BattleSetup::default();
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        let mut frame = |battle: &mut Battle, held: u32| {
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            battle.run_frame(start, pads(held), 0, &mut libc, &mut random, None, false);
        };
        for i in 0..200 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        for _ in 0..40 {
            frame(&mut battle, 0);
        }
        let dist = |b: &Battle| {
            let (a, o) = (b.roster.chars[0].pose.pos, b.roster.chars[1].pose.pos);
            ((a[0] - o[0]).powi(2) + (a[2] - o[2]).powi(2)).sqrt()
        };
        // Walk in until close (flag 0x13: the opponent within both heights and a margin).
        for _ in 0..300 {
            if battle.roster.chars[0].flags.test(0x13) {
                break;
            }
            frame(&mut battle, UP);
        }
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        assert!(
            battle.roster.chars[0].flags.test(0x13),
            "not close: {}",
            dist(&battle)
        );
        // A dash when close: the short forward dash (action 0x1E), back to neutral when its motion ends.
        let before = dist(&battle);
        frame(&mut battle, CROSS);
        let mut actions = Vec::new();
        for _ in 0..40 {
            actions.push(battle.roster.chars[0].act.current);
            frame(&mut battle, 0);
        }
        assert!(actions.contains(&0x1E), "{actions:x?}");
        assert_eq!(battle.roster.chars[0].act.current, 0xB);
        assert!(
            dist(&battle) < before - 5.0,
            "{before} -> {}",
            dist(&battle)
        );
        // Guard pressed with left during a rush step (the only decide that offers it): the vanishing step to the
        // left (action 0x21). Its ki is spent only when the previous action was the rush, which nothing records
        // (BtlAct_GetPrev reads a word nobody writes), so the ki stays.
        battle.roster.chars[0].gauge.ki = 40000;
        frame(&mut battle, SQUARE);
        frame(&mut battle, 0);
        for _ in 0..3 {
            frame(&mut battle, 0);
        }
        assert_eq!(battle.roster.chars[0].act.current, 0x44);
        let pos = battle.roster.chars[0].pose.pos;
        frame(&mut battle, LEFT | CIRCLE);
        let mut actions = Vec::new();
        for _ in 0..40 {
            actions.push(battle.roster.chars[0].act.current);
            frame(&mut battle, 0);
        }
        assert!(actions.contains(&0x21), "{actions:x?}");
        // (The rush step's hit adds ki of its own.)
        assert!(battle.roster.chars[0].gauge.ki >= 40000);
        let moved =
            crate::btl_chr::length(crate::btl_chr::sub(battle.roster.chars[0].pose.pos, pos));
        assert!(moved > 5.0, "moved {moved}");
    }

    #[test]
    fn real_pause_menu_shows_the_fighters_skill_list() {
        use crate::btl_input::pad::{CROSS, DOWN, RIGHT, START, TRIANGLE};
        let setup = BattleSetup::default();
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        let mut frame = |battle: &mut Battle, held: u32| {
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            battle.run_frame(start, pads(held), 0, &mut libc, &mut random, None, false)
        };
        for i in 0..200 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        frame(&mut battle, 0);
        frame(&mut battle, START);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        // Title, resume, skill list: one down from "resume".
        frame(&mut battle, DOWN);
        frame(&mut battle, 0);
        let closed = frame(&mut battle, 0).hud.prims.len();
        frame(&mut battle, CROSS);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        let list = battle.menu.as_ref().unwrap().skill_list(0);
        // Goku's script: four pages (special attacks, rushing techniques, signature techniques, ...).
        assert_eq!(list.pages, 4);
        assert_eq!(&list.count[..4], &[5, 7, 16, 22]);
        let open = frame(&mut battle, 0).hud.prims.len();
        assert!(
            open > closed,
            "the page drew nothing: {open} prims against {closed}"
        );
        // Right turns the page; down moves the page's cursor.
        frame(&mut battle, RIGHT);
        frame(&mut battle, 0);
        frame(&mut battle, DOWN);
        frame(&mut battle, 0);
        let list = battle.menu.as_ref().unwrap().skill_list(0);
        assert_eq!((list.page, list.cursor[1], list.cursor[0]), (1, 1, 0));
        // Triangle closes the page, then the menu; the fight goes on.
        frame(&mut battle, TRIANGLE);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        frame(&mut battle, TRIANGLE);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        assert!(!battle.seq.paused);
    }

    #[test]
    fn real_pause_menu_freezes_the_fight_and_leaves_for_the_main_menu() {
        use crate::btl_input::pad::{CROSS, DOWN, START, UP};
        let setup = BattleSetup::default();
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        // `start` is START going down, as the game derives it from the same pad.
        let mut frame = |battle: &mut Battle, held: u32| {
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            battle.run_frame(start, pads(held), 0, &mut libc, &mut random, None, false)
        };
        for i in 0..200 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        for _ in 0..30 {
            frame(&mut battle, 0);
        }
        assert!(!battle.seq.paused);
        // START opens the pause menu for pad 0; from the next frame on nothing moves.
        frame(&mut battle, START);
        assert!(battle.seq.paused && battle.seq.pause_menu);
        let still = |b: &Battle| (b.roster.frame, [0, 1].map(|i| b.roster.chars[i].pose.pos));
        let before = still(&battle);
        for _ in 0..40 {
            frame(&mut battle, 0);
        }
        assert_eq!(still(&battle), before, "the fight moved while paused");
        // START again closes it (the menu has finished opening by now) and the fight goes on.
        frame(&mut battle, START);
        for _ in 0..20 {
            if !battle.seq.paused {
                break;
            }
            frame(&mut battle, 0);
        }
        assert!(!battle.seq.paused && !battle.seq.pause_menu);
        frame(&mut battle, 0);
        frame(&mut battle, 0);
        assert!(battle.roster.frame > before.0, "the fight did not resume");
        // Pause again, down to "main menu" (title, resume, skill list, character select, main menu), confirm with
        // "yes" (the yes / no box opens on "no").
        frame(&mut battle, START);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        for _ in 0..3 {
            frame(&mut battle, DOWN);
            frame(&mut battle, 0);
        }
        frame(&mut battle, CROSS);
        for _ in 0..10 {
            frame(&mut battle, 0);
        }
        frame(&mut battle, UP);
        frame(&mut battle, 0);
        frame(&mut battle, CROSS);
        let mut exit = None;
        for _ in 0..90 {
            if let Some(reason) = frame(&mut battle, 0).exit {
                exit = Some(reason);
                break;
            }
        }
        assert_eq!(
            exit,
            Some(8),
            "state {:?}, winner {:#x}",
            battle.seq.state,
            battle.seq.winner
        );
    }

    #[test]
    fn real_charged_ki_blast_of_character_6_emits_quads() {
        use crate::btl_input::pad::TRIANGLE;
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 6;
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        // Its charged blast (action 0xAF) is in the kind 2 list, whose set has a kind 9 emitter.
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pads(0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        let (mut quads, mut emitters, mut most) = (0, 0, 0);
        for i in 0..150 {
            let p1 = if i < 45 { TRIANGLE } else { 0 };
            let frame = battle.run_frame(false, pads(p1), 0, &mut libc, &mut random, None, false);
            let (q, e) = battle.roster.scene.fx.quads_used();
            quads = quads.max(q);
            emitters = emitters.max(e);
            most = most.max(frame.effects.len());
        }
        assert!(
            emitters > 0 && quads > 0,
            "emitters {emitters}, quads {quads}"
        );
        assert!(most > 0);
        assert_eq!(
            battle.roster.scene.fx.quads_used(),
            (0, 0),
            "quads outlived the blast"
        );
    }

    /// Side 0 (character `chara`) fires a tap blast at the idle second player.
    fn ki_blast_draws_its_effects(chara: i32) {
        use crate::btl_input::pad::TRIANGLE;
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = chara;
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let pads = |a: u32| {
            [
                PadState {
                    held: a,
                    ..PadState::default()
                },
                PadState::default(),
            ]
        };
        for _ in 0..200 {
            let start = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            battle.run_frame(start, pads(0), 0, &mut libc, &mut random, None, false);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        let mut most = 0;
        let mut particles = 0;
        let mut bills = 0;
        for i in 0..30 {
            let p1 = if i < 2 { TRIANGLE } else { 0 };
            let frame = battle.run_frame(false, pads(p1), 0, &mut libc, &mut random, None, false);
            if i % 3 == 0 || !frame.effects.is_empty() && most == 0 {
                eprintln!(
                    "frame {i}: {} effect tris, first {:?}",
                    frame.effects.len(),
                    frame.effects.first().map(|t| (
                        t.texture,
                        t.blend,
                        t.vertices.map(|v| v.position),
                        t.vertices.map(|v| v.rgba)
                    ))
                );
            }
            most = most.max(frame.effects.len());
            particles = particles.max(battle.roster.scene.fx.particles_used());
            bills = bills.max(battle.roster.scene.fx.bills_alive());
            for t in &frame.effects {
                for v in &t.vertices {
                    assert!(v.position[0].is_finite() && v.position[1].is_finite());
                }
            }
        }
        assert!(most > 0, "the ki blast drew nothing");
        assert!(particles > 0, "the ki blast emitted no particles");
        assert_eq!(
            battle.roster.scene.fx.particles_used(),
            0,
            "particles leaked from the pool"
        );
        // Character 2's tap blast set has a kind 16 sprite (Goku's has none).
        assert_eq!(bills > 0, chara == 2, "sprites: {bills}");
        assert_eq!(
            battle.roster.scene.fx.bills_alive(),
            0,
            "a sprite outlived its blast"
        );
    }
}
