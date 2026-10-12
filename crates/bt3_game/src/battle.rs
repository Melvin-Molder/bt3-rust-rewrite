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
use bt3_data::chara_data::{CharaEntry, CharaTables, CommonTables, ItemSetStats, item_set_stats};
use bt3_data::eft_pack::{EffectPack, tech_blast_set, tech_pack};
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
use crate::btl_chr::{Env, MemberItems, SideData};
use crate::btl_input::{InputGate, PadState};
use crate::btl_menu::{MenuDraw, MenuInput, MenuTexts, PauseMenu, Tree};
use crate::btl_mgr::Roster;
use crate::btl_obj::ObjData;
use crate::btl_tech_fx::TechPack;
use crate::btl_tech_volley::FxTables;
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
/// `mdl->charCamAnims` (object + 0x908..): the camera animations flags 0xD7..0xD9 play (`DemoCam_PlayCharAnim0..2`).
const CHAR_CAM_SECTION: usize = 0x25;
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
/// `BTL_FILE_SND_COMMON`, `BTL_FILE_SND_STAGE` (+ stage) and `BTL_FILE_VOICE` (+ character): the battle's sample
/// banks 4, 8 and the sides' 0x10 / 0x20.
const SND_COMMON_FILE: u32 = 0x14A;
const SND_STAGE_FILE: u32 = 0x14E;
const SND_CHARA_FILE: u32 = 0xBDA;
/// `BTL_FILE_TRANSITION` / `BTL_FILE_TRANSITION_3`: the stage change's transition pack (a change to stage 3 has
/// its own).
const TRANSITION_FILE: u32 = 0x1C0;
const TRANSITION_FILE_3: u32 = 0x1BF;
/// `gFadeColors[2]`: the stage change's fade, clear then opaque (four floats each).
const FADE_COLORS_SLOT2: u32 = 0x2C_6E60 + 2 * 0x20;
/// `Battle_ResetWork`: `Bgm_Play(Battle_GetBgm() + 0x10B16)`.
pub const BGM_FIRST: u32 = 0x10B16;
/// `BtlObjLight_Init`: section 1 of common file 2 is the light's texture file.
const COMMON_LIGHT_FILE: u32 = 2;
/// `gCommonRes->data[1]`: common file 3, the common effect file (`BtlScene_GetCommonEntry`).
const COMMON_EFFECT_FILE: u32 = 3;
/// Common file 4: the character table (`ChrTblEntry`, 0x3C bytes, header word 1) and the item table
/// (`ItemTblEntry`, 0x28 bytes, header word 2).
const CHARA_TABLE_FILE: u32 = 4;
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
    /// `gEftRay->tex[0]`: the ray bursts' texture among the effect textures.
    pub(crate) ray_texture: Option<usize>,
    /// `gEftGndDust->tex`: the ground dust's texture set (common effect entry 0xB) among the effect textures.
    pub(crate) dust_textures: Vec<usize>,
    /// Stage pack entry 0x12's textures among the effect textures, on a stage that has the entry.
    pub(crate) stage_pack12: Option<Vec<usize>>,
    /// Stage pack entry 0xE's textures (the storm's) among the effect textures.
    pub(crate) stage_pack14: Option<Vec<usize>>,
    /// Stage pack entry 0xB's textures (the weather's) among the effect textures.
    pub(crate) stage_pack11: Option<Vec<usize>>,
    /// The boundary wall's parameters and texture (stage pack entries 0x15..0x17), on a stage with them.
    pub(crate) bound: Option<(bt3_data::stage::StageBound, Option<usize>)>,
    /// The scrolling sheet (stage pack entry 7) and its first texture among the stage textures.
    pub(crate) scroll: Option<(bt3_data::eft_mesh::StageScroll, usize)>,
    /// The surfaces (stage pack entry 0xD), their textures among the effect textures and the reflecting
    /// surface's normal maps (entry 0xA).
    pub(crate) surf: Option<SurfAssets>,
    pub(crate) fighters: Vec<FighterModel>,
    /// The light ramp the shading rows are looked up through, and common file 2 (`BtlLoad_StepChara` reloads a
    /// side with them).
    pub(crate) ramp: Vec<u8>,
    pub(crate) common_file: Vec<u8>,
    /// The sides' starting characters (`BattleSide.startForm`), which a rematch loads back.
    pub(crate) start: [SideLoad; 2],
    stage: StageResource,
    stage_model: StageModel,
    /// The stage's destructible objects (`gBtlStage->objs`).
    pub(crate) stage_objs: std::sync::Mutex<crate::stg_obj::StageObjs>,
    /// The fighters' sound requests and handles (`gBtlChars->snd`).
    pub(crate) snd: std::sync::Mutex<crate::btl_snd::CharSound>,
    /// Sample banks to load (`Snd_LoadBank` / `Snd_ReloadBank` by mask; `None` unloads), taken by the game.
    pub bank_requests: Vec<(u32, Option<Vec<u8>>)>,
    stage_id: i32,
    /// `Battle_GetStartStage`: the stage a rematch loads back.
    start_stage: i32,
    /// `Battle_IsStageChangeEnabled`.
    stage_change: bool,
    /// `Battle_IsTimeLimitOff`.
    time_limit_off: bool,
    /// `gFadeColors[2]` (clear, opaque): the stage change's fade.
    fade_colors: [[f32; 4]; 2],
    /// Counts the stage swaps: the stage's textures are uploaded again when it changes.
    pub stage_rev: u32,
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
    /// The executable's tables of the technique effects.
    pub(crate) fx_tables: FxTables,
    pub(crate) collision: StageCollision,
    /// `Battle_GetSetup()->side[n].cpu`.
    pub(crate) cpu: [bool; 2],
    /// `BattleSide.changeAllowed`: the side may transform, fuse and switch.
    pub(crate) change_allowed: [i32; 2],
    /// `BattleSide.members` (character, costume; the lead first) and `switchEnabled`.
    pub(crate) team: [[(i32, i32); 5]; 2],
    pub(crate) team_count: [usize; 2],
    pub(crate) switch_enabled: [bool; 2],
    /// The save's unlocked characters (`BattleSetup_InitCharaBits`).
    pub(crate) chara_bits: [u64; 3],
    /// The CPU player's data and the executable's rule tables, the stage's way points.
    pub(crate) ai: AiData,
    pub(crate) ai_tables: AiTables,
    pub(crate) nav: StageNav,
    /// The active members' `cpuLevel` and `aiType`.
    pub(crate) cpu_level: [i32; 2],
    pub(crate) ai_type: [usize; 2],
    /// The members' item bonuses and abilities.
    pub(crate) member_items: [[MemberItems; 5]; 2],
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
    pub(crate) costume: i32,
    /// The characters its two animation files are of (`BtlRes_Reload`'s files 8 and 9).
    pub(crate) anim_chara: i32,
    pub(crate) anim1_chara: i32,
    model: CharaModel,
    /// Per part, per batch: the material texture and the shading row.
    batch_textures: Vec<Vec<[usize; 2]>>,
    /// Every motion of the character's table by id (`None` for an empty section), with `BtlAnim_Play`'s table
    /// flags applied.
    pub(crate) motions: Vec<Option<Motion>>,
    /// The motions with table flag 0x20000000 as decoded, by id (what a throw's victim borrows).
    pub(crate) raw_motions: Vec<Option<Motion>>,
    pub(crate) tables: CharaTables,
    /// `gBtlChars->paramTbl[chara]`.
    pub(crate) entry: CharaEntry,
    /// `mdl.camAnims`: character file sections 0x29..0x2B (entrance, win, lose).
    pub(crate) cam_cuts: [Option<CamAnim>; 3],
    /// `DemoCam_PlayCharAnim0..2`'s animations (model sections 0x25..0x27).
    pub(crate) char_cams: [Option<CamAnim>; 3],
    /// The camera animations of the rush techniques of slots 2..4 (`EftCam_Start` once they connect).
    pub(crate) rush_cams: [Option<CamAnim>; 3],
    pub(crate) talk: Vec<u8>,
    /// `BtlMember_LoadParams` at the start of the battle.
    pub(crate) gauge: SideGauge,
    /// The character's effect pack (`BtlCharApi_GetPlayerCharPack`) as far as the simulation reads it.
    pub(crate) effect: Option<EffectPack>,
    /// The textures and sprite packs of its two ki blast emitter sets.
    pub(crate) blast_assets: [Option<SetAssets>; 2],
    /// The technique slots' effect packs (`effectPacks`, the second animation file's entries 1..5).
    pub(crate) tech_packs: [Option<TechPack>; 5],
    /// `mdl.unkA4` (object + 0xBC): the pause menu's skill-list script, section 0x2D + language (0 here).
    pub(crate) skill_text: Option<Vec<u16>>,
}

/// `BTL_FILE_CHARA_DATA`: a member's 0x1000-byte block, file 8 + character * 2 + side.
const MEMBER_FILE: u32 = 8;

/// The team panel's face of a member (`HudTeam_DrawFaceA`: texture 0 of the member's block).
fn member_face(
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    chara: i32,
    side: usize,
) -> Option<bt3_data::gs::RgbaTexture> {
    let id = MEMBER_FILE + u32::try_from(chara).ok()? * 2 + side as u32;
    let file = read(id).ok()?;
    let res = TextureResource::parse(&file).ok()?;
    let entry = res.entries.first()?;
    decode_sprite_texture_raw(entry, entry).ok()
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
    let LoadedStage {
        stage,
        model: stage_model,
        textures: stage_textures,
        cuts: stage_cuts,
        collision,
        nav,
        pack12,
        pack14,
        pack11,
        bound,
        scroll,
        surf,
    } = load_stage(read, setup.stage)?;
    // BtlLoad_StepInitial: the common, stage and both sides' sample banks.
    let mut snd = crate::btl_snd::CharSound::default();
    let mut bank_requests = Vec::new();
    for (mask, file) in [
        (4, SND_COMMON_FILE),
        (8, SND_STAGE_FILE + setup.stage.max(0) as u32),
        (0x10, SND_CHARA_FILE + setup.sides[0].chara.max(0) as u32),
        (0x20, SND_CHARA_FILE + setup.sides[1].chara.max(0) as u32),
    ] {
        let data = read(file)?;
        if let Some(kind) = crate::btl_snd::CharSound::kind_of_mask(mask) {
            snd.set_bank(kind, Some(&data));
        }
        bank_requests.push((mask, Some(data)));
    }
    let fade = elf.f32s(FADE_COLORS_SLOT2, 8).map_err(error)?;
    let fade_colors = [
        [fade[0], fade[1], fade[2], fade[3]],
        [fade[4], fade[5], fade[6], fade[7]],
    ];
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
    let ai = AiData::parse(&common).map_err(error)?;
    let ai_tables = AiTables::read(elf).map_err(error)?;
    let chara_table = read(CHARA_TABLE_FILE)?;
    let items = setup.sides.map(|side| {
        let mut sets = [side.items; 5];
        sets[1..].copy_from_slice(&side.team_items);
        sets.map(|ids| item_set_stats(&chara_table, &ids))
    });
    let ai_type = items.map(|side| side[0].ai_type.max(0) as usize);
    let member_items = std::array::from_fn(|side| {
        // BattleSetup_FinishEx: the bonuses clamped to -20..20, or -20..60 for the computer in modes 1..4.
        let wide = matches!(setup.mode, 1..=4) && setup.sides[side].control == 2;
        items[side].map(|s| member_items(&s, wide))
    });
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
        ray_texture: None,
        dust_textures: Vec::new(),
        stage_pack12: None,
        stage_pack14: None,
        stage_pack11: None,
        bound: None,
        scroll,
        surf: None,
        fighters: Vec::new(),
        ramp: Vec::new(),
        common_file: Vec::new(),
        start: setup.sides.map(|s| SideLoad {
            chara: s.chara,
            costume: s.costume,
            variant: 0,
            anim_chara: s.chara,
            anim1_chara: s.chara,
        }),
        stage_objs: std::sync::Mutex::new(crate::stg_obj::StageObjs::new(
            &stage.objects,
            &stage_model,
        )),
        snd: std::sync::Mutex::new(snd),
        bank_requests,
        stage,
        stage_model,
        stage_id: setup.stage,
        start_stage: setup.stage,
        stage_change: setup.stage_change != 0,
        time_limit_off: setup.time_limit == 0,
        fade_colors,
        stage_rev: 0,
        stage_cuts,
        mode: setup.mode,
        hud_textures,
        hud_sheets,
        hud_cpu,
        announcer: setup.announcer,
        time_limit,
        common: common_tables,
        stat_curve,
        fx_tables: FxTables::read(elf).map_err(error)?,
        collision,
        cpu: [setup.sides[0].control == 2, setup.sides[1].control == 2],
        change_allowed: setup.sides.map(|s| s.change_allowed),
        team: setup.sides.map(|s| {
            let mut t = [(s.chara, s.costume); 5];
            t[1..].copy_from_slice(&s.team);
            t
        }),
        team_count: setup.sides.map(|s| 1 + s.team_count.clamp(0, 4) as usize),
        switch_enabled: setup.sides.map(|s| s.switch_enabled != 0),
        chara_bits: crate::save::SaveData::default().chara_bits,
        ai,
        ai_tables,
        nav,
        cpu_level: setup.sides.map(|side| side.cpu_level),
        ai_type,
        member_items,
        screen_mode: setup.screen_mode,
        bgm: setup.bgm,
        menus,
        menu_sheet,
        menu_texts,
    };
    // BtlLoad: every member's block (BattleMember.data).
    for side in 0..2 {
        for m in 0..assets.team_count[side] {
            let chara = assets.team[side][m].0;
            assets.hud_cpu.members[side][m] = member_face(read, chara, side);
        }
    }
    // EftRayMgr_Init: EftTexSet_Load32 of common effect entry 10; the bursts draw its first texture.
    let common_fx = read(COMMON_EFFECT_FILE)?;
    if let Some(entry) =
        bt3_data::eft_pack::common_entry(&common_fx, bt3_data::eft_pack::COMMON_RAY_TEXTURES)
        && let Ok(res) = TextureResource::parse(entry)
        && let Some(e) = res.entries.first()
    {
        assets.ray_texture = Some(assets.effect_textures.len());
        assets
            .effect_textures
            .push(bt3_data::gs::decode_sprite_texture(e).map_err(error)?);
    }
    // EftGndDustMgr_Init: EftTexSet_Load8 of common effect entry 0xB.
    if let Some(entry) = bt3_data::eft_pack::common_entry(&common_fx, 0xB)
        && let Ok(res) = TextureResource::parse(entry)
    {
        for e in res.entries.iter().take(8) {
            assets.dust_textures.push(assets.effect_textures.len());
            assets
                .effect_textures
                .push(bt3_data::gs::decode_sprite_texture(e).map_err(error)?);
        }
    }
    assets.stage_pack12 = pack12.map(|t| assets.push_effect_textures(t));
    assets.stage_pack14 = pack14.map(|t| assets.push_effect_textures(t));
    assets.stage_pack11 = pack11.map(|t| assets.push_effect_textures(t));
    assets.surf = surf.map(|s| assets.install_surf(s));
    assets.bound = bound.map(|(p, t)| (p, assets.push_effect_textures(t).first().copied()));
    for (index, side) in setup.sides.into_iter().enumerate() {
        let load = SideLoad {
            chara: side.chara,
            costume: side.costume,
            variant: 0,
            anim_chara: side.chara,
            anim1_chara: side.chara,
        };
        let fighter = load_side(read, &ramp, &common, &mut assets, load, setup.stage, index)?;
        assets.fighters.push(fighter);
    }
    assets.ramp = ramp;
    assets.common_file = common;
    Ok(assets)
}

/// What a side loads (`BtlLoad_StepChara`): the model file `chara * 10 + costume` (+ 4 for the damaged variant)
/// and its two animation files of `anim_chara` / `anim1_chara`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SideLoad {
    pub chara: i32,
    pub costume: i32,
    pub variant: i32,
    pub anim_chara: i32,
    pub anim1_chara: i32,
}

/// A side's character: its model, textures, tables, effects and motions (the HUD face of `index` too).
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_side(
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    ramp: &[u8],
    common: &[u8],
    assets: &mut BattleAssets,
    side: SideLoad,
    stage: i32,
    index: usize,
) -> Result<FighterModel, String> {
    let error = |e: std::io::Error| e.to_string();
    let file_id = CHARA_FILE
        + side.chara.max(0) as u32 * 10
        + side.costume.max(0) as u32
        + if side.variant != 0 { 4 } else { 0 };
    let file = read(file_id)?;
    let mut fighter = load_fighter(&file, ramp, assets)
        .map_err(|e| format!("character file 0x{file_id:X}: {e}"))?;
    fighter.chara = side.chara;
    fighter.costume = side.costume;
    fighter.anim_chara = side.anim_chara;
    fighter.anim1_chara = side.anim1_chara;
    for (i, cut) in fighter.cam_cuts.iter_mut().enumerate() {
        *cut = pack_section(&file, ENTRANCE_CUT_SECTION + i)
            .ok()
            .filter(|s| !s.is_empty())
            .map(CamAnim::parse)
            .transpose()
            .map_err(error)?;
    }
    for (i, cam) in fighter.char_cams.iter_mut().enumerate() {
        *cam = pack_section(&file, CHAR_CAM_SECTION + i)
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|s| CamAnim::parse(s).ok());
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
    let anim_file = read(MOTION_FILE + 1 + side.anim1_chara.max(0) as u32 * 10)?;
    fighter.effect = EffectPack::parse(&anim_file);
    for (i, cam) in fighter.rush_cams.iter_mut().enumerate() {
        *cam = bt3_data::eft_pack::rush_cam(&anim_file, i + 2).and_then(|s| CamAnim::parse(s).ok());
    }
    for slot in 0..5 {
        let Some(pack) = tech_pack(&anim_file, slot) else {
            continue;
        };
        let blast = tech_blast_set(pack);
        let assets = match &blast {
            Some(set) => Some(SetAssets::load(set, &mut assets.effect_textures).map_err(error)?),
            _ => None,
        };
        let places = bt3_data::eft_pack::tech_places(pack, stage.max(0) as usize);
        fighter.tech_packs[slot] = Some(TechPack {
            blast,
            assets,
            places,
        });
    }
    if let Some(pack) = &fighter.effect {
        for (list, set) in pack.blast_sets.iter().enumerate() {
            if let Some(set) = set {
                fighter.blast_assets[list] =
                    Some(SetAssets::load(set, &mut assets.effect_textures).map_err(error)?);
            }
        }
    }
    let motions = read(MOTION_FILE + side.anim_chara.max(0) as u32 * 10)?;
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
        // The motions an opponent borrows (BtlObjAnim_PlayFrom) are kept as decoded as well.
        if motion_flags(&common, id) & 0x2000_0000 != 0 {
            fighter.raw_motions.resize(id + 1, None);
            fighter.raw_motions[id] = Some(motion.clone());
        }
        // BtlAnim_Play: BtlAnim_ApplyTableFlags and BtlAnim_HideModelNodes.
        motion.apply_table_flags(motion_flags(&common, id));
        motion.hide_model_nodes(side.chara);
        fighter.motions.push(Some(motion));
    }
    // BtlObjMdl_BindFiles: motions 0x19E.. are the model file's own eight (sections 0x1D..0x24, not packed; the
    // fusions' two-fighter motions among them). The roster's flag table ends before them.
    for k in 0..MODEL_MOTIONS {
        let motion = pack_section(&file, MODEL_MOTION_SECTION + k)
            .ok()
            .filter(|s| !s.is_empty())
            .map(Motion::parse)
            .transpose()
            .map_err(error)?
            .map(|mut m| {
                m.hide_model_nodes(side.chara);
                m
            });
        fighter.motions.push(motion);
    }
    Ok(fighter)
}

/// `modelAnims`: the model file's own motions, ids 0x19E.. (sections 0x1D..0x24).
const MODEL_MOTIONS: usize = 8;
const MODEL_MOTION_SECTION: usize = 0x1D;

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

/// `BtlMember_ApplyItems` and `BtlMember_ClampBonus` (`wide`: up to 60) as `BtlChar_CopyMemberBonus` /
/// `CopyMemberAbility` hand them to the fighter: stat changes 2, 0, 1, 2, 3, 3, 3 as bonuses 0..6.
fn member_items(stats: &ItemSetStats, wide: bool) -> MemberItems {
    let max = if wide { 60 } else { 20 };
    let s = stats.stat.map(|v| v.clamp(-20, max));
    MemberItems {
        bonus: [s[2], s[0], s[1], s[2], s[3], s[3], s[3]],
        ability: stats.ability,
    }
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

/// What a stage file gives the battle (`BtlStage_Init`, `StgNav_Bind`, the stage's camera cuts).
pub(crate) struct LoadedStage {
    stage: StageResource,
    model: StageModel,
    textures: Vec<RgbaTexture>,
    cuts: Vec<Option<CamAnim>>,
    collision: StageCollision,
    nav: StageNav,
    /// Stage pack entry 0x12's textures (`EftTexSet_Load32` of the water, `EftTexSet_Load8` of the smoke), when
    /// the entry is not empty.
    pack12: Option<Vec<RgbaTexture>>,
    /// Stage pack entry 0xE's textures (`EftStorm_Init`), when the entry is not empty.
    pack14: Option<Vec<RgbaTexture>>,
    /// Stage pack entry 0xB's textures (`EftWeather_Init`), when the entry is not empty.
    pack11: Option<Vec<RgbaTexture>>,
    /// `EftBoundMgr_Init`: the parameters (entry 23), colour (22) and textures (21), when all three are there.
    bound: Option<(bt3_data::stage::StageBound, Vec<RgbaTexture>)>,
    /// `EftStageScroll_Load`: the sheet (entry 7), its textures (entry 6) appended to the stage textures.
    scroll: Option<(bt3_data::eft_mesh::StageScroll, usize)>,
    /// `EftSurf_Init`: the surface file's meshes, textures and parameters (entry 0xD), and the normal maps of a
    /// reflecting one (`EftTexSet_Load34` of entry 0xA, all but its last four textures).
    surf: Option<LoadedSurf>,
}

/// A stage's surfaces as read from its file.
pub(crate) struct LoadedSurf {
    meshes: Vec<bt3_data::stage_surf::SurfMesh>,
    textures: Vec<Option<RgbaTexture>>,
    param: bt3_data::stage_surf::SurfParam,
    normals: Option<Vec<bt3_data::gs::IndexedTexture>>,
}

/// The surfaces as the battle keeps them: their textures among the effect textures, and the reflecting
/// surface's normal maps with the two textures its lit palettes are written into.
#[derive(Clone, Debug)]
pub(crate) struct SurfAssets {
    pub meshes: Vec<bt3_data::stage_surf::SurfMesh>,
    pub textures: Vec<Option<usize>>,
    pub param: bt3_data::stage_surf::SurfParam,
    pub normals: Option<(
        std::sync::Arc<Vec<bt3_data::gs::IndexedTexture>>,
        [usize; 2],
    )>,
}

/// A texture record with its TCC bit clear (the draw sets it; the texels decode the same).
fn without_tcc<'a>(e: &bt3_data::tex::TextureEntry<'a>) -> bt3_data::tex::TextureEntry<'a> {
    bt3_data::tex::TextureEntry {
        pixel_data: e.pixel_data,
        clut_data: e.clut_data,
        pixel_size: e.pixel_size,
        clut_size: e.clut_size,
        pixel_blt: e.pixel_blt,
        clut_blt: e.clut_blt,
        tbp_offset: e.tbp_offset,
        cbp_offset: e.cbp_offset,
        texture_register: e.texture_register & !(1 << 34),
    }
}

/// Reads stage `stage_id`'s file (`BTL_FILE_STAGE + stage`).
fn load_stage(
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    stage_id: i32,
) -> Result<LoadedStage, String> {
    let error = |e: std::io::Error| e.to_string();
    let stage_file = read(STAGE_FILE + stage_id.max(0) as u32)?;
    let stage = StageResource::parse(&stage_file).map_err(error)?;
    let object = StageModel::object(&stage_file).map_err(error)?;
    let model = StageModel::parse(object).map_err(error)?;
    let mut textures = TextureResource::parse_at(object, model.texture_header)
        .map_err(error)?
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let cuts = (0..3)
        .map(|i| {
            pack_section(&stage_file, STAGE_CUT_SECTION + i)
                .ok()
                .filter(|s| !s.is_empty())
                .map(CamAnim::parse)
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let entry_size = |n: usize| {
        let word = |k: usize| {
            stage_file
                .get(k * 4..k * 4 + 4)
                .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
        };
        word(n + 1).wrapping_sub(word(n))
    };
    let entry_textures = |n: usize| -> Result<Option<Vec<RgbaTexture>>, String> {
        if entry_size(n) <= 0 {
            return Ok(None);
        }
        let res =
            TextureResource::parse(pack_section(&stage_file, n).map_err(error)?).map_err(error)?;
        Ok(Some(
            res.entries
                .iter()
                .map(decode_sprite_texture)
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?,
        ))
    };
    let pack14 = entry_textures(0xE)?;
    let pack11 = entry_textures(0xB)?;
    let pack12 = if entry_size(0x12) > 0 {
        let res = TextureResource::parse(pack_section(&stage_file, 0x12).map_err(error)?)
            .map_err(error)?;
        Some(
            res.entries
                .iter()
                .map(decode_sprite_texture)
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?,
        )
    } else {
        None
    };
    let bound = if (0x15..=0x17).all(|n| entry_size(n) > 0) {
        let entry = |n| pack_section(&stage_file, n).map_err(error);
        let param =
            bt3_data::stage::StageBound::parse(entry(0x17)?, entry(0x16)?).map_err(error)?;
        let textures = TextureResource::parse(entry(0x15)?)
            .map_err(error)?
            .entries
            .iter()
            .map(decode_sprite_texture)
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        Some((param, textures))
    } else {
        None
    };
    let scroll = if entry_size(7) > 0 {
        let sheet =
            bt3_data::eft_mesh::StageScroll::parse(pack_section(&stage_file, 7).map_err(error)?)
                .map_err(error)?;
        let base = textures.len();
        for e in &TextureResource::parse(pack_section(&stage_file, 6).map_err(error)?)
            .map_err(error)?
            .entries
        {
            textures.push(decode_sprite_texture(e).map_err(error)?);
        }
        Some((sheet, base))
    } else {
        None
    };
    let surf = if entry_size(0xD) > 0 {
        let file =
            bt3_data::stage_surf::SurfFile::parse(pack_section(&stage_file, 0xD).map_err(error)?)
                .map_err(error)?;
        let textures = file
            .textures
            .iter()
            .map(|t| decode_sprite_texture(t).ok())
            .collect();
        let reflects = file.meshes.iter().any(|m| m.flags & 0x800 != 0);
        let normals = if reflects && entry_size(0xA) > 0 {
            let res = TextureResource::parse(pack_section(&stage_file, 0xA).map_err(error)?)
                .map_err(error)?;
            let n = res.entries.len().saturating_sub(4);
            res.entries[..n]
                .iter()
                .map(|e| bt3_data::gs::decode_indexed_texture(&without_tcc(e)).ok())
                .collect::<Option<Vec<_>>>()
                .filter(|f| !f.is_empty())
        } else {
            None
        };
        Some(LoadedSurf {
            meshes: file.meshes,
            textures,
            param: file.param,
            normals,
        })
    } else {
        None
    };
    Ok(LoadedStage {
        stage,
        model,
        textures,
        cuts,
        surf,
        pack12,
        pack14,
        pack11,
        bound,
        scroll,
        collision: StageCollision::parse(&stage_file).map_err(error)?,
        nav: StageNav::parse(&stage_file).map_err(error)?,
    })
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
        costume: 0,
        anim_chara: 0,
        anim1_chara: 0,
        model,
        batch_textures,
        motions: Vec::new(),
        raw_motions: Vec::new(),
        tables: CharaTables::default(),
        entry: CharaEntry::default(),
        cam_cuts: [None, None, None],
        char_cams: [None, None, None],
        rush_cams: [None, None, None],
        talk: Vec::new(),
        gauge: SideGauge::default(),
        effect: None,
        blast_assets: [None, None],
        tech_packs: Default::default(),
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
    /// Effect textures whose texels changed this frame (the reflecting surface's lit palettes).
    pub texture_updates: Vec<(usize, RgbaTexture)>,
}

/// The battle as far as it is ported.
pub struct Battle {
    pub assets: BattleAssets,
    /// `gBtlChars`.
    pub(crate) roster: Roster,
    light_dir: [f32; 3],
    pub(crate) seq: Seq,
    pub(crate) demo: DemoCam,
    /// `gEftCam`: a rush technique's camera animation.
    pub(crate) eft_cam: crate::battle_seq::EftCam,
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
    /// Reads a file of the AFS archives by id for the character changes (`BtlLoad_StepChara`); without it a
    /// change keeps the old character.
    pub loader: Option<Loader>,
    /// The loader's job of the change request being served (`BtlJob`).
    change_job: Option<ChangeJob>,
    /// The stage change being made (`BtlLoad_StepStageChange`).
    stage_job: Option<StageJob>,
    /// `BATTLE_FLAG_LOADING`: a stage change holds the fighters and the clock, and the stage is not drawn.
    pub(crate) loading: bool,
    /// Fade slot 2, the stage change's.
    fade2: Fade,
    /// `gEftBurst`: the transition while it runs.
    burst: Option<crate::eft_burst::Burst>,
    /// `gEftBurstRes` by transition file (loaded once).
    burst_res: Option<(u32, crate::eft_burst::BurstRes)>,
    /// `gEftBurstStreakAngle`: never reset.
    streak_angle: f32,
    /// `gStgAmb`: the stage's looping sound and thunder.
    amb: crate::stg_amb::StgAmb,
    /// The camera's position of the last frame (`gBtlCamView` when the stage updates).
    amb_cam: Option<[f32; 3]>,
}

/// What reads the files of a character change.
pub type Loader = Box<dyn FnMut(u32) -> Result<Vec<u8>, String> + Send + Sync>;

/// `BtlLoad_StepChara` / `BtlLoad_StepObject`: 0 the files are read, 1 they are in (`BtlChange_NotifyLoaded`), 2
/// the swap waits for the fighter (`BtlChange_IsReady`).
struct ChangeJob {
    side: usize,
    object: bool,
    /// The model only (`modelOnly`): the side's animation files, voice and AI stay.
    model_only: bool,
    load: SideLoad,
    state: u8,
    model: Option<FighterModel>,
}

/// What a side's files give the fighter simulation.
pub(crate) fn side_data(f: &FighterModel) -> SideData<'_> {
    SideData {
        obj: ObjData {
            model: &f.model,
            motions: &f.motions,
            raw: &f.raw_motions,
            body: f.tables.body.as_ref(),
            chara: f.chara,
            height: f.tables.params.height(),
        },
        tables: &f.tables,
        entry: f.entry,
        effect: f.effect.as_ref(),
        blast_assets: std::array::from_fn(|i| f.blast_assets[i].as_ref()),
        tech_packs: std::array::from_fn(|i| f.tech_packs[i].as_ref()),
        costume: f.costume,
        chara: f.chara,
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
            stage_objs: &self.stage_objs,
            snd: &self.snd,
            mode: self.mode,
            stage_4_or_27: matches!(self.stage_id, 4 | 27),
            seq_state,
            gate,
            fx: &self.fx_tables,
            change_allowed: self.change_allowed,
            team: self.team,
            team_count: self.team_count,
            switch_enabled: self.switch_enabled,
            items: self.member_items,
            stage_id: self.stage_id,
            chara_bits: std::array::from_fn(|side| {
                // BattleSetup_FixForMode: every character in mode 1, and for the computer's side in modes 2, 3,
                // 4, 7 and 9.
                let all =
                    self.mode == 1 || (matches!(self.mode, 2 | 3 | 4 | 7 | 9) && self.cpu[side]);
                if all { [u64::MAX; 3] } else { self.chara_bits }
            }),
        }
    }
}

impl BattleAssets {
    /// `Snd_ReloadBank(mask, ..)` of a file: the battle's loop flags and the game's bank.
    fn reload_bank(&mut self, read: &mut Loader, mask: u32, file: u32) {
        let data = read(file).ok();
        if let Some(kind) = crate::btl_snd::CharSound::kind_of_mask(mask) {
            self.snd
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_bank(kind, data.as_deref());
        }
        self.bank_requests.push((mask, data));
    }

    /// The surfaces' textures among the effect textures, and two for the reflecting surface's lit palettes (the
    /// first normal map through its own palette until the first frame).
    fn install_surf(&mut self, s: LoadedSurf) -> SurfAssets {
        let textures = self.push_optional_textures(s.textures);
        let normals = s.normals.map(|frames| {
            let first = frames[0].with_palette(&frames[0].palette);
            let out = self.push_effect_textures(vec![first.clone(), first]);
            (std::sync::Arc::new(frames), [out[0], out[1]])
        });
        SurfAssets {
            meshes: s.meshes,
            textures,
            param: s.param,
            normals,
        }
    }

    /// Appends the textures that decoded to the effect textures; their indices.
    fn push_optional_textures(&mut self, t: Vec<Option<RgbaTexture>>) -> Vec<Option<usize>> {
        t.into_iter()
            .map(|t| {
                t.map(|t| {
                    self.effect_textures.push(t);
                    self.effect_textures.len() - 1
                })
            })
            .collect()
    }

    /// Appends decoded textures to the effect textures; their indices.
    fn push_effect_textures(&mut self, t: Vec<RgbaTexture>) -> Vec<usize> {
        let first = self.effect_textures.len();
        self.effect_textures.extend(t);
        (first..self.effect_textures.len()).collect()
    }

    /// `EftWater_Init`: the water task of stage effect kind 7, on a stage with its textures.
    pub(crate) fn new_water(&self) -> Option<crate::eft_water::Water> {
        if !self.stage.water_fx {
            return None;
        }
        let textures = self.stage_pack12.clone()?;
        let mut w = crate::eft_water::Water::new(
            self.stage.water_level.unwrap_or(0.0),
            self.stage_id,
            self.fx_tables.sin_coef,
            textures,
        );
        w.split = self.screen_mode == 1;
        Some(w)
    }

    /// `EftStageScroll_Init`: the sheet of kind 0.
    pub(crate) fn new_scroll(&self) -> Option<crate::stg_scroll::Scroll> {
        let (sheet, base) = self.scroll.clone()?;
        Some(crate::stg_scroll::Scroll::new(sheet, base))
    }

    /// `EftSmokeMgr_Init`: the smoke emitters of kind 4, with stage flag 2 and the stage's records; texture 2 of
    /// the set of pack entry 0x12.
    pub(crate) fn new_smoke(&self) -> Option<crate::eft_smoke::Smoke> {
        if self.stage.stage_flags & 2 == 0 || self.stage.smoke.is_empty() {
            return None;
        }
        let texture = self.stage_pack12.as_ref().and_then(|t| t.get(2).copied());
        Some(crate::eft_smoke::Smoke::new(&self.stage.smoke, texture))
    }

    /// `EftGeyser_MgrInit`: the geysers of kind 6, with stage flag 1 and the stage's records; texture 1 of the
    /// set of pack entry 0x12.
    pub(crate) fn new_geysers(&self) -> Option<crate::eft_geyser::Geysers> {
        if self.stage.stage_flags & 1 == 0 || self.stage.geysers.is_empty() {
            return None;
        }
        let texture = self.stage_pack12.as_ref().and_then(|t| t.get(1).copied());
        Some(crate::eft_geyser::Geysers::new(
            &self.stage.geysers,
            texture,
        ))
    }

    /// `EftStage_CreateKinds`: the tasks of the kinds the stage has, their inits to run.
    pub(crate) fn new_stage_fx(&self) -> crate::stage_fx::StageFx {
        let mut fx = crate::stage_fx::StageFx::default();
        fx.scroll = self.new_scroll();
        // EftSurf_Init.
        fx.surf = self.surf.clone().map(|s| {
            let reflect = s.normals.map(|(frames, out)| {
                crate::eft_surf::Reflect::new(frames, s.param, self.stage.surf.as_ref(), out)
            });
            let level = self.stage.water_level.unwrap_or(0.0);
            crate::eft_surf::Surf::new(s.meshes, s.textures, level, reflect)
        });
        // EftWeather_Init: the record on, the textures of pack entry 0xB.
        fx.weather = self
            .stage
            .weather
            .as_ref()
            .zip(self.stage_pack11.clone())
            .and_then(|(rec, t)| crate::eft_weather::Weather::new(rec, t));
        // EftStorm_Init: the textures of pack entry 0xE.
        fx.storm = self.stage_pack14.clone().map(crate::eft_storm::Storm::new);
        fx.smoke = self.new_smoke();
        fx.geysers = self.new_geysers();
        fx.water = self.new_water();
        fx.bubbles = self.new_bubbles();
        fx.bound = self.new_bound();
        fx.created()
    }

    /// `EftBoundMgr_Init`: the boundary walls of kind 8, with the fighters' body radii.
    pub(crate) fn new_bound(&self) -> Option<crate::eft_bound::Bound> {
        let (param, texture) = self.bound?;
        let radius = [0, 1].map(|i| side_data(&self.fighters[i]).radius());
        let inner = self.stage.bounds.radius - 100.0;
        Some(crate::eft_bound::Bound::new(param, texture, inner, radius))
    }

    /// `EftBubble_Init`: the bubble task of stage effect kind 7, with at least six textures in the set.
    pub(crate) fn new_bubbles(&self) -> Option<crate::eft_bubble::Bubbles> {
        if !self.stage.water_fx {
            return None;
        }
        let textures = self.stage_pack12.clone()?;
        let level = self.stage.water_level.unwrap_or(0.0);
        let mut b = crate::eft_bubble::Bubbles::new(level, self.fx_tables.sin_coef, textures)?;
        b.split = self.screen_mode == 1;
        Some(b)
    }

    /// `BtlLoad_EndStageSwap`'s stage half (`StgModel_InitStage`, `StgNav_Rebind`): the stage's resources, its
    /// objects whole, its textures to upload again.
    fn install_stage(&mut self, s: LoadedStage, stage_id: i32) {
        self.stage_objs =
            std::sync::Mutex::new(crate::stg_obj::StageObjs::new(&s.stage.objects, &s.model));
        self.stage = s.stage;
        self.stage_model = s.model;
        self.stage_textures = s.textures;
        self.stage_cuts = s.cuts;
        self.collision = s.collision;
        self.nav = s.nav;
        self.stage_pack12 = s.pack12.map(|t| self.push_effect_textures(t));
        self.stage_pack14 = s.pack14.map(|t| self.push_effect_textures(t));
        self.stage_pack11 = s.pack11.map(|t| self.push_effect_textures(t));
        self.bound = s
            .bound
            .map(|(p, t)| (p, self.push_effect_textures(t).first().copied()));
        self.scroll = s.scroll;
        self.surf = s.surf.map(|t| self.install_surf(t));
        self.stage_id = stage_id;
        self.stage_rev += 1;
    }
}

/// `BtlLoad_StepStageChange`'s job: its state, the transition pack and the new stage once read.
struct StageJob {
    state: u8,
    transition: Option<Vec<u8>>,
    loaded: Option<LoadedStage>,
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
        roster.scene.gdust =
            crate::eft_gdust::GndDust::new(assets.stage.dust.as_ref(), assets.stage.light_colors);
        // Battle_Init: the tasks' inits, then Battle_Restart's reset.
        roster.scene.stage = assets.new_stage_fx();
        roster.scene.stage.reset();
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
        let hud = Hud::new(
            &assets.hud_sheets,
            assets.hud_cpu.clone(),
            assets.announcer,
            assets.mode,
        );
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
            eft_cam: crate::battle_seq::EftCam::default(),
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
            loader: None,
            change_job: None,
            stage_job: None,
            loading: false,
            fade2: Fade::default(),
            burst: None,
            burst_res: None,
            streak_angle: 0.0,
            amb: crate::stg_amb::StgAmb::default(),
            amb_cam: None,
        }
        .with_ambience()
    }

    /// `Battle_Restart`'s `BtlStage_Reset` at the first start: `StgAmb_Start`.
    fn with_ambience(mut self) -> Self {
        self.amb.start(self.assets.stage_id);
        self
    }

    /// `BtlScene_CheckStageChange` (while fighting, outside modes 4..7, without a time limit under 10 seconds): the
    /// first technique record that hit its fighter (task flag 1 alone of 1 / 2; a multi-hit only on its last hit)
    /// with definition flag 0x200000 while the rule allows stage changes and the victim is below half its health
    /// asks for the stage change (the record marked 2 and 8: no impact effect); one that can finish
    /// (`BtlCharApi_CanTechniqueFinish`: technique flag 0x80000, not a throw, the victim not in its damaged look)
    /// raises the victim's held flag 0xAB.
    fn check_stage_change(&mut self) {
        if (4..8).contains(&self.assets.mode) {
            return;
        }
        if !self.assets.time_limit_off && self.assets.time_limit < 10 {
            return;
        }
        if self.seq.state_id() != 3 {
            return;
        }
        let chars = &self.roster.chars;
        let mut found = None;
        for i in 0..self.roster.scene.records.len() {
            let rec = self.roster.scene.records[i];
            let Some(src) = rec.tech else {
                continue;
            };
            let (me, target) = (&chars[rec.owner], &chars[rec.owner ^ 1]);
            let below_half = !me.is_dead()
                && !target.is_dead()
                && target.gauge.health < target.gauge.health_max / 2;
            let can_finish = crate::btl_act::is_technique_id(me.act.current) && {
                let t = self.assets.fighters[rec.owner].tables.tech();
                let slot = crate::btl_act::id_class(me.act.current);
                t.flags(slot) & 0x8_0000 != 0 && !t.is_throw(slot) && target.gauge.variant == 0
            };
            let hit = if self.assets.stage_change && src.def.flags & 0x20_0000 != 0 && below_half {
                1
            } else if can_finish {
                2
            } else {
                0
            };
            let attr = self.roster.scene.task_flags(i);
            if hit != 0
                && attr & 3 == 1
                && (attr & crate::btl_tech_fx::T_MULTI == 0 || rec.flags & 0x10 != 0)
            {
                if hit == 1 {
                    self.roster.scene.records[i].flags |= 2 | 8;
                }
                found = Some((hit, rec.owner ^ 1));
                break;
            }
        }
        match found {
            Some((1, _)) => {
                // BtlStage_RequestChange: BtlStage_GetChangeTarget.
                let target = match self.assets.stage.change_kind {
                    1 => 0xF,
                    2 => 3,
                    _ => -1,
                };
                self.request_stage_change(target);
            }
            Some((_, target)) => self.roster.chars[target].flags.set_held(0xAB),
            None => {}
        }
    }

    /// `BtlLoad_RequestStageChange`: not while a character change loads or a stage loads, nor to the same stage;
    /// the stage becomes the new one at once and the screen fades in from the slot's colour.
    fn request_stage_change(&mut self, stage: i32) {
        if self.change_job.is_some() || self.loading || stage < 0 || stage == self.assets.stage_id {
            return;
        }
        self.stage_job = Some(StageJob {
            state: 0,
            transition: None,
            loaded: None,
        });
        self.assets.stage_id = stage;
        self.fade2 = Fade::start_colored(self.assets.fade_colors, true, 1.0);
    }

    /// `BtlLoad_StepStageChange`, one step a frame: the loading flag; the transition pack; the transition
    /// (`EftBurst_Start`: its camera animation, pack entry 1, held); the new stage's file; once the transition's 150
    /// frames are over, the fade out to the slot's colour; covered, the transition ends; the new stage in
    /// (`BtlLoad_EndStageSwap`: the fighters back at their starting places); the fade in. The stage's sound bank,
    /// the transition's particles and the stage effects are not ported.
    fn step_stage_job(&mut self, paused: bool, libc: &mut LibcRandom) {
        let Some(job) = &mut self.stage_job else {
            return;
        };
        match job.state {
            0 => {
                self.loading = true;
                job.state = 1;
            }
            1 => {
                // BtlLoad_BeginStageSwap: EftStage_ResetAll (BtlScene_Reset(0): the dust puffs end), BtlStage_Term
                // (StgAmb_Stop).
                self.roster.scene.gdust.reset();
                // ... and EftStage_FreeKinds.
                let fx = &mut self.roster.scene.stage;
                fx.reset();
                fx.run_pending(libc, &mut self.vu0);
                *fx = Default::default();
                self.amb.stop();
                let id = if self.assets.stage_id == 3 {
                    TRANSITION_FILE_3
                } else {
                    TRANSITION_FILE
                };
                job.transition = self.loader.as_mut().and_then(|read| read(id).ok());
                if self.burst_res.as_ref().is_none_or(|(f, _)| *f != id)
                    && let Some(pack) = job.transition.as_deref()
                {
                    let res =
                        crate::eft_burst::BurstRes::load(pack, &mut self.assets.effect_textures);
                    self.burst_res = Some((id, res));
                }
                job.state = 2;
            }
            2 => job.state = 3,
            3 => {
                if paused {
                    return;
                }
                // EftBurst_Start: the transition task.
                self.burst = Some(crate::eft_burst::Burst::new());
                let anim = job
                    .transition
                    .as_deref()
                    .and_then(|t| pack_section(t, 1).ok())
                    .and_then(|s| CamAnim::parse(s).ok());
                job.state = 4;
                if let Some(anim) = anim {
                    self.eft_cam.start(&mut self.demo, anim, None, 0.0);
                    self.eft_cam.set_hold(true);
                }
            }
            4 => {
                let stage = self.assets.stage_id;
                job.loaded = self.loader.as_mut().and_then(|read| {
                    load_stage(read, stage)
                        .map_err(|e| bevy::log::error!("cannot load stage {stage}: {e}"))
                        .ok()
                });
                job.state = 5;
            }
            _ => {
                if job.state == 5 {
                    // Snd_ReloadBank(8): the new stage's bank.
                    job.state = 6;
                    let file = SND_STAGE_FILE + self.assets.stage_id.max(0) as u32;
                    if let Some(read) = self.loader.as_mut() {
                        self.assets.reload_bank(read, 8, file);
                    }
                }
                match job.state {
                    6 => {
                        if paused || self.burst.as_ref().is_some_and(|b| b.is_busy()) {
                            return;
                        }
                        self.fade2 = Fade::start_colored(self.assets.fade_colors, false, 1.0);
                        job.state = 7;
                    }
                    7 => {
                        if self.fade2.is_done() {
                            // EftBurst_End.
                            self.burst = None;
                            self.eft_cam.stop(&mut self.demo);
                            job.state = 8;
                        }
                    }
                    8 => {
                        if paused {
                            return;
                        }
                        self.loading = false;
                        if let Some(loaded) = job.loaded.take() {
                            let stage = self.assets.stage_id;
                            self.assets.install_stage(loaded, stage);
                            // EftStage_CreateKinds: the new stage's effects.
                            // EftStage_Recreate.
                            self.roster.scene.stage = self.assets.new_stage_fx();
                            self.light_dir =
                                normalize(self.assets.stage.light_dir.unwrap_or(DEFAULT_LIGHT));
                            self.with_roster(|roster, data, env| {
                                for (chr, data) in roster.chars.iter_mut().zip(data) {
                                    chr.on_stage_loaded(data, env);
                                }
                                false
                            });
                        }
                        // BtlLoad_EndStageSwap: the new stage's BtlStage_Reset (StgAmb_Start).
                        self.amb.start(self.assets.stage_id);
                        if let Some(job) = &mut self.stage_job {
                            job.transition = None;
                            job.state = 9;
                        }
                    }
                    _ => {
                        self.fade2 = Fade::start_colored(self.assets.fade_colors, true, 1.0);
                        self.stage_job = None;
                    }
                }
            }
        }
    }

    /// `BtlLoad_PollCharaRequest` / `BtlLoad_PollObjectRequest`: the started request of a side becomes the loader's
    /// job (`BtlChange_NotifyTaken`); a character change frees the side's effects first (`BtlScene_FreeChar`).
    fn poll_changes(&mut self) {
        for object in [false, true] {
            for side in 0..2 {
                let Some(req) = self.roster.change.pending(side, object) else {
                    continue;
                };
                let model_only = req.anim_chara < 0 && req.anim1_chara < 0 && req.voice_chara < 0;
                if !object && !model_only {
                    self.roster.scene.reset_side(&mut self.roster.chars, side);
                }
                let cur = &self.assets.fighters[side];
                let load = SideLoad {
                    chara: req.chara,
                    costume: req.costume,
                    variant: req.variant,
                    // A model-only change keeps the side's animation files.
                    anim_chara: if model_only {
                        cur.anim_chara
                    } else {
                        req.anim_chara
                    },
                    anim1_chara: if model_only {
                        cur.anim1_chara
                    } else {
                        req.anim1_chara
                    },
                };
                self.change_job = Some(ChangeJob {
                    side,
                    object,
                    model_only,
                    load,
                    state: 0,
                    model: None,
                });
                self.roster.change.set_taken();
            }
        }
    }

    /// One step of the loader's change job (after the frame, as `Job_Run`): the files are read at once, the next
    /// step reports them in, and the swap waits until the fighter is ready and the battle is not paused. The extra
    /// objects (partners) are not ported: their job only reports in.
    fn step_change_job(&mut self, paused: bool) {
        let Some(job) = &mut self.change_job else {
            return;
        };
        match job.state {
            0 => {
                if !job.object {
                    let (side, load) = (job.side, job.load);
                    let mut model = None;
                    if let Some(read) = self.loader.as_mut() {
                        let (ramp, common) = (
                            std::mem::take(&mut self.assets.ramp),
                            std::mem::take(&mut self.assets.common_file),
                        );
                        let stage = self.assets.stage_id;
                        match load_side(read, &ramp, &common, &mut self.assets, load, stage, side) {
                            Ok(m) => model = Some(m),
                            Err(e) => {
                                bevy::log::error!("cannot load the change of side {side}: {e}")
                            }
                        }
                        self.assets.ramp = ramp;
                        self.assets.common_file = common;
                    }
                    if let Some(job) = &mut self.change_job {
                        job.model = model;
                    }
                }
                if let Some(job) = &mut self.change_job {
                    job.state = 1;
                }
            }
            1 => {
                self.roster.change.set_loaded();
                job.state = 2;
            }
            _ => {
                if !self.roster.change.is_ready(paused) {
                    return;
                }
                let job = self.change_job.take().unwrap();
                if let Some(model) = job.model {
                    self.swap_side(job.side, model, job.model_only);
                }
            }
        }
    }

    /// The swap of `BtlLoad_StepChara`: the side's files replaced (`BtlRes_CommitReload2`), the fighter's object
    /// set up again (`BtlChars_OnModelLoaded`), the side's AI reset (`BtlAiMgr_ResetSide`), its face and skill
    /// list. The voice bank is not ported.
    fn swap_side(&mut self, side: usize, model: FighterModel, model_only: bool) {
        // Snd_ReloadBank(0x10 / 0x20): the side's bank of its new character.
        if !model_only && let Some(read) = self.loader.as_mut() {
            let chara = model.chara.max(0) as u32;
            let mask = if side == 0 { 0x10 } else { 0x20 };
            self.assets.reload_bank(read, mask, SND_CHARA_FILE + chara);
        }
        self.hud
            .set_face(side, self.assets.hud_cpu.faces[side].clone());
        if let Some(menu) = &mut self.menu {
            menu.scripts[side] = model.skill_text.clone();
        }
        self.assets.fighters[side] = model;
        let data = [
            side_data(&self.assets.fighters[0]),
            side_data(&self.assets.fighters[1]),
        ];
        let env = self.assets.env(self.seq.state_id(), self.gate());
        let switching = matches!(self.roster.chars[side].act.current, 0xF3..=0xF8);
        self.roster.chars[side].on_model_loaded(&data[side], &data[side ^ 1], &env);
        self.roster.sync_abilities();
        if model_only {
            return;
        }
        // A change that is not a member switch reloads the active member's block for its new character.
        if !switching && let Some(read) = self.loader.as_mut() {
            let chara = self.assets.fighters[side].chara;
            let face = member_face(&mut **read, chara, side);
            let active = self.roster.chars[side].active;
            self.hud.set_member_face(side, active, face);
        }
        let mut unused = GameRandom::from_key(&[0]);
        let mut no_blasts = Vec::new();
        let mut ctx = AiCtx {
            chars: &mut self.roster.chars,
            data: [&data[0], &data[1]],
            env: &env,
            ai: &self.assets.ai,
            tab: &self.assets.ai_tables,
            nav: &self.assets.nav,
            rng: &mut unused,
            blasts: &mut no_blasts,
        };
        self.ai.reset_one(
            &mut ctx,
            side,
            self.assets.cpu_level[side],
            self.assets.ai_type[side],
        );
    }

    /// The demo camera's requests of this frame: `ChrCam_UpdateDemo` (flag 0xDA stops it, 0xD7..0xD9 play the
    /// fighter's camera animations 0..2), `EftCam_Update`, and the cuts of the rushes that connected
    /// (`EftShotTech_StartCam` / `EftRushShot_StartCam`: the character's camera of the technique's slot, timed by
    /// its rush sequence).
    fn update_demo_requests(&mut self) {
        for side in 0..2 {
            let c = &self.roster.chars[side];
            if c.is_frozen() {
                continue;
            }
            let f = &c.flags;
            if f.test(0xDA) {
                self.demo.stop();
            }
            for (flag, n) in [(0xD7, 0), (0xD8, 1), (0xD9, 2)] {
                if f.test(flag)
                    && let Some(anim) = &self.assets.fighters[side].char_cams[n]
                {
                    self.demo.play_char(anim.clone(), side, n < 2);
                }
            }
        }
        let frame = |c: &crate::btl_chr::Chr| {
            crate::btl_act::is_technique_id(c.act.current)
                .then(|| crate::btl_act::rush_sequence_frame(c))
        };
        let eft_frame = self
            .eft_cam_side()
            .and_then(|s| frame(&self.roster.chars[s]));
        self.eft_cam.update(&mut self.demo, eft_frame, false);
        for (side, slot) in std::mem::take(&mut self.roster.scene.tech.cam_requests) {
            let anim = usize::try_from(slot - 2)
                .ok()
                .and_then(|i| self.assets.fighters[side].rush_cams.get(i))
                .and_then(Option::as_ref);
            if let Some(anim) = anim {
                let time = crate::btl_act::rush_sequence_frame(&self.roster.chars[side]);
                self.eft_cam
                    .start(&mut self.demo, anim.clone(), Some(side), time);
            }
        }
        if std::mem::take(&mut self.roster.scene.tech.cam_stop) {
            self.eft_cam.stop(&mut self.demo);
        }
    }

    /// The fighter the running `EftCam` follows.
    fn eft_cam_side(&self) -> Option<usize> {
        self.eft_cam.side()
    }

    /// `Battle_Restart` for a rematch: `Battle_ResetWork` (sounds stopped, the battle music again, the result
    /// cleared), `StgFx_Reset` (the cross-fade), `BtlGame_Reset` (the menus, the HUD, the sequence),
    /// `BtlChar_ResetAll` and the 1 s fade-in. The AI is not reset, as in the original.
    fn restart(&mut self, sounds: &mut Vec<TitleSound>) {
        // Snd_StopBankAndResume(0x3C): the battle banks' sounds.
        sounds.push(TitleSound::SeStopBanks(0x3C));
        sounds.push(TitleSound::VoiceStop);
        sounds.push(TitleSound::Bgm(BGM_FIRST + self.assets.bgm.max(0) as u32));
        self.xfade = ScrXfade::default();
        // BtlLoad_Reload: a running stage change is dropped and the starting stage loaded back (Fade_ResetAll
        // clears slot 2).
        self.stage_job = None;
        self.loading = false;
        self.burst = None;
        self.fade2 = Fade::default();
        // Battle_Restart's BtlScene_Reset resets the stage effects (kept); loading the first stage back resets
        // them once more (the reload's stage swap) and makes the new stage's.
        let mut stage_fx = std::mem::take(&mut self.roster.scene.stage);
        stage_fx.reset();
        let start = self.assets.start_stage;
        if self.assets.stage_id != start
            && let Some(read) = self.loader.as_mut()
        {
            match load_stage(read, start) {
                Ok(loaded) => {
                    self.assets.install_stage(loaded, start);
                    stage_fx.reset();
                    let old = std::mem::replace(&mut stage_fx, self.assets.new_stage_fx());
                    stage_fx.prev = Some(Box::new(old));
                    self.assets
                        .reload_bank(read, 8, SND_STAGE_FILE + start.max(0) as u32);
                    self.light_dir =
                        normalize(self.assets.stage.light_dir.unwrap_or(DEFAULT_LIGHT));
                }
                Err(e) => bevy::log::error!("cannot load stage {start} back: {e}"),
            }
        }
        // BtlStage_Reset: every stage object whole again, the ambience started again.
        let objs = self.assets.stage_objs.get_mut();
        objs.unwrap_or_else(|e| e.into_inner()).reset_stage();
        self.amb.start(self.assets.stage_id);
        if let Some(menu) = &mut self.menu {
            menu.reset();
        }
        // BtlLoad_Reload: a side that changed character loads its starting one back (Load_RunBlocking).
        self.change_job = None;
        for side in 0..2 {
            let start = self.assets.start[side];
            let f = &self.assets.fighters[side];
            if f.chara == start.chara && f.costume == start.costume {
                continue;
            }
            let Some(read) = self.loader.as_mut() else {
                continue;
            };
            let (ramp, common) = (
                std::mem::take(&mut self.assets.ramp),
                std::mem::take(&mut self.assets.common_file),
            );
            let stage = self.assets.stage_id;
            match load_side(read, &ramp, &common, &mut self.assets, start, stage, side) {
                Ok(model) => {
                    if let Some(menu) = &mut self.menu {
                        menu.scripts[side] = model.skill_text.clone();
                    }
                    self.assets.fighters[side] = model;
                    let mask = if side == 0 { 0x10 } else { 0x20 };
                    let file = SND_CHARA_FILE + start.chara.max(0) as u32;
                    self.assets.reload_bank(read, mask, file);
                }
                Err(e) => bevy::log::error!("cannot load side {side} back: {e}"),
            }
            self.assets.ramp = ramp;
            self.assets.common_file = common;
        }
        self.hud = Hud::new(
            &self.assets.hud_sheets,
            self.assets.hud_cpu.clone(),
            self.assets.announcer,
            self.assets.mode,
        );
        self.seq = Seq::new(self.assets.mode, self.assets.time_limit);
        self.demo = DemoCam::default();
        self.eft_cam = crate::battle_seq::EftCam::default();
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
        // The scene's dust manager keeps the starting stage's template.
        self.roster.scene.gdust = crate::eft_gdust::GndDust::new(
            self.assets.stage.dust.as_ref(),
            self.assets.stage.light_colors,
        );
        self.roster.scene.stage = stage_fx;
        self.roots = std::array::from_fn(|side| self.roster.chars[side].obj.mtx);
        self.fade = Fade::start_in(1.0);
        self.started = false;
    }

    /// The fighter phases with both sides' data and the shared state.
    /// The input gate of the sequence, with the change queue's time-stop word (`BtlChars_IsTimeStopped`).
    fn gate(&self) -> InputGate {
        let mut gate = self.seq.input_gate(self.assets.stage_id);
        gate.time_stopped = self.roster.change.time_stop;
        gate
    }

    fn with_roster(&mut self, f: impl FnOnce(&mut Roster, [&SideData; 2], &Env) -> bool) -> bool {
        let data = [
            side_data(&self.assets.fighters[0]),
            side_data(&self.assets.fighters[1]),
        ];
        let env = self.assets.env(self.seq.state_id(), self.gate());
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
        self.roster.scene.stage.run_pending(libc, &mut self.vu0);
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
        self.fade2.update();
        if self.with_roster(|roster, data, env| roster.check_start(data, env)) {
            self.seq.started = true;
        }
        let sides = [
            self.roster.chars[0].hud_gauge(),
            self.roster.chars[1].hud_gauge(),
        ];
        let teams = [
            self.roster.chars[0].hud_team(),
            self.roster.chars[1].hud_team(),
        ];
        let fight = self.seq.state_id() == 3;
        let [c0, c1] = &self.roster.chars;
        let combos = [c0.hud_combo(c1, fight), c1.hud_combo(c0, fight)];
        self.hud.pre_update(
            self.seq.time_left(),
            &sides,
            &teams,
            &combos,
            self.seq.paused,
        );
        self.seq_pre_update(&mut input);
        // BATTLE_FLAG_PAUSE as the sequence left it last frame.
        let paused = self.seq.paused;
        if !paused {
            // BtlAiMgr_Update, after Battle_UpdateWork and before the fighters sample their input.
            let data = [
                side_data(&self.assets.fighters[0]),
                side_data(&self.assets.fighters[1]),
            ];
            let env = self.assets.env(self.seq.state_id(), self.gate());
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
        let loading = self.loading;
        self.with_roster(|roster, data, env| {
            // BtlChars_SampleInput, UpdateInput, UpdateMain and PostScene do nothing while paused or while a stage
            // loads; the effect scene runs with its time stopped.
            if !paused && !loading {
                roster.sample_input(&pads, data, env);
                roster.update_input(data, env);
                roster.update_main(data, env, &mut *input.libc, &mut vu0);
            }
            roster.update_scene(data, env, &mut *input.libc, &mut vu0, paused);
            if !paused && !loading {
                roster.post_scene(data, env, &mut *input.libc);
            }
            false
        });
        self.check_stage_change();
        if !paused && !self.loading {
            // BtlStage_Update: StgAmb_Update, then the objects' animations and StgRigid_Update (not while paused or
            // loading).
            let water = self.assets.stage.water_level.unwrap_or(0.0);
            self.amb.update(
                self.amb_cam,
                water,
                &mut *input.libc,
                &mut self.assets.snd.lock().unwrap_or_else(|e| e.into_inner()),
            );
            self.with_roster(|roster, _, env| {
                env.objs().update(
                    env.inner_radius(),
                    &mut vu0,
                    &mut *input.libc,
                    &mut roster.scene.gdust,
                    roster.scene.stage.water.as_mut(),
                );
                false
            });
        }
        let objs = self.assets.stage_objs.get_mut();
        objs.unwrap_or_else(|e| e.into_inner())
            .sync_model(&mut self.assets.stage_model);
        self.poll_changes();
        self.vu0 = vu0;
        if !paused {
            self.update_demo_requests();
        }
        // BtlCam_UpdateOverride: the camera with flag 0xD3 (a cut's priority) takes the screen; on a tie the
        // default view, side 0's.
        let pri = [0, 1].map(|s| self.roster.chars[s].flags.test(0xD3));
        let view_side = if pri[1] && !pri[0] { 1 } else { 0 };
        let cam = self.roster.chars[view_side].cam;
        let mut view = View::new(cam.pos, cam.rot);
        if self.demo.is_active() {
            let base = self.demo.attached().map(|side| {
                if self.demo.chr_base() {
                    self.roster.chars[side].obj.mtx
                } else {
                    self.roots[side]
                }
            });
            // A body-changed fighter's cut with scaleHeight: the height times the parameter's height / 1.1.
            let y_scale = self.demo.attached().map_or(1.0, |side| {
                if self.demo.scale_height && self.roster.chars[side].gauge.body_changed {
                    self.assets.fighters[side].tables.params.height() / 1.1
                } else {
                    1.0
                }
            });
            if let Some(demo) = self.demo.update(base, paused, y_scale) {
                view = demo;
                // The camera stops where the stage hides the attached fighter's body from it (BtlCam_TraceStage
                // from the body to the eye).
                if let Some(side) = self.demo.attached()
                    && !self.loading
                {
                    let to = self.roster.chars[side].obj.body_sphere().0;
                    let eye = view.eye();
                    let mut moved = None;
                    self.with_roster(|_, _, env| {
                        if let (pos, Some(_)) = crate::btl_mgr::trace_camera(env, eye, to) {
                            moved = Some(pos);
                        }
                        false
                    });
                    if let Some(pos) = moved {
                        let mut camera = inverse_rt(&view.world2view);
                        camera[3] = [pos[0], pos[1], pos[2], 1.0];
                        view = View::from_world_to_view(inverse_rt(&camera));
                    }
                }
            }
        }
        // BtlChars_EndFrame (not while a stage loads): BtlChange_Update runs even while paused.
        if !self.loading {
            self.roster.change.update();
            if !paused {
                self.with_roster(|roster, data, env| {
                    roster.end_frame(data, env);
                    true
                });
                // BtlCharSnd_PlayRequests, from this frame's camera.
                let charas = [0, 1].map(|i| {
                    if self.roster.chars[i].gauge.body_changed {
                        0x56
                    } else {
                        self.assets.fighters.get(i).map_or(0, |f| f.chara)
                    }
                });
                self.assets
                    .snd
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .play_requests(
                        &view.world2view,
                        self.assets.screen_mode == 1,
                        charas,
                        input.sounds,
                    );
            }
        }
        self.amb_cam = Some(view.eye());
        if let Some(b) = &mut self.roster.scene.stage.bubbles {
            b.cam = Some(view.world2view);
        }
        input.sounds.append(&mut self.amb.out);
        self.step_change_job(paused);
        // EftBurst_Update (not while paused): its particles, frame and sounds.
        if !paused && let Some(b) = &mut self.burst {
            let mut snd = self.assets.snd.lock().unwrap_or_else(|e| e.into_inner());
            b.update(
                &mut *input.libc,
                &mut self.streak_angle,
                &mut snd,
                input.sounds,
            );
        }
        self.step_stage_job(paused, &mut *input.libc);
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
        // StgModel_Cull / StgModel_Draw: nothing while a stage loads.
        let mut stage = if self.loading {
            Vec::new()
        } else {
            draw_stage(&mut self.assets.stage_model, self.assets.stage_id, &view)
        };
        // EftStageScroll_Draw, from the stage draw after group 0.
        if !self.loading
            && let Some(s) = &self.roster.scene.stage.scroll
        {
            stage.extend(s.draw(&view));
        }
        if !self.loading {
            let objs = self
                .assets
                .stage_objs
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            stage.extend(crate::stage_draw::draw_debris(
                &self.assets.stage_model,
                &objs.falling_pieces(),
                objs.node_matrices(),
                &view,
                view.eye(),
            ));
        }
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
        let mut texture_updates = Vec::new();
        let effects = {
            let ctx = DrawCtx::new(view.world2screen, view.world2view);
            let mut ot = OrderTable::new();
            let fxw = FxWorld {
                sets: std::array::from_fn(|s| {
                    let f = &self.assets.fighters[s];
                    std::array::from_fn(|l| match l {
                        0 | 1 => f.blast_assets[l].as_ref(),
                        2..=4 => f.tech_packs[l].as_ref().and_then(|p| p.assets.as_ref()),
                        _ => f.tech_packs[l - 5].as_ref().and_then(|p| p.assets.as_ref()),
                    })
                }),
                any_frozen: self.roster.chars.iter().any(|c| c.is_frozen()),
                // Drawing does not test it.
                stopped: [false; 2],
                clash_a: self
                    .roster
                    .chars
                    .each_ref()
                    .map(crate::btl_clash::Clash::in_clash_a),
                node3: self.roster.chars.each_ref().map(|c| c.obj.node_pos(3)),
                ray_texture: self.assets.ray_texture,
            };
            // BtlCharApi_IsCamShown without camera priorities: the demo camera shows both, else side 0's view.
            let demo = self.demo.is_active();
            // Layer 0's stage effects, then the others.
            let roster = &mut self.roster;
            let mut direct = Vec::new();
            roster.scene.stage.draw(
                &mut ot,
                &mut direct,
                &mut texture_updates,
                &ctx,
                &roster.chars,
                self.light_dir,
                self.seq.paused,
                &mut *libc,
            );
            self.roster
                .scene
                .fx
                .draw(&fxw, &mut ot, &ctx, |chr| demo || chr == 0);
            // EftBurst_IsBusy: the transition's first 150 frames.
            let busy = self.burst.as_ref().is_some_and(|b| b.frame < 150);
            self.roster
                .scene
                .gdust
                .draw(&mut ot, &ctx, &self.assets.dust_textures, busy);
            // EftSurf_DrawMeshes (layer 0) and EftBurst_Draw (layer 4) send their packets straight to the GS while
            // the scene draws: before the table.
            let mut list = direct;
            if let (Some(b), Some((_, res))) = (&self.burst, &self.burst_res) {
                list.extend(b.draw(res, &view.world2view, &view.world2screen));
            }
            list.extend(ot.into_list());
            list
        };
        self.hud.set_paused(self.seq.paused);
        let mut hud = self.hud.draw(libc, shown);
        if let Some(font) = font.as_deref_mut() {
            let scripts = [0, 1].map(|s| self.assets.fighters[s].skill_text.as_deref());
            self.hud.draw_combo_text(font, scripts);
        }
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
            // StgFx_DrawPost's fade slot 2, then Fade_DrawScreen's slot 0.
            quads: self
                .fade2
                .quad()
                .into_iter()
                .chain(self.fade.quad())
                .collect(),
            hud,
            sounds,
            exit: self.seq.left.then_some(self.seq.reason),
            effects,
            texture_updates,
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
    /// The distance preset (`cam->distMode`).
    pub dist_mode: i32,
}

/// `ChrCam_GetOffset`: the offset behind the fighter and the pitch of distance preset `mode` (0..2, SELECT steps
/// it).
pub(crate) fn camera_offset(height: f32, mode: i32) -> ([f32; 3], f32) {
    let dist = (height * 0.5 + 1.0).max(10.0);
    match mode {
        1 => ([0.0, -dist * 1.5, -dist * 2.8], height * -0.001 + -0.06),
        2 => ([0.0, -dist * 2.0, -dist * 3.3], height * -0.001 + -0.1),
        _ => ([0.0, -dist * 1.2, -dist * 2.4], height * -0.001 + -0.05),
    }
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
    let (ofs, mut base) = camera_offset(scale, cam.dist_mode);
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
    pub(crate) fn new(pos: [f32; 3], rot: [f32; 3]) -> Self {
        Self::from_world_to_view(view_matrix(pos, rot))
    }

    /// The camera's position (`view->pos`): the inverse view matrix's translation.
    pub(crate) fn eye(&self) -> [f32; 3] {
        let m = inverse_rt(&self.world2view);
        [m[3][0], m[3][1], m[3][2]]
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
pub(crate) fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

/// `Vec4_RotateEuler` with only a Y angle (`Vu0Cur_RotateY`).
pub(crate) fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
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
            dist_mode: 0,
        };
        let (eye, rot, target) = lock_on_camera(&cam);
        assert!(eye[2] < 0.0 && eye[1] < 0.0 && eye[0].abs() < 1e-5);
        // The farther presets stand farther back.
        let far = lock_on_camera(&LockOn {
            dist_mode: 2,
            ..cam
        })
        .0;
        assert!(far[2] < eye[2] && far[1] < eye[1]);
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
        let mut battle = Battle::new(load(&mut read, &elf, setup).unwrap());
        let mut image = bt3_data::Iso9660::open(&path).ok()?;
        battle.loader = Some(Box::new(move |id: u32| {
            let location = bt3_data::afs::partition_entry_for_id(id);
            let archive = bt3_data::afs::AfsArchive::open(
                &mut image,
                format!("DATA/PZS3US{}.AFS", location.partition),
            )
            .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        }));
        Some(battle)
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
    fn real_cpu_closes_in_rushes_throws_and_knocks_out_an_idle_player() {
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
        let mut dust = 0;
        for _ in 0..5400 {
            battle.run_frame(false, idle, 0, &mut libc, &mut random, None, false);
            dust = dust.max(battle.roster.scene.gdust.fighter_parts());
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
        // The fight raises the fighters' dust (dashes, landings, falls).
        assert!(dust > 0);
        assert!(min_gap < start_gap - 10.0, "the CPU did not close in");
        assert!(
            battle.roster.chars[0].gauge.health < health,
            "the CPU never hit"
        );
        // The CPU rushes, uses its attack records and its techniques (a beam, and a rush technique into its
        // sequence); the idle player is knocked out and the round ends for the CPU side.
        for a in [0x44, 0x106, 0x11C, 0x12E] {
            assert!(actions.contains(&a), "no action {a:X}");
        }
        assert!(
            actions.iter().any(|&a| (0x70..=0xAD).contains(&a)),
            "{actions:X?}"
        );
        assert_eq!(battle.seq.state, SeqState::Finish);
        assert_eq!(
            (battle.seq.winner, battle.seq.reason),
            (crate::battle_seq::WIN_P2, crate::battle_seq::REASON_KO)
        );
    }

    #[test]
    fn real_cpu_transforms_and_switches() {
        // A CPU at low health with blast stocks transforms (sense bit 56: character 1's default transformation),
        // and a CPU with a team member switches (bit 58) once its switch gauge is full (kept full here).
        for (team, want) in [(false, 0xEC..=0xF0), (true, 0xF3..=0xF5)] {
            let mut setup = BattleSetup::default();
            setup.sides[1].chara = 1;
            setup.sides[1].control = 2;
            setup.sides[1].cpu_level = 13;
            setup.sides[1].change_allowed = 1;
            if team {
                setup.sides[1].team[0] = (2, 0);
                setup.sides[1].team_count = 1;
                setup.sides[1].switch_enabled = 1;
            }
            let Some(mut battle) = real_battle(&setup) else {
                return;
            };
            battle.assets.chara_bits = [u64::MAX; 3];
            let mut libc = LibcRandom::new(1);
            let mut random = GameRandom::from_key(&[1]);
            let idle = [PadState::default(); 2];
            for _ in 0..200 {
                let start =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                battle.run_frame(start, idle, 0, &mut libc, &mut random, None, false);
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            let mut seen = false;
            let mut acts = std::collections::BTreeSet::new();
            for _ in 0..1800 {
                let c = &mut battle.roster.chars[1];
                c.gauge.health = c.gauge.health.min(c.gauge.health_max / 5);
                c.gauge.blast = c.gauge.blast_max;
                c.switch_gauge = 100_000;
                battle.run_frame(false, idle, 0, &mut libc, &mut random, None, false);
                acts.insert(battle.roster.chars[1].act.current);
                seen |= want.contains(&battle.roster.chars[1].act.current);
                if seen {
                    break;
                }
            }
            assert!(seen, "team {team}: {acts:X?}");
        }
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
        // Let the dash stop (the guard button during a dash is the air step 0x3E), then P1 guards (0x38) while
        // P2 rushes into it.
        for held in [0, CIRCLE] {
            for _ in 0..30 {
                battle.run_frame(false, pads(held, 0), 0, &mut libc, &mut random, None, false);
            }
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
            col.ground_probe(zone, &probe, p[0], p[2], -5000.0, 0).y
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
    fn real_flying_kick_dash_smash_and_smash_vanish() {
        use crate::btl_input::pad::{CROSS, L2, SQUARE, START, TRIANGLE, UP};
        let run = |pad: &dyn Fn(usize) -> u32| {
            let mut setup = BattleSetup::default();
            setup.sides[1].chara = 1;
            setup.sides[1].pad = 1;
            let mut battle = real_battle(&setup)?;
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
                let skip =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            for _ in 0..20 {
                frame(&mut battle, 0);
            }
            let (mut mine, mut anims, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
            let mut vanished = 0;
            for i in 0..150 {
                frame(&mut battle, pad(i));
                let (a, b) = (&battle.roster.chars[0], &battle.roster.chars[1]);
                mine.push(a.act.current);
                anims.push(a.anim.cur);
                theirs.push(b.act.current);
                vanished = vanished.max(a.vanish_count);
            }
            mine.dedup();
            anims.dedup();
            theirs.dedup();
            Some((mine, anims, theirs, vanished))
        };
        // A dash (Cross), then the rush button: the flying kick 0x58 (wind-up, hold, strike) staggers the opponent.
        let Some((mine, anims, theirs, _)) = run(&|i| match i {
            0..2 => CROSS,
            4..6 => SQUARE,
            _ => 0,
        }) else {
            return;
        };
        // Then, 90 frames idle, the talk 0x43.
        assert_eq!(mine, [0xF, 0x58, 0xB, 0x43], "{mine:x?}");
        assert!(
            anims.windows(3).any(|w| w == [0x6B, 0x6C, 0x6D]),
            "{anims:x?}"
        );
        assert_eq!(theirs, [0xB, 0xC6, 0xB, 0x43], "{theirs:x?}");
        // The homing dash (L2 + Cross), the rush button held: the dash smash 0x57 blows the opponent away (0xD5);
        // up and the blast button then: the vanishing smash 0x5C strikes it again on its way down.
        let Some((mine, anims, theirs, vanished)) = run(&|i| match i {
            0..2 => CROSS | L2,
            20..60 => SQUARE,
            60..70 if i % 2 == 0 => TRIANGLE | UP,
            _ => 0,
        }) else {
            return;
        };
        assert_eq!(&mine[..4], &[0x19, 0x57, 0x5C, 0xB], "{mine:x?}");
        assert!(
            anims.windows(3).any(|w| w == [0x5C, 0x5D, 0x5E]),
            "{anims:x?}"
        );
        assert!(anims.windows(2).any(|w| w == [0x5F, 0x61]), "{anims:x?}");
        assert_eq!(&theirs[..4], &[0xB, 0xD5, 0xDD, 0xDB], "{theirs:x?}");
        assert_eq!(vanished, 1);
    }

    #[test]
    fn real_rush_chains_into_the_attack_records() {
        use crate::btl_input::pad::{SQUARE, START, UP};
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
        for _ in 0..300 {
            if battle.roster.chars[0].flags.test(0x13) {
                break;
            }
            frame(&mut battle, UP);
        }
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        // Tap the rush button: five rush hits, then record 0x7B (motions 0x50, 0x52) knocks the opponent back
        // (0xD4); its follow-up 0x7D comes through the vanishing lead-in 0x2B and blows it away (0xD5).
        let (mut mine, mut anims, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..130 {
            frame(&mut battle, if i % 4 < 2 { SQUARE } else { 0 });
            let a = &battle.roster.chars[0];
            mine.push(a.act.current);
            anims.push(a.anim.cur);
            theirs.push(battle.roster.chars[1].act.current);
        }
        mine.dedup();
        anims.dedup();
        theirs.dedup();
        assert_eq!(&mine[..6], &[0xB, 0x44, 0x7B, 0x2B, 0x7D, 0xB], "{mine:x?}");
        assert!(anims.windows(2).any(|w| w == [0x50, 0x52]), "{anims:x?}");
        assert!(anims.windows(2).any(|w| w == [0x4D, 0x4F]), "{anims:x?}");
        assert_eq!(&theirs[..5], &[0xB, 0xC0, 0xC6, 0xD4, 0xD9], "{theirs:x?}");
        assert!(theirs.contains(&0xD5), "{theirs:x?}");
    }

    #[test]
    fn real_rush_finishers_by_the_rush_step() {
        use crate::btl_input::pad::{SQUARE, START, TRIANGLE, UP};
        let run = |hits: usize, hold: usize| {
            let mut setup = BattleSetup::default();
            setup.sides[1].chara = 1;
            setup.sides[1].pad = 1;
            let mut battle = real_battle(&setup)?;
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
                let skip =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            for _ in 0..300 {
                if battle.roster.chars[0].flags.test(0x13) {
                    break;
                }
                frame(&mut battle, UP);
            }
            for _ in 0..20 {
                frame(&mut battle, 0);
            }
            let (mut mine, mut anims, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
            let tri = hits * 4;
            for i in 0..150 {
                let held = if i < tri {
                    if i % 4 < 2 { SQUARE } else { 0 }
                } else if i < tri + hold {
                    TRIANGLE
                } else {
                    0
                };
                frame(&mut battle, held);
                let a = &battle.roster.chars[0];
                mine.push(a.act.current);
                anims.push(a.anim.cur);
                theirs.push(battle.roster.chars[1].act.current);
            }
            mine.dedup();
            anims.dedup();
            theirs.dedup();
            Some((mine, anims, theirs))
        };
        // Goku's finishers by the rush step (parameter +0x84: 0, 1, 3, 0): the blast button during the rush. Held,
        // 0x60 charges through its wind-up and hold (0x47, 0x48) and its release 0x49 staggers the opponent (0xCB).
        let Some((mine, anims, theirs)) = run(2, 40) else {
            return;
        };
        assert_eq!(&mine[..4], &[0xB, 0x44, 0x60, 0xB], "{mine:x?}");
        assert!(
            anims.windows(3).any(|w| w == [0x47, 0x48, 0x49]),
            "{anims:x?}"
        );
        assert_eq!(theirs, [0xB, 0xCB, 0xB], "{theirs:x?}");
        let Some((mine, anims, _)) = run(3, 2) else {
            return;
        };
        assert_eq!(&mine[..4], &[0xB, 0x44, 0x61, 0xB], "{mine:x?}");
        assert!(
            anims.windows(3).any(|w| w == [0x90, 0x91, 0x92]),
            "{anims:x?}"
        );
        let Some((mine, anims, _)) = run(4, 2) else {
            return;
        };
        assert_eq!(&mine[..4], &[0xB, 0x44, 0x63, 0xB], "{mine:x?}");
        assert!(anims.contains(&0x93), "{anims:x?}");
    }

    #[test]
    fn real_circle_dash_goes_round_the_opponent() {
        use crate::btl_input::pad::{CROSS, L2, START};
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
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        // The bearing of the fighter seen from the opponent.
        let bearing = |b: &Battle| {
            let (a, o) = (b.roster.chars[0].pose.pos, b.roster.chars[1].pose.pos);
            (a[0] - o[0]).atan2(a[2] - o[2])
        };
        let before = bearing(&battle);
        // The homing dash (L2 + Cross), and the same again while it flies: the circle dash 0x33.
        let (mut mine, mut anims) = (Vec::new(), Vec::new());
        for i in 0..100 {
            let held = if i < 2 || (20..22).contains(&i) {
                CROSS | L2
            } else {
                0
            };
            frame(&mut battle, held);
            let a = &battle.roster.chars[0];
            mine.push(a.act.current);
            anims.push(a.anim.cur);
        }
        mine.dedup();
        anims.dedup();
        assert_eq!(mine, [0x19, 0x33, 0xB], "{mine:x?}");
        assert!(
            anims.windows(3).any(|w| w == [0x2D, 0x19, 0x1A]),
            "{anims:x?}"
        );
        let turned = crate::btl_input::wrap_angle(bearing(&battle) - before).abs();
        assert!(turned > 2.0, "went round by {turned} rad");
    }

    #[test]
    fn real_grab_throws_the_opponent() {
        use crate::btl_input::pad::{CROSS, START, UP};
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
        let gap = |b: &Battle| crate::btl_chr::opp_distance(&b.roster.chars[0], &b.roster.chars[1]);
        for _ in 0..300 {
            if gap(&battle) < 25.0 {
                break;
            }
            frame(&mut battle, UP);
        }
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        // A double tap of the dash button when close: the step in, then the grab 0xB4. It catches: the thrower
        // plays 0xB7 holding the victim in 0xB8, whose motion is the thrower's 0x97 moved to its skeleton; the
        // throw's damage comes from the queue, and the victim is let go along the launch angles, blown away (0xD5)
        // into a wall (0xE7).
        let health = battle.roster.chars[1].gauge.health;
        let (mut mine, mut theirs) = (Vec::new(), Vec::new());
        let (mut borrowed, mut held, mut nearest) = (false, false, f32::MAX);
        for i in 0..120 {
            frame(&mut battle, if i == 0 || i == 2 { CROSS } else { 0 });
            let (a, b) = (&battle.roster.chars[0], &battle.roster.chars[1]);
            mine.push(a.act.current);
            theirs.push(b.act.current);
            if b.act.current == 0xB8 {
                borrowed |= b.anim.cur == 0x97 && b.obj.layers[0].borrowed;
                held |= b.flags.test(0x23);
                nearest = nearest.min(gap(&battle));
            }
        }
        mine.dedup();
        theirs.dedup();
        assert_eq!(&mine[..4], &[0x1E, 0xB4, 0xB7, 0xB], "{mine:x?}");
        let at = theirs.iter().position(|&a| a == 0xB8).expect("not thrown");
        assert_eq!(&theirs[at..at + 3], &[0xB8, 0xD5, 0xE7], "{theirs:x?}");
        assert!(borrowed, "the victim did not play the thrower's motion");
        assert!(held, "the victim was not held");
        assert!(nearest < 20.0, "held {nearest} away");
        assert!(battle.roster.chars[1].gauge.health < health);
    }

    #[test]
    fn real_homing_dashes_clash_and_the_faster_masher_wins() {
        use crate::btl_input::pad::{CROSS, L2, START, UP};
        // Both fighters homing dash (L2 + Cross) into each other: clash B (both in 0xFA). The side that taps a
        // direction counts more and wins (0x175); the other loses (0x176), takes damage and is blown away (0xD5).
        for masher in 0..2 {
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
            let mut frame = |battle: &mut Battle, a: u32, b: u32| {
                let start = a & START != 0 && battle.prev_held[0] & START == 0;
                battle.run_frame(start, pads(a, b), 0, &mut libc, &mut random, None, false);
            };
            for i in 0..200 {
                let skip =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 }, 0);
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            for _ in 0..20 {
                frame(&mut battle, 0, 0);
            }
            let health = [0, 1].map(|i| battle.roster.chars[i].gauge.health);
            let mut acts = [Vec::new(), Vec::new()];
            let mut states = Vec::new();
            for i in 0..140 {
                let dash = if i < 2 { CROSS | L2 } else { 0 };
                let tap = if i > 10 && i % 4 == 0 { UP } else { 0 };
                let (a, b) = if masher == 0 {
                    (dash | tap, dash)
                } else {
                    (dash, dash | tap)
                };
                frame(&mut battle, a, b);
                for (k, v) in acts.iter_mut().enumerate() {
                    v.push((
                        battle.roster.chars[k].act.current,
                        battle.roster.chars[k].anim.cur,
                    ));
                }
                states.push(battle.roster.clash.state);
            }
            states.dedup();
            assert_eq!(&states[..4], &[0, 7, 8, 0], "{states:?}");
            let (w, l) = (masher, masher ^ 1);
            assert!(acts[w].contains(&(0xFA, 0x175)), "{:x?}", acts[w]);
            assert!(acts[l].contains(&(0xFA, 0x176)), "{:x?}", acts[l]);
            assert!(acts[l].iter().any(|a| a.0 == 0xD5), "{:x?}", acts[l]);
            assert!(battle.roster.chars[l].gauge.health < health[l]);
            assert_eq!(battle.roster.chars[w].gauge.health, health[w]);
        }
    }

    #[test]
    fn real_blast_2_beam_fires_on_release_and_hits_six_times() {
        use crate::btl_input::pad::{L2, START, TRIANGLE};
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
        // Charge ki (L2) to afford Goku's first Blast 2 (slot 2: 60000 ki, kind 2, the chargeable 0x10F).
        for _ in 0..120 {
            frame(&mut battle, L2);
        }
        for _ in 0..5 {
            frame(&mut battle, 0);
        }
        assert_eq!(battle.roster.chars[0].gauge.ki, 100_000);
        // L2 + Triangle starts it (L2 alone for two frames is the ki charge 0x37); holding Triangle charges (motion
        // 0x106), letting go (flag 0xA2's button command) fires (0x107), then the firing loop and the end. The
        // Kamehameha (effect type 0, capsule records, hit limit 5 at 3-frame intervals) leaves the muzzle on the
        // motion's FIRE event, holds on the opponent for its six hits, the last blowing it away (0xCF), and its end
        // raises flag 0xA8, which ends the firing loop.
        let (mut acts, mut fire_at, mut beam_at, mut end_at) = (Vec::new(), None, None, None);
        let mut hits = Vec::new();
        let mut health = battle.roster.chars[1].gauge.health;
        for i in 0..110 {
            let held = match i {
                2..4 => L2,
                4..6 => L2 | TRIANGLE,
                6..40 => TRIANGLE,
                _ => 0,
            };
            frame(&mut battle, held);
            let a = &battle.roster.chars[0];
            let o = &battle.roster.chars[1];
            acts.push((a.act.current, a.anim.cur));
            if fire_at.is_none() && a.anim.cur == 0x107 {
                fire_at = Some(i);
            }
            if beam_at.is_none() && !battle.roster.scene.tech.beams_of(0).is_empty() {
                beam_at = Some(i);
            }
            if end_at.is_none() && a.anim.cur == 0x10B {
                end_at = Some(i);
            }
            if o.gauge.health < health {
                hits.push((i, health - o.gauge.health, o.react.reaction));
                health = o.gauge.health;
            }
        }
        acts.dedup();
        assert_eq!(
            acts,
            [
                (0xB, 0),
                (0x37, 0x34),
                (0x10F, 0x105),
                (0x10F, 0x106),
                (0x10F, 0x107),
                (0x10F, 0x108),
                (0x10F, 0x10B),
                (0xB, 0)
            ],
            "{acts:x?}"
        );
        let fire_at = fire_at.unwrap();
        assert!((40..44).contains(&fire_at), "fired at {fire_at}");
        assert_eq!(beam_at, Some(48));
        assert_eq!(hits.len(), 6, "{hits:?}");
        assert!(hits.iter().all(|h| h.1 == hits[0].1), "{hits:?}");
        assert_eq!(hits[0].0, 53);
        // Three frames apart; the knock-away of the last hit is reaction 0xF, the others 3.
        assert!(hits.windows(2).all(|w| w[1].0 - w[0].0 == 3), "{hits:?}");
        assert!(hits[..5].iter().all(|h| h.2 == 3), "{hits:?}");
        assert_eq!(hits[5].2, 0xF);
        assert!(end_at.is_some_and(|e| e < hits[5].0 + 5), "{end_at:?}");
        assert!(battle.roster.scene.tech.beams_of(0).is_empty());
        assert_eq!(battle.roster.chars[0].gauge.ki, 40_000);
    }

    #[test]
    fn real_blast_2_beam_is_guarded() {
        use crate::btl_input::pad::{CIRCLE, L2, START, TRIANGLE};
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
        let mut frame = |battle: &mut Battle, p1: u32, p2: u32| {
            let start = p1 & START != 0 && battle.prev_held[0] & START == 0;
            battle.run_frame(start, pads(p1, p2), 0, &mut libc, &mut random, None, false);
        };
        for i in 0..200 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 }, 0);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        for _ in 0..120 {
            frame(&mut battle, L2, CIRCLE);
        }
        for _ in 0..5 {
            frame(&mut battle, 0, CIRCLE);
        }
        // P2 guards (0x38) throughout; P1 fires Goku's Kamehameha uncharged. Every one of its six hits is guarded
        // (flag 0x66 or 0x67 by the technique's guard kind) for the guarded damage, less than a plain hit's 1170.
        let mut health = battle.roster.chars[1].gauge.health;
        let (mut drops, mut guarded) = (Vec::new(), 0);
        for i in 0..110 {
            let p1 = match i {
                2..4 => L2,
                4..6 => L2 | TRIANGLE,
                _ => 0,
            };
            frame(&mut battle, p1, CIRCLE);
            let o = &battle.roster.chars[1];
            if o.gauge.health < health {
                drops.push(health - o.gauge.health);
                health = o.gauge.health;
                if o.flags.test(0x66) || o.flags.test(0x67) {
                    guarded += 1;
                }
            }
        }
        assert_eq!(drops.len(), 6, "{drops:?}");
        assert_eq!(guarded, drops.len(), "{drops:?}");
        assert!(
            drops.iter().all(|&d| d == drops[0] && d < 1170),
            "{drops:?}"
        );
        assert_eq!(battle.roster.chars[1].act.current, 0x38);
    }

    #[test]
    fn real_volley_fires_ten_steered_shots() {
        use crate::btl_input::pad::{L2, START, TRIANGLE};
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 45;
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
        for _ in 0..120 {
            frame(&mut battle, L2);
        }
        for _ in 0..5 {
            frame(&mut battle, 0);
        }
        // Character 45's slot 2 (technique 0x184, effect type 1): the beam action's firing motion fires one shot on
        // each of its ten FIRE events; each is spread round the aim and steered at P2 (1 degree a frame). Nine hit
        // for equal damage (the ninth passes beside P2 while it reels), the last shot (the motion's last FIRE)
        // knocking it away. The volley's end raises flag 0xA8.
        let mut health = battle.roster.chars[1].gauge.health;
        let (mut hits, mut most, mut end_at) = (Vec::new(), 0, None);
        for i in 0..120 {
            let held = match i {
                2..4 => L2,
                4..6 => L2 | TRIANGLE,
                _ => 0,
            };
            frame(&mut battle, held);
            let a = &battle.roster.chars[0];
            let o = &battle.roster.chars[1];
            most = most.max(battle.roster.scene.tech.shots.of(0).len());
            if end_at.is_none() && a.anim.cur == 0x10B {
                end_at = Some(i);
            }
            if o.gauge.health < health {
                hits.push((health - o.gauge.health, o.react.reaction));
                health = o.gauge.health;
            }
        }
        assert_eq!(hits.len(), 9, "{hits:?}");
        assert!(hits.iter().all(|h| h.0 == hits[0].0), "{hits:?}");
        assert!(hits[..8].iter().all(|h| h.1 == 3), "{hits:?}");
        assert_eq!(hits[8].1, 0xF);
        assert!((2..=4).contains(&most), "{most}");
        assert!(end_at.is_some_and(|e| e < 90), "{end_at:?}");
        assert!(battle.roster.scene.tech.shots.of(0).is_empty());
        assert_eq!(battle.roster.chars[0].act.current, 0xB);
    }

    /// Fires character `chara`'s slot 2 technique (L2 + Triangle, after charging ki) at an idle character 1 for
    /// `frames` frames: P2's health drops with its reaction, the frame the technique's end motion
    /// (`super_anim(2, 6)`) starts, and P1's action at the end.
    fn fire_slot_2(chara: i32, frames: usize) -> Option<(Vec<(i32, i32)>, Option<usize>, i32)> {
        fire_slot(chara, 2, frames)
    }

    /// The same for slot 2 (neutral) or 3 (up); the end motion is that of the slot.
    fn fire_slot(
        chara: i32,
        slot: i32,
        frames: usize,
    ) -> Option<(Vec<(i32, i32)>, Option<usize>, i32)> {
        fire_slot_with(chara, slot, frames, |_, _, _| {})
    }

    /// `fire_slot`, calling `each` with the battle after every frame of the technique.
    fn fire_slot_with(
        chara: i32,
        slot: i32,
        frames: usize,
        mut each: impl FnMut(&Battle, usize, &BattleFrame),
    ) -> Option<(Vec<(i32, i32)>, Option<usize>, i32)> {
        use crate::btl_input::pad::{L2, START, TRIANGLE, UP};
        let dir = if slot == 3 { UP } else { 0 };
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = chara;
        setup.sides[1].chara = 1;
        setup.sides[1].pad = 1;
        let mut battle = real_battle(&setup)?;
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
        for _ in 0..120 {
            frame(&mut battle, L2);
        }
        for _ in 0..5 {
            frame(&mut battle, 0);
        }
        let end_anim = slot * 0x22 + 0xC1 + 6;
        let mut health = battle.roster.chars[1].gauge.health;
        let (mut hits, mut end_at) = (Vec::new(), None);
        for i in 0..frames {
            let held = match i {
                2..4 => L2 | dir,
                4..6 => L2 | TRIANGLE | dir,
                _ => 0,
            };
            let out = frame(&mut battle, held);
            each(&battle, i, &out);
            let a = &battle.roster.chars[0];
            let o = &battle.roster.chars[1];
            if end_at.is_none() && a.anim.cur == end_anim {
                end_at = Some(i);
            }
            if o.gauge.health < health {
                hits.push((health - o.gauge.health, o.react.reaction));
                health = o.gauge.health;
            }
        }
        Some((hits, end_at, battle.roster.chars[0].act.current))
    }

    #[test]
    fn real_prop_shot_flies_and_hits() {
        // Character 81's slot 2 (technique 0x166, effect type 6): the disc leaves on the FIRE event and hits P2
        // once, its last hit knocking it away; its stage test uses the radius times the hit scale (0.05), so it
        // skims over the floor. Its end raises flag 0xA8.
        let Some((hits, end_at, act)) = fire_slot_2(81, 150) else {
            return;
        };
        assert_eq!(hits, [(7730, 0xF)]);
        assert_eq!(end_at, Some(74));
        assert_eq!(act, 0xB);
    }

    #[test]
    fn real_sweep_crosses_the_opponent() {
        // Character 118's slot 2 (technique 0x1A0, effect type 3): from the FIRE event the beam's end sweeps round
        // the firing node and crosses P2 once, knocking it away; the sweep ends 0.6 s later, raising flag 0xA8.
        let Some((hits, end_at, act)) = fire_slot_2(118, 150) else {
            return;
        };
        assert_eq!(hits, [(6360, 0xF)]);
        assert_eq!(end_at, Some(62));
        assert_eq!(act, 0xB);
    }

    #[test]
    fn real_multi_releases_its_pieces() {
        // Character 95's slot 2 (technique 0x188, effect type 5 with flag 0x8000): two disc pieces grown in the
        // hands on START, released one by one on FIRE along the aim; the second, the technique's last hit, knocks
        // P2 away. Character 60's slot 2 (technique 0x1A3): one piece fired from the muzzle on FIRE.
        let Some((hits, end_at, act)) = fire_slot_2(95, 120) else {
            return;
        };
        assert_eq!(hits, [(2960, 3), (2960, 0xF)]);
        assert_eq!(end_at, Some(56));
        assert_eq!(act, 0xB);
        let Some((hits, end_at, act)) = fire_slot_2(60, 120) else {
            return;
        };
        assert_eq!(hits, [(5450, 0xF)]);
        assert_eq!(end_at, Some(50));
        assert_eq!(act, 0xB);
    }

    #[test]
    fn real_disc_ki_blasts_grow_fly_and_hit() {
        use crate::btl_input::pad::{START, TRIANGLE};
        // The charged ki blast (0xAF) of character 25 is of type 4: request 0x17 grows a disc in the hand while it
        // charges, the fire event throws it. Character 39's is of type 5: the fire event sends a disc from the
        // event's node. Each hits the idle P2 once, knocking it away, and is gone.
        for (chara, held_at, hit) in [(25, Some(6), (39, 1430)), (39, None, (34, 1550))] {
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
            let mut frame = |battle: &mut Battle, held: u32| {
                let start = held & START != 0 && battle.prev_held[0] & START == 0;
                battle.run_frame(start, pads(held), 0, &mut libc, &mut random, None, false);
            };
            for i in 0..200 {
                let skip =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            let mut health = battle.roster.chars[1].gauge.health;
            let (mut first_disc, mut hits) = (None, Vec::new());
            for i in 0..80 {
                frame(&mut battle, if i < 60 { TRIANGLE } else { 0 });
                if first_disc.is_none() && !battle.roster.scene.tech.discs.of(0).is_empty() {
                    first_disc = Some(i);
                }
                let o = &battle.roster.chars[1];
                if o.gauge.health < health {
                    hits.push((i, health - o.gauge.health, o.react.reaction));
                    health = o.gauge.health;
                }
            }
            assert_eq!(hits, [(hit.0, hit.1, 0xF)], "character {chara}");
            if let Some(at) = held_at {
                assert_eq!(first_disc, Some(at), "character {chara}");
            } else {
                assert!(
                    first_disc.is_some_and(|f| f > 20),
                    "character {chara}: {first_disc:?}"
                );
            }
            assert!(battle.roster.scene.tech.discs.of(0).is_empty());
        }
    }

    #[test]
    fn real_ring_shots() {
        // Effect type 4 under the long beam action (0x10C..): its first wait ends on flag 0xA8 (the record placed,
        // or the carrier's path flown), the second on 0xA9 at the ring shot's end. Characters 147 / 80 (slot 2,
        // techniques 0x197 / 0x200): a multi-hit record at P2's node 3 from event 0x10. Characters 25 / 75 (slot
        // 3, techniques 0x156 / 0x1DC): the carrier's spline over P2, then a volley of ten shots fired at it from
        // there, the spread ones missing.
        for (c, slot, dmg, n, end) in [
            (147, 2, 1370, 6, 127),
            (80, 2, 1060, 6, 104),
            (25, 3, 780, 4, 105),
            (75, 3, 780, 4, 107),
        ] {
            let Some((hits, end_at, act)) = fire_slot(c, slot, 140) else {
                return;
            };
            assert_eq!(hits.len(), n, "character {c}: {hits:?}");
            assert!(hits.iter().all(|h| h.0 == dmg), "character {c}: {hits:?}");
            assert!(
                hits[..n - 1].iter().all(|h| h.1 == 3),
                "character {c}: {hits:?}"
            );
            assert_eq!(hits[n - 1].1, 0xF, "character {c}");
            assert_eq!(end_at, Some(end), "character {c}");
            assert_eq!(act, 0xB, "character {c}");
        }
        // Character 137's slot 3 (technique 0x18D): eight shots slide to two rings round the fighter, held
        // harmless; once all are there flag 0xA8 comes, and event 0x40 launches them, the later the earlier made,
        // each along the velocity it was made with (they do not home), so three of them hit P2.
        let Some((hits, end_at, _)) = fire_slot(137, 3, 140) else {
            return;
        };
        assert_eq!(
            hits.iter().map(|h| h.0).collect::<Vec<_>>(),
            [910, 910, 910]
        );
        assert_eq!(end_at, Some(115));
    }

    #[test]
    fn real_beam_struggle_is_won_by_mashing() {
        use crate::btl_input::pad::{DOWN, L2, LEFT, RIGHT, START, TRIANGLE, UP};
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 1;
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
        let mut frame = |battle: &mut Battle, p1: u32, p2: u32| {
            let start = p1 & START != 0 && battle.prev_held[0] & START == 0;
            battle.run_frame(start, pads(p1, p2), 0, &mut libc, &mut random, None, false);
        };
        for i in 0..200 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 }, 0);
            if battle.seq.state == SeqState::Fight {
                break;
            }
        }
        for _ in 0..120 {
            frame(&mut battle, L2, L2);
        }
        for _ in 0..5 {
            frame(&mut battle, 0, 0);
        }
        // Both fire the beam of slot 2 (technique 0x1CE); P2's start is longer, so P1's beam passes it while it
        // is invulnerable, and the two beams meet head-on: both are held in a struggle (clash A, 0x130). From 60
        // frames on P1's mashing (a direction every other frame) gives it the lead, which then grows to 71: P2
        // loses (0x104, the damage of both techniques) and falls, P1 is free, the struggle and the beams are over.
        let mash = [LEFT, UP, RIGHT, DOWN];
        let health = battle.roster.chars[1].gauge.health;
        let (mut struggle_at, mut lost_at, mut moved) = (None, None, false);
        let (mut clash_frames, mut cut_frames) = (0, 0);
        let mut first_pos = None;
        for i in 0..300 {
            let (p1, p2) = match i {
                2..4 => (L2, L2),
                4..6 => (L2 | TRIANGLE, L2 | TRIANGLE),
                _ if lost_at.is_some() => (0, 0),
                _ => (mash[(i / 2) % 4], 0),
            };
            frame(&mut battle, p1, p2);
            let r = &battle.roster;
            if let Some(s) = r.scene.struggle {
                if struggle_at.is_none() {
                    struggle_at = Some(i);
                    first_pos = Some(s.pos);
                }
                moved |= first_pos.is_some_and(|p| p != s.pos);
            }
            if r.clash.state != 0 {
                assert_eq!(r.chars[0].act.current, 0x130);
                assert_eq!(r.chars[1].act.current, 0x130);
            }
            // While the clash runs, its one-frame cuts (with the screen's priority, flag 0xD3) hold one of the
            // fighter cameras.
            if (2..=4).contains(&r.clash.state) {
                clash_frames += 1;
                cut_frames += i32::from(r.chars.iter().any(|c| c.flags.test(0xD3)));
            }
            if lost_at.is_none() && r.chars[1].act.current == 0x104 {
                lost_at = Some(i);
            }
        }
        let r = &battle.roster;
        let struggle_at = struggle_at.expect("no struggle");
        let lost_at = lost_at.expect("P2 did not lose");
        assert!(moved, "the struggle point never moved");
        // (The frame state 1 becomes 2 sets no cut yet.)
        assert!(
            clash_frames > 100 && cut_frames == clash_frames - 1,
            "{cut_frames} / {clash_frames}"
        );
        assert!(lost_at > struggle_at + 106, "{struggle_at} {lost_at}");
        assert_eq!(health - r.chars[1].gauge.health, 10910);
        assert!(!(0x130..=0x132).contains(&r.chars[0].act.current));
        assert_ne!(r.chars[0].act.current, 0x104);
        // The loser, still undamaged, changed to its damaged model.
        assert_eq!(r.chars[1].gauge.variant, 1);
        assert!(r.scene.struggle.is_none());
        assert!(r.scene.records.is_empty());
    }

    /// Runs character `chara`'s technique of `slot` (charged first; `near`: from beside the opponent) against an
    /// idle Goku / Vegeta: the action pairs as they change (frame, P1 action, P2 action), the health P2 lost and
    /// the technique's table damage.
    fn run_technique(
        chara: i32,
        slot: i32,
        frames: usize,
        near: bool,
    ) -> Option<(Vec<(usize, i32, i32)>, i32, i32, [usize; 3], [i32; 4])> {
        run_technique_watched(chara, slot, frames, near, &mut |_| {})
    }

    /// `run_technique`, calling `watch` after every frame of the technique.
    fn run_technique_watched(
        chara: i32,
        slot: i32,
        frames: usize,
        near: bool,
        watch: &mut dyn FnMut(&Battle),
    ) -> Option<(Vec<(usize, i32, i32)>, i32, i32, [usize; 3], [i32; 4])> {
        use crate::btl_input::pad::{DOWN, L2, START, TRIANGLE, UP};
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = chara;
        setup.sides[1].chara = if chara == 0 { 1 } else { 0 };
        setup.sides[1].pad = 1;
        let mut battle = real_battle(&setup)?;
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
        if slot == 4 {
            // A blast stock for the powered-up mode an ultimate needs.
            battle.roster.chars[0].gauge.blast = 100_000;
        }
        for i in 0..600 {
            frame(&mut battle, L2);
            if i >= 150 && (slot != 4 || battle.roster.chars[0].flags.test(6)) {
                break;
            }
        }
        for _ in 0..5 {
            frame(&mut battle, 0);
        }
        // Up to the opponent (forward on the stick) for a technique that strikes where it stands.
        for _ in 0..300 {
            let r = &battle.roster;
            let d = crate::btl_chr::opp_distance_xz(&r.chars[0], &r.chars[1]);
            if !near || d < 30.0 {
                break;
            }
            frame(&mut battle, UP);
        }
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        let dir = match slot {
            3 => UP,
            4 => DOWN,
            _ => 0,
        };
        let health = battle.roster.chars[1].gauge.health;
        let mut log: Vec<(usize, i32, i32)> = Vec::new();
        let mut parts = [0usize; 3];
        for i in 0..frames {
            let p1 = match i {
                2..4 => L2,
                4..6 => L2 | dir | TRIANGLE,
                _ => 0,
            };
            frame(&mut battle, p1);
            watch(&battle);
            let r = &battle.roster;
            let (a, b) = (r.chars[0].act.current, r.chars[1].act.current);
            let live: usize = r.scene.fx.counts(None).iter().sum();
            let shots = r.scene.tech.shots_of(0);
            parts = [parts[0].max(live), live, parts[2].max(shots)];
            if log.last().map(|l| (l.1, l.2)) != Some((a, b)) {
                log.push((i, a, b));
            }
        }
        let table = battle.assets.fighters[0].tables.tech().damage(slot);
        let r = &battle.roster.chars;
        let state = [
            r[0].gauge.variant,
            r[1].gauge.variant,
            battle.assets.fighters[0].chara,
            i32::from(r[0].gauge.body_changed),
        ];
        Some((
            log,
            health - battle.roster.chars[1].gauge.health,
            table,
            parts,
            state,
        ))
    }

    /// A battle of `chara` (side 0, changes allowed when `allowed`) against an idle Goku, run into the fight.
    fn transform_battle(chara: i32, allowed: bool) -> Option<Battle> {
        use crate::btl_input::pad::START;
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = chara;
        setup.sides[0].change_allowed = i32::from(allowed);
        setup.sides[1].pad = 1;
        let mut battle = real_battle(&setup)?;
        battle.assets.chara_bits = [u64::MAX; 3];
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut fight = 0;
        for i in 0..400 {
            if battle.seq.state == SeqState::Fight {
                fight += 1;
                if fight > 40 {
                    break;
                }
            }
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            let held = if skip && i % 2 == 0 { START } else { 0 };
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            battle.run_frame(start, pads, 0, &mut libc, &mut random, None, false);
        }
        Some(battle)
    }

    /// R3 tapped (with `dir` held), then `frames` more: the (frame, action, side 0's character) changes, whether
    /// time stood still in the change and the opponent's position kept.
    fn run_transform(
        battle: &mut Battle,
        dir: u32,
        frames: usize,
    ) -> (Vec<(usize, i32, i32)>, bool, bool) {
        use crate::btl_input::pad::R3;
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut log: Vec<(usize, i32, i32)> = Vec::new();
        let (mut stopped, mut opp_still) = (false, true);
        let opp = battle.roster.chars[1].pose.pos;
        for i in 0..frames {
            let held = match i {
                2..4 => dir | R3,
                4..6 => dir,
                _ => 0,
            };
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
            let a = battle.roster.chars[0].act.current;
            if (0xEC..=0xF0).contains(&a) {
                stopped |= battle.roster.change.time_stop;
                opp_still &= battle.roster.chars[1].pose.pos == opp;
            }
            let entry = (a, battle.assets.fighters[0].chara);
            if log.last().map(|l| (l.1, l.2)) != Some(entry) {
                log.push((i, entry.0, entry.1));
            }
        }
        (log, stopped, opp_still)
    }

    #[test]
    fn real_technique_model_changes() {
        // Through the change queue: character 37's self-destruct Ultimate Blast (flag 0x2000) changes the user to
        // its damaged model; the rush sequences of characters 40 (the victim's, flag 0x80000) and 28 (both) reload
        // the damaged models in their last step; character 86's technique 0x280 turns the user into one of six
        // fixed characters by the frame counter (here 13, costume 2), its body changed.
        let expect: [(i32, fn([i32; 4]) -> bool); 4] = [
            (37, |s| s[0] == 1 && s[1] == 0),
            (40, |s| s[0] == 0 && s[1] == 1),
            (28, |s| s[0] == 1 && s[1] == 1),
            (86, |s| s[2] == 13 && s[3] == 1),
        ];
        for (chara, ok) in expect {
            let Some((log, _, _, parts, state)) = run_technique(chara, 4, 700, false) else {
                return;
            };
            assert!(ok(state), "{chara}: {state:?} {log:x?}");
            assert_eq!(log.last().map(|l| l.1), Some(0xB), "{chara}: {log:x?}");
            assert_eq!(parts[1], 0, "{chara}");
        }
    }

    /// A team battle run into the fight: side `team_side` has the extra members `team` (after the lead, Goku).
    fn team_battle(team_side: usize, team: &[(i32, i32)]) -> Option<Battle> {
        let mut setup = BattleSetup::default();
        for (i, &m) in team.iter().enumerate() {
            setup.sides[team_side].team[i] = m;
        }
        setup.sides[team_side].team_count = team.len() as i32;
        setup.sides[team_side].change_allowed = 1;
        setup.sides[1].pad = 1;
        fight_battle(&setup)
    }

    /// The battle of `setup` run 40 frames into the fight (side 0 pressing START through the intro).
    fn fight_battle(setup: &BattleSetup) -> Option<Battle> {
        use crate::btl_input::pad::START;
        let mut battle = real_battle(setup)?;
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut fight = 0;
        for i in 0..400 {
            if battle.seq.state == SeqState::Fight {
                fight += 1;
                if fight > 40 {
                    break;
                }
            }
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            let held = if skip && i % 2 == 0 { START } else { 0 };
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            battle.run_frame(start, pads, 0, &mut libc, &mut random, None, false);
        }
        Some(battle)
    }

    #[test]
    fn real_stage_change_swaps_the_stage_behind_a_fade() {
        // The first stage whose environment names a change (`changeKind` 1: to stage 0xF, 2: to stage 3).
        let Some(mut probe) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = probe.loader.as_mut().unwrap();
        let Some((from, to)) = (0..0x27).find_map(|s| {
            let kind = load_stage(read, s).ok()?.stage.change_kind;
            let to = match kind {
                1 => 0xF,
                2 => 3,
                _ => return None,
            };
            (to != s).then_some((s, to))
        }) else {
            panic!("no stage changes");
        };
        let mut setup = BattleSetup::default();
        setup.stage = from;
        setup.stage_change = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let rev = battle.assets.stage_rev;
        battle.request_stage_change(to);
        assert_eq!(battle.assets.stage_id, to);
        let (mut loading_frames, mut cam_frames, mut frames) = (0, 0, 0);
        let (mut burst_frames, mut drawn_model, mut drawn_ptcl, mut burst_sounds) = (0, 0, 0, 0);
        let time = battle.seq.time_left();
        while battle.stage_job.is_some() && frames < 400 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            if battle.loading {
                loading_frames += 1;
                assert!(
                    frame.stage.is_empty(),
                    "the stage is not drawn while it loads"
                );
                cam_frames += i32::from(battle.demo.is_active());
            }
            // The transition: its model writes depth, its particles (once they start) do not; its sounds.
            if battle.burst.as_ref().is_some_and(|b| b.frame > 12) {
                burst_frames += 1;
                let model = frame.effects.iter().filter(|t| t.zwrite).count();
                let ptcl = frame.effects.iter().filter(|t| !t.zwrite).count();
                drawn_model = drawn_model.max(model);
                drawn_ptcl = drawn_ptcl.max(ptcl);
            }
            burst_sounds += frame
                .sounds
                .iter()
                .filter(|s| {
                    matches!(
                        s,
                        TitleSound::SeEx {
                            id: 0x4A,
                            pitch: -0x352,
                            ..
                        }
                    )
                })
                .count();
            frames += 1;
        }
        assert!(burst_frames > 100, "{burst_frames}");
        assert!(
            drawn_model > 100 && drawn_ptcl > 100,
            "{drawn_model} {drawn_ptcl}"
        );
        assert!(burst_sounds >= 2, "{burst_sounds}");
        // The transition's 150 frames, the fade out (30) and the steps around them.
        assert!(battle.stage_job.is_none() && !battle.loading, "{frames}");
        assert!((180..200).contains(&loading_frames), "{loading_frames}");
        // The transition's camera animation (pack entry 1) holds the view meanwhile.
        assert!(cam_frames > 150, "{cam_frames}");
        assert_eq!(
            battle.seq.time_left(),
            time,
            "the clock stops while the stage loads"
        );
        assert_eq!(battle.assets.stage_rev, rev + 1);
        assert_eq!(battle.assets.stage.stage_id, to);
        // The fighters stand at the new stage's starting places.
        for side in 0..2 {
            let p = battle.assets.stage.starting_placements[side].position;
            let c = &battle.roster.chars[side];
            assert!((c.pose.pos[0] - p[0]).abs() < 1.0 && (c.pose.pos[2] - p[2]).abs() < 1.0);
        }
        // A rematch loads the first stage back.
        let mut sounds = Vec::new();
        battle.restart(&mut sounds);
        assert_eq!(battle.assets.stage_id, from);
        assert_eq!(battle.assets.stage.stage_id, from);
    }

    #[test]
    fn real_stages_raise_their_boundary_wall_near_the_edge() {
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = battle.loader.as_mut().unwrap();
        let mut with = 0;
        for s in 0..0x27 {
            let Some((p, textures)) = load_stage(read, s).ok().and_then(|s| s.bound) else {
                continue;
            };
            with += 1;
            assert!(!textures.is_empty(), "stage {s}");
            assert!(
                p.range > 0.0 && p.fade_dist > 0.0 && p.arc > 0.0,
                "stage {s}: {p:?}"
            );
            assert!((0..4).contains(&p.layer), "stage {s}: {p:?}");
        }
        // Stages 4, 27 and 35..38 have none.
        assert_eq!(with, 0x27 - 6);
        // A fighter 50 from the edge: its wall draws, seen from the middle of the stage.
        let mut b = battle.assets.new_bound().unwrap();
        let edge = battle.assets.stage.bounds.radius - 100.0;
        let mut chars = [crate::eft_bound::BoundChr::default(); 2];
        chars[0].node3 = [0.0, -20.0, edge - 50.0];
        b.update(&chars, false);
        let view = View::new([0.0, -20.0, 0.0], [0.0; 3]);
        let ctx = crate::eft::DrawCtx::new(view.world2screen, view.world2view);
        let mut ot = crate::eft::OrderTable::new();
        b.draw(&mut ot, &ctx, &[Default::default(); 2]);
        let tris = ot.into_list();
        assert!(!tris.is_empty());
        assert!(
            tris.iter()
                .all(|t| t.texture == battle.assets.bound.unwrap().1)
        );
        assert!(battle.roster.scene.stage.bound.is_some());
    }

    #[test]
    fn real_stage_8_scrolls_its_sheet_behind_the_stage() {
        let mut setup = BattleSetup::default();
        setup.stage = 8;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let (sheet, base) = battle.assets.scroll.clone().unwrap();
        // Stage 8: size 1, a random direction, two textures after the stage's own.
        assert_eq!((sheet.size, sheet.angle < 0), (1, true));
        assert_eq!(base + 2, battle.assets.stage_textures.len());
        let mut libc = LibcRandom::new(3);
        let mut random = GameRandom::from_key(&[3]);
        let mut shown = 0;
        for _ in 0..30 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            shown += frame
                .stage
                .iter()
                .filter(|b| b.group == 0 && b.order >= 1000 && b.texture >= base)
                .filter(|b| b.vertices.iter().any(|v| v.rgba[3] > 0.0))
                .count();
        }
        assert!(shown > 0);
    }

    #[test]
    fn real_smoke_rises_on_its_stages() {
        // The stages with smoke emitters (stage flag 2 and records in the MEF0 list 1).
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = battle.loader.as_mut().unwrap();
        let smoky: Vec<(i32, usize)> = (0..0x27)
            .filter_map(|s| {
                let st = load_stage(read, s).ok()?.stage;
                (st.stage_flags & 2 != 0 && !st.smoke.is_empty()).then(|| (s, st.smoke.len()))
            })
            .collect();
        assert_eq!(smoky, [(3, 4), (6, 8), (9, 6), (28, 8), (29, 6), (30, 6)]);
        let mut setup = BattleSetup::default();
        setup.stage = 3;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let texture = battle.assets.stage_pack12.as_ref().unwrap()[2];
        let smoke = battle.roster.scene.stage.smoke.as_ref().unwrap();
        // Its emitters ran their first updates at the start: the smoke is there.
        assert!(smoke.count() > 20, "{}", smoke.count());
        let mut libc = LibcRandom::new(4);
        let mut random = GameRandom::from_key(&[4]);
        let mut drawn = 0;
        for _ in 0..10 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            drawn += frame
                .effects
                .iter()
                .filter(|t| t.texture == Some(texture))
                .count();
        }
        // The emitters keep their particles going, and the starting camera sees them.
        assert!(battle.roster.scene.stage.smoke.as_ref().unwrap().count() > 20);
        assert!(drawn > 100, "{drawn}");
        // Stage 3's nine geysers (stage flag 1): within 6 s each has erupted, with texture 1 of the set.
        let column = battle.assets.stage_pack12.as_ref().unwrap()[1];
        assert_eq!(battle.assets.stage.geysers.len(), 9);
        let mut columns = 0;
        for _ in 0..200 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            columns += frame
                .effects
                .iter()
                .filter(|t| t.texture == Some(column))
                .count();
        }
        let states = battle.roster.scene.stage.geysers.as_ref().unwrap().states();
        assert!(states.iter().all(|&s| s != 0), "{states:?}");
        assert!(columns > 0);
    }

    #[test]
    fn real_weather_drifts_on_its_stages() {
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = battle.loader.as_mut().unwrap();
        let stages: Vec<i32> = (0..0x27)
            .filter(|&s| {
                load_stage(read, s)
                    .ok()
                    .is_some_and(|st| st.pack11.is_some() && st.stage.weather.is_some_and(|w| w.on))
            })
            .collect();
        assert_eq!(stages, [1, 5, 8, 14, 19, 21, 25]);
        let mut setup = BattleSetup::default();
        setup.stage = stages[0];
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let textures = battle.assets.stage_pack11.clone().unwrap();
        assert!(battle.roster.scene.stage.weather.is_some());
        let mut libc = LibcRandom::new(7);
        let mut random = GameRandom::from_key(&[7]);
        let mut drawn = 0;
        for _ in 0..60 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            drawn += frame
                .effects
                .iter()
                .filter(|t| t.texture.is_some_and(|i| textures.contains(&i)))
                .count();
        }
        assert!(drawn > 0);
    }

    #[test]
    fn real_surfaces_scroll_on_their_stages() {
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = battle.loader.as_mut().unwrap();
        let mut with = Vec::new();
        for s in 0..0x27 {
            let Some(LoadedSurf {
                meshes,
                textures,
                normals,
                ..
            }) = load_stage(read, s).ok().and_then(|st| st.surf)
            else {
                continue;
            };
            // The reflecting surfaces' normal maps decode.
            assert_eq!(
                normals.is_some(),
                meshes.iter().any(|m| m.flags & 0x800 != 0),
                "stage {s}"
            );
            // Every texture of the surface files decodes; every mesh names one.
            assert!(textures.iter().all(Option::is_some), "stage {s}");
            assert!(
                meshes
                    .iter()
                    .all(|m| m.tex < textures.len() || textures.is_empty()),
                "stage {s}"
            );
            with.push(s);
        }
        // (Stage 38's file has one too, but the stage does not load: its camera animations are cut short.)
        assert_eq!(
            with,
            [0, 2, 3, 7, 10, 11, 12, 14, 15, 17, 18, 21, 22, 23, 32, 33]
        );
        let mut setup = BattleSetup::default();
        setup.stage = 10;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let textures: Vec<usize> = battle
            .assets
            .surf
            .as_ref()
            .unwrap()
            .textures
            .iter()
            .flatten()
            .copied()
            .collect();
        let mut libc = LibcRandom::new(8);
        let mut random = GameRandom::from_key(&[8]);
        let mut drawn = 0;
        for _ in 0..10 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            drawn += frame
                .effects
                .iter()
                .filter(|t| t.texture.is_some_and(|i| textures.contains(&i)))
                .count();
        }
        assert!(drawn > 0);
        // Stage 2's reflecting surface: its two lit textures are written every frame and its triangles draw
        // with them, the specular one added.
        setup.stage = 2;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let (_, out) = battle
            .assets
            .surf
            .as_ref()
            .unwrap()
            .normals
            .clone()
            .unwrap();
        let (mut lit, mut added) = (0, 0);
        for _ in 0..10 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            let updated: Vec<usize> = frame.texture_updates.iter().map(|u| u.0).collect();
            assert_eq!(updated, out);
            lit += frame
                .effects
                .iter()
                .filter(|t| t.texture == Some(out[0]))
                .count();
            added += frame
                .effects
                .iter()
                .filter(|t| t.texture == Some(out[1]) && t.blend == crate::eft::EffectBlend::Add)
                .count();
        }
        assert!(lit > 0 && added > 0, "{lit} {added}");
    }

    #[test]
    fn real_stage_15_storms() {
        let mut setup = BattleSetup::default();
        setup.stage = 15;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let textures = battle.assets.stage_pack14.clone().unwrap();
        assert!(textures.len() >= 6);
        assert!(battle.roster.scene.stage.storm.is_some());
        let mut libc = LibcRandom::new(6);
        let mut random = GameRandom::from_key(&[6]);
        let (mut rain, mut bolts) = (0, 0);
        for _ in 0..120 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            rain += frame
                .effects
                .iter()
                .filter(|t| t.texture.is_none() && t.vertices[0].rgba == [0x60, 0x60, 0x60, 0x20])
                .count();
            bolts += frame
                .effects
                .iter()
                .filter(|t| t.texture.is_some_and(|i| textures.contains(&i)))
                .count();
        }
        // 64 drops a frame (some off screen), and the lightning on the horizon ahead.
        assert!(rain > 1000, "{rain}");
        assert!(bolts > 0);
    }

    #[test]
    fn real_water_stages_splash_and_draw_their_water() {
        // The stages whose files have the water effects (stage pack entry 0x12 and the water record's bit 0).
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let read = battle.loader.as_mut().unwrap();
        let water: Vec<(i32, usize)> = (0..0x27)
            .filter_map(|s| {
                let st = load_stage(read, s).ok()?;
                let w = st.pack12.filter(|_| st.stage.water_fx)?;
                Some((s, w.len()))
            })
            .collect();
        assert!(!water.is_empty());
        // EftWater draws five textures of the set.
        assert!(water.iter().all(|&(_, n)| n >= 5), "{water:?}");
        let stage = water[0].0;
        let mut setup = BattleSetup::default();
        setup.stage = stage;
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let textures = battle.assets.stage_pack12.clone().unwrap();
        let level = battle.assets.stage.water_level.unwrap_or(0.0);
        let mut libc = LibcRandom::new(2);
        let mut random = GameRandom::from_key(&[2]);
        // A splash between the fighters, on the surface.
        let mid: [f32; 3] = std::array::from_fn(|k| {
            (battle.roster.chars[0].pose.pos[k] + battle.roster.chars[1].pose.pos[k]) * 0.5
        });
        let w = battle.roster.scene.stage.water.as_mut().unwrap();
        w.add_splash_at([mid[0], level, mid[2]], 3.0, &mut libc);
        let mut drawn = 0;
        for _ in 0..20 {
            let frame = battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            drawn += frame
                .effects
                .iter()
                .filter(|t| t.texture.is_some_and(|i| textures.contains(&i)))
                .count();
        }
        assert!(drawn > 20, "stage {stage}: {drawn}");
        // The bubbles need six textures: only stage 22's set has them.
        for &(s, n) in &water {
            assert_eq!(n >= 6, s == 22, "stage {s}: {n}");
        }
        assert!(battle.roster.scene.stage.bubbles.is_none());
        setup.stage = 22;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let b = battle.roster.scene.stage.bubbles.as_mut().unwrap();
        b.start_burst(0, 10.0);
        assert_eq!(b.burst(0), 50.0);
        for _ in 0..8 {
            battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
        }
        assert_eq!(
            battle.roster.scene.stage.bubbles.as_ref().unwrap().burst(0),
            0.0
        );
    }

    #[test]
    fn real_auto_rush_runs_its_camera_cut() {
        // Flag 0x7E forces the auto rush (0x45), which asks for the common cut 0x11 / 0x12 by the camera's side.
        let mut setup = BattleSetup::default();
        setup.sides[1].pad = 1;
        let Some(mut battle) = fight_battle(&setup) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut cut = None;
        for _ in 0..40 {
            battle.roster.chars[0].flags.set_held(0x7E);
            battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            let c = &battle.roster.chars[0];
            if c.act.current == 0x45 && crate::chr_cam::is_cut_active(c) {
                cut = Some(c.cut.req_index);
                break;
            }
        }
        assert!(matches!(cut, Some(0x11 | 0x12)), "{cut:?}");
    }

    #[test]
    fn real_camera_stops_at_the_stage() {
        // An eye below the ground (+Y is down) is brought up to where a sphere swept from the target first meets
        // the floor; one in the open stays where it is.
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        battle.with_roster(|_, _, env| {
            let target = [0.0, -50.0, 0.0];
            let (pos, frac) = crate::btl_mgr::trace_camera(env, [0.0, 500.0, 0.0], target);
            let frac = frac.expect("the floor stops the camera");
            assert!(pos[1] < 50.0 && 0.0 < frac && frac < 1.0, "{pos:?} {frac}");
            let open = [0.0, -150.0, -60.0];
            assert_eq!(
                crate::btl_mgr::trace_camera(env, open, target),
                (open, None)
            );
            false
        });
    }

    #[test]
    fn real_knocked_fighter_breaks_a_stage_object() {
        // On the first stage with a destructible object, side 1 knocked about (flag 0xA) is driven into the
        // object's first part: the body sphere pushed out of its mesh breaks it (StgCol_FighterBreakObj), the
        // model draws its broken parts and the collision takes its remains; a rematch puts it back.
        for stage in 0..0x27 {
            let mut setup = BattleSetup::default();
            setup.stage = stage;
            setup.sides[1].pad = 1;
            let Some(mut battle) = fight_battle(&setup) else {
                return;
            };
            let m = &battle.assets.stage_model;
            let Some((obj, center, radius)) = m.objects.iter().enumerate().find_map(|(i, o)| {
                let p = &m.parts[o.parts[0]?];
                Some((i, p.center, p.radius))
            }) else {
                continue;
            };
            let whole = m.drawable.clone();
            let mut libc = LibcRandom::new(1);
            let mut random = GameRandom::from_key(&[1]);
            let start = [center[0] + radius * 3.0 + 100.0, center[1], center[2]];
            let mut break_sound = false;
            let kind = battle.assets.stage_objs.lock().unwrap().kind(obj);
            for f in 0..40 {
                let t = (f as f32 / 20.0).min(1.0);
                let data = side_data(&battle.assets.fighters[1]);
                let c = &mut battle.roster.chars[1];
                c.flags.set_held(0xA);
                c.pose.pos = std::array::from_fn(|k| start[k] + (center[k] - start[k]) * t);
                c.pose_to_obj(false);
                c.obj.calc_matrices(&data.obj);
                let frame = battle.run_frame(
                    false,
                    [PadState::default(); 2],
                    0,
                    &mut libc,
                    &mut random,
                    None,
                    false,
                );
                if battle.assets.stage_objs.lock().unwrap().is_broken(obj) {
                    // The break sound: stage bank (8) sound 8.
                    break_sound = frame
                        .sounds
                        .iter()
                        .any(|s| matches!(s, TitleSound::SeEx { mask: 8, id: 8, .. }));
                    break;
                }
            }
            assert!(break_sound, "stage {stage}");
            assert!(
                battle.assets.stage_objs.lock().unwrap().is_broken(obj),
                "stage {stage}: object {obj} at {center:?} r {radius}"
            );
            assert_ne!(battle.assets.stage_model.drawable, whole, "stage {stage}");
            // Its pieces fall: their matrices move from frame to frame, and seen from above the object the debris
            // pass draws them.
            let falling = |b: &Battle| {
                let objs = b.assets.stage_objs.lock().unwrap();
                let pieces = objs.falling_pieces();
                let mtx: Vec<_> = pieces
                    .iter()
                    .map(|p| objs.node_matrices()[p.node])
                    .collect();
                (pieces, mtx)
            };
            let (pieces, before) = falling(&battle);
            if !battle.assets.stage_model.objects[obj].pieces.is_empty() {
                assert!(!pieces.is_empty(), "stage {stage}");
                // An object of a material raises dust puffs as its pieces start falling.
                assert_eq!(
                    battle.roster.scene.gdust.count() > 0,
                    kind & 7 != 0,
                    "stage {stage}: kind {kind:x}"
                );
                for _ in 0..4 {
                    battle.run_frame(
                        false,
                        [PadState::default(); 2],
                        0,
                        &mut libc,
                        &mut random,
                        None,
                        false,
                    );
                }
                let (pieces, after) = falling(&battle);
                assert!(!pieces.is_empty() && after != before, "stage {stage}");
                let eye = [center[0], center[1] - radius - 400.0, center[2] - 600.0];
                let view = View::new(eye, [0.6, 0.0, 0.0]);
                let objs = battle.assets.stage_objs.lock().unwrap();
                let drawn = crate::stage_draw::draw_debris(
                    &battle.assets.stage_model,
                    &objs.falling_pieces(),
                    objs.node_matrices(),
                    &view,
                    view.eye(),
                );
                assert!(
                    drawn.iter().any(|b| !b.vertices.is_empty()),
                    "stage {stage}"
                );
            }
            let mut sounds = Vec::new();
            battle.restart(&mut sounds);
            battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            assert_eq!(battle.assets.stage_objs.lock().unwrap().broken_mask(), 0);
            assert_eq!(battle.assets.stage_model.drawable, whole, "stage {stage}");
            return;
        }
        panic!("no stage has a destructible object");
    }

    #[test]
    fn real_combat_sounds_play_from_the_loaded_banks() {
        let Some(battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let voices = &battle.assets.common.voices;
        // BTL_VOICE_KINDS kinds, each with at least one line; the file pads the table with empty entries.
        let (used, pad) = voices.split_at(crate::btl_snd::VOICE_KINDS);
        assert!(used.iter().all(|v| v.count >= 1), "{used:?}");
        assert!(used.iter().any(|v| v.count >= 2), "{used:?}");
        assert!(pad.iter().all(|v| v.count == 0), "{pad:?}");
        let mut snd = battle.assets.snd.lock().unwrap_or_else(|e| e.into_inner());
        let chr = &battle.roster.chars[0];
        chr.play_common(&mut snd, 0x44);
        chr.play_own(&mut snd, 0);
        let mut out = Vec::new();
        snd.play_requests(&IDENTITY, true, [0; 2], &mut out);
        let plays: Vec<_> = out
            .iter()
            .filter_map(|s| match s {
                TitleSound::SeEx { mask, id, .. } => Some((*mask, *id)),
                _ => None,
            })
            .collect();
        // Bank 4 is the common one, 0x10 side 0's own (the first player's character bank).
        assert_eq!(plays, vec![(4, 0x44), (0x10, 0)], "{out:?}");
    }

    #[test]
    fn real_actions_request_their_sounds_and_voices() {
        use crate::btl_input::pad::{L2, START};
        let Some(mut battle) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut reqs = Vec::new();
        let mut sounds = Vec::new();
        for i in 0..400 {
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            let held = if skip && i % 2 == 0 {
                START
            } else if battle.seq.state == SeqState::Fight {
                L2
            } else {
                0
            };
            let start = held & START != 0 && battle.prev_held[0] & START == 0;
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            let frame = battle.run_frame(start, pads, 0, &mut libc, &mut random, None, false);
            sounds.extend(frame.sounds);
            let snd = battle.assets.snd.lock().unwrap_or_else(|e| e.into_inner());
            reqs.extend(snd.requests());
        }
        // The ki charge (action 0x37): the start sound 0x15 with voice kind 0xD (an own-bank line, request kind 2
        // for side 0), then the looping 0x16 every frame of the loop.
        assert!(reqs.contains(&(0, 0, 0x15)), "{reqs:x?}");
        assert!(reqs.iter().any(|&(o, k, _)| o == 0 && k == 2), "{reqs:x?}");
        assert!(
            reqs.iter().filter(|r| **r == (0, 0, 0x16)).count() > 30,
            "{reqs:x?}"
        );
        // The stage's looping ambience on the second music player.
        let stage = battle.assets.stage_id as u32;
        assert!(
            sounds.contains(&TitleSound::MapBgm(0x10BA4 + stage)),
            "{sounds:x?}"
        );
        assert!(
            sounds
                .iter()
                .any(|s| matches!(s, TitleSound::MapBgmVolume(v) if *v > 0))
        );
    }

    #[test]
    fn real_technique_requests_its_voice_line_and_sounds() {
        // Goku's first blast 2 (slot 2): its motion raises event 0x8000 for voice line 0x57 (streamed, request kind
        // 5 for side 0) and events 0x10000.. for sounds 0x2D.. of the character's own bank (kind 2).
        let mut reqs = Vec::new();
        let watch = &mut |b: &Battle| {
            let snd = b.assets.snd.lock().unwrap_or_else(|e| e.into_inner());
            reqs.extend(snd.requests());
        };
        if run_technique_watched(0, 2, 300, false, watch).is_none() {
            return;
        }
        assert!(reqs.contains(&(0, 5, 0x57)), "{reqs:x?}");
        assert!(
            reqs.iter()
                .any(|&(o, k, id)| o == 0 && k == 2 && (0x2D..0x31).contains(&id)),
            "{reqs:x?}"
        );
        let mut out = Vec::new();
        let mut snd = crate::btl_snd::CharSound::default();
        snd.request([0.0; 3], 0, 5, 200.0, 1500.0, 0x57);
        snd.play_requests(&IDENTITY, true, [0, 1], &mut out);
        assert!(
            matches!(out[..], [TitleSound::VoiceEx { player: 0, id, .. }] if id == 0xCC32 + 0x57)
        );
    }

    #[test]
    fn real_items_give_bonuses_and_abilities() {
        // From the item table of common file 4: an item starting the member with full ki (ability 0x19), and one
        // raising stat 0 (bonus 1, the melee damage level) without abilities, equipped on side 0.
        let Some(mut probe) = real_battle(&BattleSetup::default()) else {
            return;
        };
        let table = (probe.loader.as_mut().unwrap())(CHARA_TABLE_FILE).unwrap();
        let find = |want: &dyn Fn(&ItemSetStats) -> bool| {
            (1..=400u16).find(|&id| want(&item_set_stats(&table, &[id, 0, 0, 0, 0, 0, 0, 0])))
        };
        let full_ki =
            find(&|s| s.ability[0] & (1 << 0x19) != 0).expect("an item with ability 0x19");
        let melee =
            find(&|s| s.stat[0] > 0 && s.ability == [0; 4]).expect("an item raising stat 0");
        let raise = item_set_stats(&table, &[melee, 0, 0, 0, 0, 0, 0, 0]).stat[0].min(20);
        let mut setup = BattleSetup::default();
        setup.sides[0].items[..2].copy_from_slice(&[full_ki, melee]);
        setup.sides[1].pad = 1;
        let Some(battle) = fight_battle(&setup) else {
            return;
        };
        let [me, opp] = &battle.roster.chars;
        assert!(me.has_ability(0x19) && !opp.has_ability(0x19));
        assert_eq!(me.members[0].items.bonus[1], raise);
        assert_eq!(opp.members[0].items, Default::default());
        // The same character on both sides: only side 0 starts with full ki and a higher melee level.
        assert_eq!(me.gauge.ki, me.gauge.ki_max);
        assert!(opp.gauge.ki < opp.gauge.ki_max);
        assert_eq!(me.stat_level(1) - opp.stat_level(1), raise);
    }

    #[test]
    fn real_combo_counter() {
        use crate::btl_input::pad::{SQUARE, UP};
        // Side 0 rushes the idle opponent: from the second hit the combo part shows the hits and the damage the
        // opponent took, then slides them out when the combo ends.
        let Some(mut battle) = transform_battle(1, false) else {
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
        for _ in 0..300 {
            let r = &battle.roster;
            if crate::btl_chr::opp_distance(&r.chars[0], &r.chars[1]) < 25.0 {
                break;
            }
            battle.run_frame(false, pad(UP), 0, &mut libc, &mut random, None, false);
        }
        let mut most = (0, 0);
        let base = battle.assets.hud_sheets[2].base;
        let mut drawn = false;
        for _ in 0..80 {
            let near =
                crate::btl_chr::opp_distance(&battle.roster.chars[0], &battle.roster.chars[1])
                    < 25.0;
            let held = if near { SQUARE } else { UP };
            battle.run_frame(false, pad(held), 0, &mut libc, &mut random, None, false);
            let f = battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            if let Some(m) = battle.hud.combo().on_show(0) {
                most = most.max(m);
            }
            drawn |= f
                .hud
                .prims
                .iter()
                .any(|p| p.texture.is_some_and(|t| (base..base + 8).contains(&t)));
        }
        assert!(most.0 >= 2 && most.1 > 0, "{most:?}");
        assert!(drawn);
        for _ in 0..200 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
        }
        assert_eq!(battle.hud.combo().on_show(0), None);
    }

    #[test]
    fn real_team_panel() {
        // Side 0 has two members in reserve: its team panel (frame, face of the switch target, plate) is drawn at
        // the top left; side 1 fights alone and has none (its mirrored corner stays empty).
        let Some(mut battle) = team_battle(0, &[(2, 0), (3, 0)]) else {
            return;
        };
        let mut libc = LibcRandom::new(1);
        assert!(battle.assets.hud_cpu.members[0][1].is_some());
        let frame = battle.hud.draw(&mut libc, true);
        let layer = frame.layer.expect("the HUD layer");
        let covered = |x0: usize, x1: usize| {
            (66..86)
                .flat_map(|y| (x0..x1).map(move |x| (y * 512 + x) * 4 + 3))
                .filter(|&i| layer[i] != 0)
                .count()
        };
        assert!(covered(26, 120) > 500, "{}", covered(26, 120));
        assert_eq!(covered(392, 486), 0);
    }

    #[test]
    fn real_team_member_switches() {
        use crate::btl_input::pad::{L3, R3, SQUARE, UP};
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
        // L3 + R3 with a full switch gauge: the lead (Goku, 30000 health) leaves (0xF3), character 1 arrives (0xF4,
        // 0xF5) with its own gauges, the lead's kept.
        let Some(mut battle) = team_battle(0, &[(1, 0)]) else {
            return;
        };
        battle.roster.chars[0].switch_gauge = 100_000;
        battle.roster.chars[0].gauge.health = 30000;
        let mut acts = Vec::new();
        for i in 0..120 {
            let held = if (2..4).contains(&i) { L3 | R3 } else { 0 };
            battle.run_frame(false, pads(held), 0, &mut libc, &mut random, None, false);
            let a = battle.roster.chars[0].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        assert_eq!(acts, [0xB, 0xF3, 0xF4, 0xF5, 0xB], "{acts:x?}");
        let c = &battle.roster.chars[0];
        assert_eq!((battle.assets.fighters[0].chara, c.active), (1, 1));
        assert_eq!(c.gauge.health, c.gauge.health_max);
        assert_eq!(c.members[0].gauge.health, 30000);
        assert_eq!(c.switch_gauge, 0);
        // Without a full gauge nothing happens.
        battle.run_frame(false, pads(L3 | R3), 0, &mut libc, &mut random, None, false);
        battle.run_frame(false, pads(0), 0, &mut libc, &mut random, None, false);
        assert_eq!(battle.roster.chars[0].act.current, 0xB);
        // A knocked-out member is replaced: the second side's lead lies down (0xD8), its next member loads (0xF6),
        // appears (0xF7) and flies in (0xF8); both fighters reset the round and the battle goes on.
        let mut battle = team_battle(1, &[(2, 0)]).unwrap();
        battle.roster.chars[1].gauge.health = 100;
        let (mut acts, mut hit) = (Vec::new(), false);
        for i in 0..400 {
            let near =
                crate::btl_chr::opp_distance(&battle.roster.chars[0], &battle.roster.chars[1])
                    < 25.0;
            let held = if hit {
                0
            } else if near {
                if i % 2 == 0 { SQUARE } else { 0 }
            } else {
                UP
            };
            battle.run_frame(false, pads(held), 0, &mut libc, &mut random, None, false);
            hit |= battle.roster.chars[1].gauge.health <= 0;
            let a = battle.roster.chars[1].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        let tail: Vec<i32> = acts.iter().copied().skip_while(|&a| a != 0xD8).collect();
        assert_eq!(tail, [0xD8, 0xF6, 0xF7, 0xF8, 0xB], "{acts:x?}");
        assert_eq!(battle.seq.state, SeqState::Fight);
        let c = &battle.roster.chars[1];
        assert_eq!((battle.assets.fighters[1].chara, c.active), (2, 1));
        assert_eq!(c.gauge.health, c.gauge.health_max);
    }

    #[test]
    fn real_fusion() {
        use crate::btl_input::pad::{LEFT, R3};
        // Character 44 with character 46 on its team holds R3 with left: fusion 0 (character 48, two stocks,
        // sequence 1: action 0xF1). The partner object and then the fused character load; the two-fighter motion
        // 0x1A2 is the model file's own; the fused fighter has both members' health and the partner leaves.
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 44;
        setup.sides[0].team[0] = (46, 0);
        setup.sides[0].team_count = 1;
        setup.sides[0].change_allowed = 1;
        setup.sides[1].pad = 1;
        let Some(mut battle) = real_battle(&setup) else {
            return;
        };
        battle.assets.chara_bits = [u64::MAX; 3];
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
        let mut fight = 0;
        for i in 0..400 {
            if battle.seq.state == SeqState::Fight {
                fight += 1;
                if fight > 40 {
                    break;
                }
            }
            let skip = battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
            let start = skip && i % 2 == 0;
            battle.run_frame(start, pads(0), 0, &mut libc, &mut random, None, false);
        }
        battle.roster.chars[0].gauge.blast = 300_000;
        let before = [0, 1].map(|i| battle.roster.chars[0].member_gauge(i).health);
        let mut acts = Vec::new();
        for i in 0..340 {
            let held = if (2..20).contains(&i) { LEFT | R3 } else { 0 };
            battle.run_frame(false, pads(held), 0, &mut libc, &mut random, None, false);
            let a = battle.roster.chars[0].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        assert_eq!(acts[..3], [0xB, 0xF1, 0xB], "{acts:x?}");
        let c = &battle.roster.chars[0];
        assert_eq!(battle.assets.fighters[0].chara, 48);
        assert_eq!(c.gauge.health, before[0] + before[1]);
        assert!(c.gauge.fused);
        assert!(!c.members[1].present);
        assert!(c.gauge.blast < 300_000 - 190_000, "{}", c.gauge.blast);
    }

    /// Holds charge (L2) and presses guard (circle; with up for slot 1) to use a skill, then runs `frames`; the
    /// actions side 0 went through.
    fn run_skill(battle: &mut Battle, up: bool, frames: usize) -> Vec<i32> {
        use crate::btl_input::pad::{CIRCLE, L2, UP};
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut acts = vec![battle.roster.chars[0].act.current];
        for i in 0..frames {
            let mut held = match i {
                2..4 => L2,
                4..6 => L2 | CIRCLE,
                _ => 0,
            };
            if up && held != 0 {
                held |= UP;
            }
            let pads = [
                PadState {
                    held,
                    ..PadState::default()
                },
                PadState::default(),
            ];
            battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
            let a = battle.roster.chars[0].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        acts
    }

    #[test]
    fn real_technique_cameras() {
        // Character 1's Blast 2 (slot 2, a beam) takes the camera with his cut 0 (1.2 s, held, priority: flag 0xD3) from the
        // start until the fire motion; the view is his camera's.
        let mut cut_frames = 0;
        let mut ended_after_cut = false;
        let mut was_cut = false;
        let watch = &mut |b: &Battle| {
            let c = &b.roster.chars[0];
            let cut = crate::chr_cam::is_cut_active(c);
            if cut {
                cut_frames += 1;
                assert!(c.flags.test(0xD3));
            }
            ended_after_cut |= was_cut && !cut;
            was_cut = cut;
        };
        let Some((log, ..)) = run_technique_watched(1, 2, 200, false, watch) else {
            return;
        };
        assert!(
            log.iter().any(|l| (0x106..=0x108).contains(&l.1)),
            "{log:x?}"
        );
        assert!(cut_frames > 10, "{cut_frames}");
        assert!(ended_after_cut);
        // A rush technique that connects plays the character's camera animation of its slot on the demo
        // camera, timed by the rush sequence (Goku's slot 3).
        let mut demo = false;
        let watch = &mut |b: &Battle| {
            demo |= b.eft_cam.side() == Some(0);
        };
        let Some((log, dmg, ..)) = run_technique_watched(0, 3, 700, true, watch) else {
            return;
        };
        assert!(dmg > 0, "{log:x?}");
        assert!(demo, "{log:x?}");
        // Transformations: character 1's 0xEC under its cuts 4 / 5 / 6; character 29's 0xEE plays its model's
        // camera animation (flag 0xD7) on the demo camera.
        use crate::btl_input::pad::R3;
        for (chara, action, want_demo) in [(1, 0xEC, false), (29, 0xEE, true)] {
            let mut battle = transform_battle(chara, true).unwrap();
            battle.roster.chars[0].gauge.blast = 300_000;
            let mut libc = LibcRandom::new(1);
            let mut random = GameRandom::from_key(&[1]);
            let (mut seen, mut cut, mut demo) = (false, false, false);
            for i in 0..400 {
                let held = if (2..4).contains(&i) { R3 } else { 0 };
                let pads = [
                    PadState {
                        held,
                        ..PadState::default()
                    },
                    PadState::default(),
                ];
                battle.run_frame(false, pads, 0, &mut libc, &mut random, None, false);
                let c = &battle.roster.chars[0];
                if c.act.current == action {
                    seen = true;
                    cut |= crate::chr_cam::is_cut_active(c);
                    demo |= battle.demo.is_active();
                }
            }
            assert!(seen, "{chara}");
            assert!(cut || demo, "{chara}");
            assert_eq!(demo, want_demo, "{chara}");
        }
    }

    #[test]
    fn real_skills() {
        // Character 1's second skill (id 0x56, two stocks, sequence 0: action 0xFE) is applied when its motion
        // ends (attribute 0x80): 20000 ki, the timed levels 3 / 4 / 3 / 4 for 600 frames and a charge gauge floor
        // of 30000.
        let Some(mut battle) = transform_battle(1, false) else {
            return;
        };
        battle.roster.chars[0].gauge.blast = 300_000;
        battle.roster.chars[0].gauge.ki = 0;
        let acts = run_skill(&mut battle, true, 120);
        assert!(acts.contains(&0xFE), "{acts:x?}");
        let c = &battle.roster.chars[0];
        assert!(c.gauge.blast < 300_000 - 150_000, "{}", c.gauge.blast);
        assert!(c.gauge.ki >= 20000, "{}", c.gauge.ki);
        assert_eq!(c.gauge.used[1], 1);
        assert!(c.slot_on[1]);
        assert_eq!([0, 1, 2, 3].map(|s| c.stat_mod(s)), [3, 4, 3, 4]);
        assert!(c.stat[0].timer[1] > 400, "{}", c.stat[0].timer[1]);
        assert!(c.charge_gauge >= 30000, "{}", c.charge_gauge);
        assert_eq!(c.stat_level(1), 3);
        // With one stock left it is refused.
        let acts = run_skill(&mut battle, true, 60);
        assert!(!acts.contains(&0xFE), "{acts:x?}");

        // Character 2's first skill (id 0x37, sequence 2: action 0x101) gives one automatic evasion at once and is
        // refused while it lasts; its second (id 0x49, kind 3, attributes 0x800280) enters the powered-up mode with
        // a penalty on stat 2 that the mode ignores, and its levels end with the mode.
        let mut battle = transform_battle(2, false).unwrap();
        battle.roster.chars[0].gauge.blast = 500_000;
        let acts = run_skill(&mut battle, false, 30);
        assert!(acts.contains(&0x101), "{acts:x?}");
        assert_eq!(battle.roster.chars[0].dodges_b, 1);
        let acts = run_skill(&mut battle, false, 30);
        assert!(!acts.contains(&0x101), "{acts:x?}");
        let acts = run_skill(&mut battle, true, 120);
        assert!(acts.contains(&0xFE), "{acts:x?}");
        let c = &battle.roster.chars[0];
        assert!(c.flags.test(6));
        assert_eq!(c.stat[2].penalty[1], -10);
        assert_eq!([0, 1, 2, 3].map(|s| c.stat_mod(s)), [3, 1, 2, 2]);
        battle.roster.chars[0].gauge.max_power = 1;
        run_skill(&mut battle, false, 0);
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        for _ in 0..3 {
            battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
        }
        let c = &battle.roster.chars[0];
        assert!(!c.flags.test(6));
        assert!(!c.slot_on[1]);
        assert_eq!([0, 1, 2, 3].map(|s| c.stat_mod(s)), [0, 0, -10, 0]);

        // Character 3's first skill (id 0, lock-on, sequence 1: action 0xFF) vanishes and comes back behind the
        // opponent.
        let mut battle = transform_battle(3, false).unwrap();
        battle.roster.chars[0].gauge.blast = 300_000;
        let d0 = crate::btl_chr::sub(
            battle.roster.chars[1].pose.pos,
            battle.roster.chars[0].pose.pos,
        );
        let acts = run_skill(&mut battle, false, 120);
        assert!(acts.contains(&0xFF), "{acts:x?}");
        let d1 = crate::btl_chr::sub(
            battle.roster.chars[1].pose.pos,
            battle.roster.chars[0].pose.pos,
        );
        // The opponent was ahead; now the fighter is on its other side.
        assert!(crate::btl_chr::dot(d0, d1) < 0.0, "{d0:?} {d1:?}");
    }

    #[test]
    fn real_ki_exhaustion() {
        // A fighter whose ki ran out (flag 0xBE) stands in 0xEA, recovering ki, until it has the parameter's
        // amount back; then neutral.
        let Some(mut battle) = transform_battle(1, false) else {
            return;
        };
        battle.roster.chars[1].gauge.ki = 0;
        battle.roster.chars[1].flags.set_held(0xBE);
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut acts = Vec::new();
        for _ in 0..600 {
            battle.run_frame(
                false,
                [PadState::default(); 2],
                0,
                &mut libc,
                &mut random,
                None,
                false,
            );
            let a = battle.roster.chars[1].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        assert_eq!(acts[..2], [0xEA, 0xB], "{acts:x?}");
        assert!(!battle.roster.chars[1].flags.test(0xBE));
        assert!(battle.roster.chars[1].gauge.ki > 0);
    }

    #[test]
    fn real_skill_grab() {
        use crate::btl_input::pad::UP;
        // Character 0's first skill (id 1, the blinding grab: flag 0x4C as its motion ends) catches the opponent
        // facing it within 100: reaction 9 (action 0xC6), blinded for the skill's frames, lock-on lost.
        let Some(mut battle) = transform_battle(0, false) else {
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
        for _ in 0..300 {
            let r = &battle.roster;
            if crate::btl_chr::opp_distance_xz(&r.chars[0], &r.chars[1]) < 40.0 {
                break;
            }
            battle.run_frame(false, pad(UP), 0, &mut libc, &mut random, None, false);
        }
        battle.roster.chars[0].gauge.blast = 300_000;
        let acts = run_skill(&mut battle, false, 10);
        let (mut opp, mut blind) = (Vec::new(), 0);
        for _ in 0..120 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            let o = &battle.roster.chars[1];
            blind = blind.max(o.blind_timer);
            if opp.last() != Some(&o.act.current) {
                opp.push(o.act.current);
            }
        }
        assert!(acts.contains(&0xFD), "{acts:x?}");
        assert!(opp.contains(&0xC6), "{opp:x?}");
        let frames = battle.assets.fighters[0].tables.skill().frames(0);
        assert!(blind > 0 && blind <= frames, "{blind} {frames}");
    }

    #[test]
    fn real_skill_effects() {
        use crate::btl_input::pad::UP;
        // Character 15's first skill (id 0x10, effect type 0 at its node with the frame table) knocks back an
        // opponent standing close: the skill record hits through `BtlColl_HitByStrike` for the table's damage,
        // and its effects return to their pools.
        let Some(mut battle) = transform_battle(15, false) else {
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
        for _ in 0..300 {
            let r = &battle.roster;
            if crate::btl_chr::opp_distance_xz(&r.chars[0], &r.chars[1]) < 30.0 {
                break;
            }
            battle.run_frame(false, pad(UP), 0, &mut libc, &mut random, None, false);
        }
        for _ in 0..20 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
        }
        battle.roster.chars[0].gauge.blast = 300_000;
        let health = battle.roster.chars[1].gauge.health;
        let mut libc = LibcRandom::new(1);
        let mut random = GameRandom::from_key(&[1]);
        let mut acts = run_skill(&mut battle, false, 10);
        let mut most = 0;
        for _ in 0..150 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            most = most.max(battle.roster.scene.fx.counts(None).iter().sum::<usize>());
            let a = battle.roster.chars[0].act.current;
            if acts.last() != Some(&a) {
                acts.push(a);
            }
        }
        assert!(acts.contains(&0xFD), "{acts:x?}");
        let s = battle.assets.fighters[0].tables.skill();
        let lost = health - battle.roster.chars[1].gauge.health;
        assert!(
            lost > 0 && lost % s.damage(0) == 0,
            "{lost} {}",
            s.damage(0)
        );
        assert!(most > 0);
        assert_eq!(battle.roster.scene.fx.counts(None).iter().sum::<usize>(), 0);

        // Character 22's first skill (id 7, lock-on, a capsule from node 47) stuns the opponent (reaction 0x17..:
        // action 0xD3 for the skill's frames).
        let mut battle = transform_battle(22, false).unwrap();
        battle.roster.chars[0].gauge.blast = 300_000;
        let health = battle.roster.chars[1].gauge.health;
        run_skill(&mut battle, false, 10);
        let mut opp = Vec::new();
        for _ in 0..60 {
            battle.run_frame(false, pad(0), 0, &mut libc, &mut random, None, false);
            let a = battle.roster.chars[1].act.current;
            if opp.last() != Some(&a) {
                opp.push(a);
            }
        }
        assert!(opp.contains(&0xD3), "{opp:x?}");
        assert!(battle.roster.chars[1].gauge.health < health);
    }

    #[test]
    fn real_transformations() {
        use crate::btl_input::pad::DOWN;
        // Character 1 taps R3: its default transformation (character 2, one stock, sequence 0: action 0xEC) loads
        // through the change queue while time stands still and the opponent is frozen; the stock is paid when the
        // model is in. Down + R3 takes character 2's transformation 3 back to character 1 (sequence 1: 0xED).
        let Some(mut battle) = transform_battle(1, true) else {
            return;
        };
        battle.roster.chars[0].gauge.blast = 300_000;
        let (log, stopped, still) = run_transform(&mut battle, 0, 120);
        assert!(log.iter().any(|l| l.1 == 0xEC), "{log:x?}");
        assert_eq!(log.last().map(|l| (l.1, l.2)), Some((0xB, 2)), "{log:x?}");
        assert!(stopped && still);
        let c = &battle.roster.chars[0];
        assert!(c.gauge.blast < 300_000 - 90_000, "{}", c.gauge.blast);
        let (log, _, _) = run_transform(&mut battle, DOWN, 120);
        assert!(log.iter().any(|l| l.1 == 0xED), "{log:x?}");
        assert_eq!(log.last().map(|l| l.2), Some(1), "{log:x?}");
        // Without the stocks, or on a side not allowed to change, nothing happens.
        battle.roster.chars[0].gauge.blast = 0;
        let (log, _, _) = run_transform(&mut battle, 0, 60);
        assert!(log.iter().all(|l| l.1 != 0xEC && l.2 == 1), "{log:x?}");
        let mut battle = transform_battle(1, false).unwrap();
        battle.roster.chars[0].gauge.blast = 300_000;
        let (log, _, _) = run_transform(&mut battle, 0, 60);
        assert!(log.iter().all(|l| l.1 != 0xEC && l.2 == 1), "{log:x?}");
        // Character 29 (three stocks, sequence 2: 0xEE) takes the new character's health at the same ratio;
        // character 105 (sequence 3: 0xEF) loads its extra object first.
        let mut battle = transform_battle(29, true).unwrap();
        battle.roster.chars[0].gauge.blast = 300_000;
        let (log, _, _) = run_transform(&mut battle, 0, 400);
        assert!(log.iter().any(|l| l.1 == 0xEE), "{log:x?}");
        assert_eq!(log.last().map(|l| (l.1, l.2)), Some((0xB, 30)), "{log:x?}");
        let g = battle.roster.chars[0].gauge;
        assert_eq!(
            g.health_max,
            battle.assets.fighters[0].entry.health_max.max(10000)
        );
        assert_eq!(g.health, g.health_max);
        // A rematch loads the starting character back (BtlLoad_Reload).
        battle.restart(&mut Vec::new());
        assert_eq!(battle.assets.fighters[0].chara, 29);
        assert_eq!(
            battle.roster.chars[0].gauge.health_max,
            battle.assets.fighters[0].entry.health_max.max(10000)
        );
        let mut battle = transform_battle(105, true).unwrap();
        battle.roster.chars[0].gauge.blast = 300_000;
        let (log, _, _) = run_transform(&mut battle, 0, 420);
        assert!(log.iter().any(|l| l.1 == 0xEF), "{log:x?}");
        assert_eq!(log.last().map(|l| (l.1, l.2)), Some((0xB, 106)), "{log:x?}");
    }

    #[test]
    fn real_rush_technique_shots() {
        // The rush techniques' effect of type 2 (EftShotTech) drives its emitters through the dash and the
        // sequence and leaves none behind: Vegeta's and Goku's slot 3, character 52's (whose point moves past the
        // opponent), and the Ultimate Blasts of characters 85 and 157 (powered up: 0x105, then the dash 0x11D and
        // the sequence 0x12F / 0x13B), whose sets fire blast objects, and 13's (in front of the opponent).
        for (chara, slot, shots) in [
            (1, 3, false),
            (0, 3, false),
            (52, 3, false),
            (85, 4, true),
            (157, 4, true),
            (13, 4, false),
        ] {
            let Some((log, dmg, table, parts, _)) = run_technique(chara, slot, 600, false) else {
                return;
            };
            let seq = if slot == 4 {
                (0x12F, 0x13B)
            } else {
                (0x12E, 0x13A)
            };
            assert!(log.iter().any(|l| (l.1, l.2) == seq), "{chara}: {log:x?}");
            if slot == 4 {
                assert!(log.iter().any(|l| l.1 == 0x105), "{chara}: {log:x?}");
            }
            assert!(dmg >= table, "{chara}: {dmg} {table}");
            assert!(parts[0] > 0 && parts[1] == 0, "{chara}: {parts:?}");
            assert_eq!(parts[2] > 0, shots, "{chara}: {parts:?}");
        }
    }

    #[test]
    fn real_catching_technique_effects() {
        // The rush shot (type 9) of character 65's slot 3 catches (reaction 0x22) into the sequence; character
        // 94's (technique 0x19D, reaction 0x1D) and the thrown objects (type 8) of characters 49 (0x167), 115
        // (0x168) and 55 (0x162, held in the hand: from beside the opponent) catch it while the attacker closes in
        // 0x12A.. (the looped catch 0x1E held until the attacker leaves its technique, flag 0xAF), and character
        // 150's object (0x1F3) is thrown again at the caught victim for the technique's second reaction (15:
        // 0xCF). Each deals the table damage.
        let has = |log: &[(usize, i32, i32)], a: &[i32], b: &[i32]| {
            log.iter().any(|(_, x, y)| a.contains(x) && b.contains(y))
        };
        let Some((log, dmg, table, parts, _)) = run_technique(65, 3, 400, false) else {
            return;
        };
        assert!(has(&log, &[0x12E], &[0x13A]), "{log:x?}");
        assert_eq!(dmg, table);
        assert!(parts[0] > 0 && parts[1] == 0, "{parts:?}");
        for (chara, slot, near) in [
            (94, 3, false),
            (49, 3, false),
            (115, 2, false),
            (55, 3, true),
            (150, 3, false),
        ] {
            let (log, dmg, table, parts, _) = run_technique(chara, slot, 400, near).unwrap();
            assert!(parts[0] > 0 && parts[1] == 0, "{chara}: {parts:?}");
            let (fin, caught) = if slot == 2 {
                (0x12A, 0x133)
            } else {
                (0x12B, 0x134)
            };
            assert!(has(&log, &[fin], &[caught]), "{chara}: {log:x?}");
            assert!(dmg >= table, "{chara}: {dmg} {table} {log:x?}");
            if chara == 150 {
                assert!(has(&log, &[fin], &[0xCF]), "{log:x?}");
            }
            assert_eq!(log.last().map(|l| l.2), Some(0xB), "{chara}: {log:x?}");
        }
    }

    #[test]
    fn real_rush_techniques_catch_and_finish() {
        // Vegeta's slot 3 (kind 4, homing dash 0x11C), Goku's slot 3 (kind 6, straight dash 0x11F) and slot 3 of
        // character 133 (kind 10, struck where it stands, 0x122) catch the opponent (BtlHit_CheckRush ->
        // BtlColl_StartThrow): both play the sequence (0x12E / victim 0x13A) placed by the technique's table, the
        // damage queue deals the whole table damage, and the victim lands by the landing kind. Character 61's slot
        // 2 (kind 8, steered dash 0x124) hits through its follow effect (type 7, reaction 41: launched up 0x136)
        // and ends when the effect does (flag 0xA8).
        let has = |log: &[(usize, i32, i32)], a: i32, b: Option<i32>| {
            log.iter()
                .any(|&(_, x, y)| x == a && b.is_none_or(|b| y == b))
        };
        for (chara, slot, near, dash) in [
            (1, 3, false, 0x11C),
            (0, 3, false, 0x11F),
            (133, 3, true, 0x122),
        ] {
            let Some((log, dmg, table, _, _)) = run_technique(chara, slot, 700, near) else {
                return;
            };
            assert!(has(&log, dash, None), "{chara}: {log:x?}");
            assert!(has(&log, 0x12E, Some(0x13A)), "{chara}: {log:x?}");
            assert_eq!(dmg, table, "{chara}: {log:x?}");
            let end = log.iter().position(|&(_, a, _)| a == 0x12E).unwrap() + 1;
            let (_, a, b) = log[end];
            assert_eq!(a, 0xB, "{chara}: {log:x?}");
            assert!(matches!(b, 0xCF | 0xD5), "{chara}: {log:x?}");
            assert_eq!(
                log.last().map(|l| (l.1, l.2)),
                Some((0xB, 0xB)),
                "{chara}: {log:x?}"
            );
        }
        let Some((log, dmg, table, _, _)) = run_technique(61, 2, 300, false) else {
            return;
        };
        assert!(has(&log, 0x124, Some(0x136)), "{log:x?}");
        assert!(has(&log, 0x124, Some(0xD0)), "{log:x?}");
        assert_eq!(dmg, table);
        assert!(log.iter().any(|&(i, a, _)| a == 0xB && i > 100), "{log:x?}");
    }

    #[test]
    fn real_max_power_drains_and_lets_the_fighter_charge_again() {
        // Goku charges with a blast stock into the powered-up mode (flag 6, the max power gauge full); let go, the
        // gauge drains over the parameter's time (+0x54 seconds) and the mode ends, after which the charge input
        // starts the ki charge (0x37) again.
        use crate::btl_input::pad::{L2, START};
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
        battle.roster.chars[0].gauge.blast = 100_000;
        let mut powered_at = None;
        for i in 0..600 {
            frame(&mut battle, L2);
            if battle.roster.chars[0].flags.test(6) {
                powered_at = Some(i);
                break;
            }
        }
        assert!(powered_at.is_some(), "never reached max power");
        assert_eq!(battle.roster.chars[0].gauge.max_power, 30000);
        let secs = battle.assets.fighters[0].tables.params.f32_at(0x54);
        let step = 30000 / (secs * 30.0) as i32;
        let mut ended_at = None;
        for i in 0..2000 {
            frame(&mut battle, 0);
            let c = &battle.roster.chars[0];
            if !c.flags.test(6) {
                ended_at = Some(i);
                break;
            }
        }
        let ended_at = ended_at.expect("the powered-up mode never ended");
        assert_eq!(battle.roster.chars[0].gauge.max_power, 0);
        // The last motion of the charge (0x36) still holds the gauge (flag 0xBB) for a few frames.
        let frames = (30000 + step - 1) / step;
        assert!(
            (frames..frames + 60).contains(&(ended_at as i32 + 1)),
            "{ended_at} for {frames}"
        );
        let mut charging = false;
        for _ in 0..30 {
            frame(&mut battle, L2);
            charging |= battle.roster.chars[0].act.current == 0x37;
        }
        assert!(charging);
    }

    #[test]
    fn real_technique_emitters_draw_and_end() {
        // The technique sets' emitters drive every particle module: Goku's Kamehameha (slot 2) sprite animations,
        // a ribbon, particles, sprites, a ray burst and rays; character 1's slot 2 chains; character 106's ring
        // emitters; character 13's sprite chains. Their objects draw while the technique runs and are gone, with
        // every pooled node, ring, particle and sprite, once it is over.
        const ANIM: usize = 0;
        const RIBBON: usize = 1;
        const PTCL: usize = 2;
        const BILL: usize = 3;
        const CHAIN: usize = 5;
        const RAY: usize = 6;
        const RAYS: usize = 7;
        const PART10: usize = 8;
        const LINK: usize = 9;
        let cases: [(i32, i32, &[usize]); 4] = [
            (0, 2, &[ANIM, RIBBON, PTCL, BILL, RAY, RAYS]),
            (1, 2, &[CHAIN]),
            (106, 2, &[PART10]),
            (13, 2, &[LINK]),
        ];
        for (chara, slot, kinds) in cases {
            let mut most = [0usize; 10];
            let mut tris = 0;
            let mut end = None;
            let r = fire_slot_with(chara, slot, 160, |b, i, out| {
                let fx = &b.roster.scene.fx;
                for (m, c) in most.iter_mut().zip(fx.counts(None)) {
                    *m = (*m).max(c);
                }
                tris = tris.max(out.effects.len());
                if i == 159 {
                    end = Some((
                        fx.counts(None),
                        fx.chain_nodes_used(),
                        fx.part10_particles(),
                        fx.link_sprites(),
                    ));
                }
            });
            if r.is_none() {
                return;
            }
            assert!(tris > 0, "character {chara}");
            for &k in kinds {
                assert!(most[k] > 0, "character {chara}: module {k} {most:?}");
            }
            assert_eq!(end, Some(([0; 10], 0, 0, 0)), "character {chara}");
        }
    }

    #[test]
    fn real_ki_objects_and_bombs() {
        use crate::btl_input::pad::{START, TRIANGLE};
        // Character 55: the tap blast (0xAE) is a thrown object (type 2): it lobs from the hand under gravity
        // (over the idle P2), breaks where it lands and is gone 15 frames later. The charged blast (0xAF) is of
        // type 3: two bombs, thrown with the VU0 generator's jitter, bounce, come to rest and explode.
        for (hold, bomb) in [(2, false), (60, true)] {
            let mut setup = BattleSetup::default();
            setup.sides[0].chara = 55;
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
                let skip =
                    battle.seq.state != SeqState::Ready && battle.seq.state != SeqState::Fight;
                frame(&mut battle, if skip && i % 2 == 0 { START } else { 0 });
                if battle.seq.state == SeqState::Fight {
                    break;
                }
            }
            let list = usize::from(bomb);
            let (mut most, mut created, mut gone, mut still) = (0, None, None, 0);
            let mut last = Vec::new();
            for i in 0..200 {
                frame(&mut battle, if i < hold { TRIANGLE } else { 0 });
                let objs = &battle.roster.scene.ki_objs[0][list];
                most = most.max(objs.len());
                if created.is_none() && !objs.is_empty() {
                    created = Some(i);
                }
                if created.is_some() && gone.is_none() && objs.is_empty() {
                    gone = Some(i);
                }
                let pos: Vec<_> = objs.iter().map(|o| o.pos()).collect();
                if !pos.is_empty() && pos == last {
                    still += 1;
                }
                last = pos;
            }
            let (created, gone) = (created.unwrap(), gone.expect("never ended"));
            assert_eq!(most, if bomb { 2 } else { 1 });
            assert!(still >= 10, "it never stopped: {still}");
            if bomb {
                assert!(gone - created > 60, "{created} {gone}");
            } else {
                assert!(gone - created < 40, "{created} {gone}");
            }
            assert_eq!(battle.roster.chars[1].gauge.health, 40000);
        }
    }

    #[test]
    fn real_charged_smash_knocks_the_opponent_away() {
        use crate::btl_input::pad::{SQUARE, START, UP};
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
        for _ in 0..300 {
            if battle.roster.chars[0].flags.test(0x13) {
                break;
            }
            frame(&mut battle, UP);
        }
        for _ in 0..20 {
            frame(&mut battle, 0);
        }
        // Hold the rush button (the smash after six frames), charge, let go.
        let (mut mine, mut theirs) = (Vec::new(), Vec::new());
        let health = battle.roster.chars[1].gauge.health;
        for i in 0..120 {
            frame(&mut battle, if i < 30 { SQUARE } else { 0 });
            mine.push(battle.roster.chars[0].act.current);
            theirs.push(battle.roster.chars[1].act.current);
        }
        mine.dedup();
        theirs.dedup();
        // The smash (0x4B) blows the defender away (0xD5) into the floor (0xDD), down (0xDB) and lying (0xD8).
        assert_eq!(mine, [0xB, 0x4B, 0xB], "{mine:x?}");
        assert_eq!(&theirs[..5], &[0xB, 0xD5, 0xDD, 0xDB, 0xD8], "{theirs:x?}");
        assert!(battle.roster.chars[1].gauge.health < health);
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
