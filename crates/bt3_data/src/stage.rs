use std::io;

const STAGE_HEADER_SIZE: usize = 0x60;
const STAGE_DATA_SIZE: usize = 0xA8;
const BOUNDS_SIZE: usize = 0x0C;
const ENV_SIZE: usize = 0x10;
const PLACEMENT_SIZE: usize = 0x20;
const OBJECT_DEF_SIZE: usize = 0x40;

#[derive(Clone, Debug, PartialEq)]
pub struct StageResource {
    pub stage_id: i32,
    pub flags: i32,
    /// `StgEnv.changeKind`: what a stage change leads to (`BtlStage_GetChangeTarget`: 1 stage 0xF, 2 stage 3).
    pub change_kind: i32,
    /// The first word of the stage data's table at +0x74 (`d->flags[0]`): 1 / 2 / 4 the stage effects, 8, 0x10
    /// the moon (`BtlStage_HasMoon`); 0 when there is none.
    pub stage_flags: u32,
    pub water_level: Option<f32>,
    pub object_count: usize,
    /// `StgDataA.objDefs` (+0x08 / +0x0C): the destructible objects' definitions.
    pub objects: Vec<StageObjectDef>,
    pub zone_count: usize,
    pub bounds: StageBounds,
    pub starting_placements: Vec<StagePlacement>,
    pub alternate_starting_placements: Vec<StagePlacement>,
    /// `StgDataA.places` (+0x38 / +0x3C): the first is `BtlStage_GetPlace`'s, where the technique placements
    /// (fighter requests 0xF4 / 0xF6 / 0xF7) are relative to.
    pub places: Vec<StagePlacement>,
    /// `StgLight.dir` of the first light record (`BtlStage_GetLightVecB`, with `BtlStage_Relocate`'s y / z sign
    /// flip): the direction `BtlObj_UpdateAll` gives the battle objects' light every frame. `None` when the
    /// stage has no light.
    pub light_dir: Option<[f32; 3]>,
    /// The first record of `StgDataA` +0x20 / +0x24 (`BtlStage_GetLightDir`), y and z negated as `BtlStage_Relocate`
    /// does: the chains take it as the wind, its w the strength.
    pub wind: Option<[f32; 4]>,
    /// The stage paths (`StgPathTable`, file word 0x18; `BtlStage_GetPath`): the clash C exchanges are placed on
    /// them and its camera looks along them.
    pub paths: Vec<StagePath>,
    /// `StgDataA.dust` (+0x68 / +0x6C, `BtlStage_GetList68`): the stage's dust look, `None` without a record.
    pub dust: Option<StageDust>,
    /// Bit 0 of the first water record (`StgDataA` +0x90 / +0x94, `BtlStage_GetList90`): with stage pack entry
    /// 0x12 the stage has the water effects (`EftStage_HasKind(7)`).
    pub water_fx: bool,
    /// The MEF0 block's list 1 (stage pack entry 0x13, `BtlStage_GetFxResB2`): the smoke emitters, used with
    /// stage flag 2.
    pub smoke: Vec<StageSmoke>,
    /// The MEF0 block's list 0 (`BtlStage_GetFxResA2`): the geysers, used with stage flag 1.
    pub geysers: Vec<StageGeyser>,
    /// `StgDataA.weather` (+0x60 / +0x64, `BtlStage_GetList60`): the first weather record.
    pub weather: Option<StageWeather>,
    /// `StgDataA.surf` (+0x58 / +0x5C, `BtlStage_GetList58`): the reflecting surface's record.
    pub surf: Option<StageSurfRec>,
    /// `StgDataA.lightColors` (+0x78 / +0x7C, `BtlStage_GetLightColors`): the first record's two colours (the
    /// fighters' dust takes them).
    pub light_colors: Option<[u8; 8]>,
    /// The same record's bytes 8..12: the objects' light colour (`BtlObj_UpdateAll`'s `BtlObjLight_SetColor`),
    /// which the ground shadows are drawn in.
    pub shadow_color: Option<[u8; 4]>,
    /// `StgDataA.tint` (+0x80 / +0x84, `BtlStage_GetList80`): the two colours of the stage tint's slot 0 (the
    /// darkening of request 0x12), RGBA each.
    pub tint: Option<[u8; 8]>,
}

/// `StgSurfRec`: what the reflecting surface takes over its file's parameters (the light's y and z negated as
/// `BtlStage_BindData` does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageSurfRec {
    pub light_dir: [f32; 3],
    pub specular: f32,
    pub fade_dist: f32,
    pub anim_speed: u16,
    pub color_a: [u8; 4],
    pub color_b: [u8; 4],
}

/// `StgWeatherRec`: the weather particles' wind, size, flags and count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageWeather {
    pub wind: [f32; 3],
    pub size: f32,
    /// Bit 0 of the record's flags: the weather is on.
    pub on: bool,
    /// Bit 0: fade near y = 0; bit 1: stay above the ground.
    pub ptcl_flags: u16,
    pub count: u16,
}

/// `EftGeyserRec`: a geyser's column (its y and z negated as `BtlStage_BindMef` does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageGeyser {
    pub pos: [f32; 3],
    pub width: f32,
    pub height: f32,
    /// The texture's scroll a frame.
    pub scroll: f32,
    /// Frames at full height, of the fade in, and of the wait between two eruptions.
    pub hold: u16,
    pub fade: u16,
    pub wait: u16,
    pub rgba: [u8; 4],
}

/// `EftSmokeSrc`: a smoke emitter of the stage (its y and z negated as `BtlStage_BindMef` does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageSmoke {
    pub pos: [f32; 3],
    /// The particles' sideways speed factor a frame.
    pub damp: f32,
    pub size: f32,
    pub speed: f32,
    pub life_base: u16,
    /// One particle a frame with a chance of 1 in this.
    pub rate: u16,
    pub ambient: [u8; 4],
    pub diffuse: [u8; 4],
}

/// `EftGndDustStage`: the colours, speed and size of the debris' dust puffs when `enabled`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageDust {
    pub enabled: bool,
    pub col_a: [u8; 4],
    pub col_b: [u8; 4],
    pub speed: f32,
    pub speed_rand: f32,
    pub size: u16,
    pub r_min: u8,
    pub r_max: u8,
}

/// `EftBoundParam` (stage pack entry 23) and the colour of entry 22: the boundary wall that shows in front of a
/// fighter near the stage's edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageBound {
    /// The distance from the wall at which it starts to show.
    pub range: f32,
    pub height: f32,
    /// Added to the stage's inner radius.
    pub radius_add: f32,
    pub alpha: f32,
    /// How far from the point nearest to the fighter the wall fades out.
    pub fade_dist: f32,
    /// The angle the wall spans, and the texture's repeat in angle and height.
    pub arc: f32,
    pub tex_arc: f32,
    pub tex_height: f32,
    /// The texture's scroll and the pulse's turns per frame.
    pub scroll: [f32; 2],
    pub phase_rate: f32,
    pub layer: i32,
    pub color: [f32; 3],
}

impl StageBound {
    /// Entries 23 (the parameters) and 22 (the colour) of a stage file's pack.
    pub fn parse(param: &[u8], color: &[u8]) -> io::Result<Self> {
        let f = |o: usize| read_f32(param, o);
        Ok(Self {
            range: f(0)?,
            height: f(4)?,
            radius_add: f(0xC)?,
            alpha: f(0x14)?,
            fade_dist: f(0x18)?,
            arc: f(0x1C)?,
            tex_arc: f(0x20)?,
            tex_height: f(0x24)?,
            scroll: [f(0x2C)?, f(0x30)?],
            phase_rate: f(0x34)?,
            layer: read_i32(param, 0x38)?,
            color: [
                read_f32(color, 0)?,
                read_f32(color, 4)?,
                read_f32(color, 8)?,
            ],
        })
    }
}

/// `StgPath`: points `first .. end` are the usable ones; each point is 12 floats (x, y, z, ?, then the angles at
/// 4 / 5 the clash camera looks along).
#[derive(Clone, Debug, PartialEq)]
pub struct StagePath {
    pub end: i32,
    pub first: i32,
    pub points: Vec<[f32; 12]>,
}

/// `StgObjDef`: a destructible object's hit points, `STG_OBJ_*` type bits, model animation, parent (whose breaking
/// also breaks it) and position (y and z negated as `BtlStage_BindData` does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageObjectDef {
    pub hp: i32,
    pub kind: i32,
    pub anim: i32,
    pub parent: i32,
    pub position: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageBounds {
    pub radius: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StagePlacement {
    pub position: [f32; 3],
    pub target: [f32; 3],
}

impl StageResource {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let stage_offset = align_word(read_u32(bytes, 4)?)?;
        let data_offset = align_word(read_u32(bytes, 8)?)?;
        checked_range(bytes, stage_offset, STAGE_HEADER_SIZE, "stage header")?;
        checked_range(bytes, data_offset, STAGE_DATA_SIZE, "stage data")?;

        let zone_count = read_count(bytes, stage_offset + 0x38, "stage zone")?;
        let bounds_offset = resolve_word_relative(
            bytes,
            stage_offset,
            read_u32(bytes, stage_offset + 0x40)?,
            BOUNDS_SIZE,
            "stage bounds",
        )?;
        let bounds = StageBounds {
            radius: read_f32(bytes, bounds_offset)?,
            top: read_f32(bytes, bounds_offset + 4)?,
            bottom: read_f32(bytes, bounds_offset + 8)?,
        };
        let object_count = read_count(bytes, data_offset + 8, "stage object")?;
        let objects_offset = resolve_relative(
            bytes,
            data_offset,
            read_u32(bytes, data_offset + 0xC)?,
            object_count * OBJECT_DEF_SIZE,
            "stage objects",
        )?;
        let objects = (0..object_count)
            .map(|i| {
                let o = objects_offset + i * OBJECT_DEF_SIZE;
                Ok(StageObjectDef {
                    hp: read_i32(bytes, o)?,
                    kind: read_i32(bytes, o + 4)?,
                    anim: read_i32(bytes, o + 8)?,
                    parent: read_i32(bytes, o + 0xC)?,
                    position: read_game_position(bytes, o + 0x20)?,
                })
            })
            .collect::<io::Result<_>>()?;
        let environment_count = read_count(bytes, data_offset + 0x10, "stage environment")?;
        let environment_offset = resolve_relative(
            bytes,
            data_offset,
            read_u32(bytes, data_offset + 0x14)?,
            if environment_count == 0 { 0 } else { ENV_SIZE },
            "stage environment",
        )?;
        if environment_count == 0 {
            return Err(invalid_data("stage has no environment record"));
        }

        let stage_id = read_i32(bytes, environment_offset)?;
        let flags = read_i32(bytes, environment_offset + 4)?;
        let change_kind = read_i32(bytes, environment_offset + 0xC)?;
        let stored_water_level = read_f32(bytes, environment_offset + 8)?;
        let water_level = (flags & 2 != 0).then_some(-stored_water_level);

        let starting_placements =
            parse_placements(bytes, data_offset, 0x28, 0x2C, "stage placements")?;
        let alternate_starting_placements =
            parse_placements(bytes, data_offset, 0x30, 0x34, "alternate stage placements")?;
        let places = parse_placements(bytes, data_offset, 0x38, 0x3C, "stage places")?;
        let stage_flags = read_u32(bytes, data_offset + 0x74)
            .and_then(|rel| resolve_relative(bytes, data_offset, rel, 4, "stage flags"))
            .and_then(|o| read_u32(bytes, o))
            .unwrap_or(0);
        let light_count = read_count(bytes, data_offset + 0x18, "stage light")?;
        let light_dir = if light_count == 0 {
            None
        } else {
            let light = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x1C)?,
                0x20,
                "stage light",
            )?;
            Some(read_game_position(bytes, light + 0x10)?)
        };

        let wind = if read_count(bytes, data_offset + 0x20, "stage wind")? == 0 {
            None
        } else {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x24)?,
                0x10,
                "stage wind",
            )?;
            let p = read_game_position(bytes, o)?;
            Some([p[0], p[1], p[2], read_f32(bytes, o + 12)?])
        };
        let paths = parse_paths(bytes)?;
        let dust = if read_count(bytes, data_offset + 0x68, "stage dust")? == 0 {
            None
        } else {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x6C)?,
                0x18,
                "stage dust",
            )?;
            let b = |k: usize| bytes[o + k];
            Some(StageDust {
                enabled: read_i32(bytes, o)? != 0,
                col_a: [b(4), b(5), b(6), b(7)],
                col_b: [b(8), b(9), b(10), b(11)],
                speed: read_f32(bytes, o + 0xC)?,
                speed_rand: read_f32(bytes, o + 0x10)?,
                size: u16::from_le_bytes([b(0x14), b(0x15)]),
                r_min: b(0x16),
                r_max: b(0x17),
            })
        };

        let water_fx = read_count(bytes, data_offset + 0x90, "stage water")? != 0 && {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x94)?,
                4,
                "stage water",
            )?;
            read_u32(bytes, o)? & 1 != 0
        };

        let smoke = parse_mef_list(bytes, 1)?
            .into_iter()
            .map(|o| {
                let b = |k: usize| bytes[o + k];
                Ok(StageSmoke {
                    pos: [
                        read_f32(bytes, o)?,
                        -read_f32(bytes, o + 4)?,
                        -read_f32(bytes, o + 8)?,
                    ],
                    damp: read_f32(bytes, o + 0xC)?,
                    size: read_f32(bytes, o + 0x10)?,
                    speed: read_f32(bytes, o + 0x14)?,
                    life_base: u16::from_le_bytes([b(0x18), b(0x19)]),
                    rate: u16::from_le_bytes([b(0x1A), b(0x1B)]),
                    ambient: [b(0x1C), b(0x1D), b(0x1E), b(0x1F)],
                    diffuse: [b(0x20), b(0x21), b(0x22), b(0x23)],
                })
            })
            .collect::<io::Result<Vec<_>>>()?;

        let weather = if read_count(bytes, data_offset + 0x60, "stage weather")? == 0 {
            None
        } else {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x64)?,
                0x18,
                "stage weather",
            )?;
            Some(StageWeather {
                wind: [
                    read_f32(bytes, o)?,
                    read_f32(bytes, o + 4)?,
                    read_f32(bytes, o + 8)?,
                ],
                size: read_f32(bytes, o + 0xC)?,
                on: read_i32(bytes, o + 0x10)? & 1 != 0,
                ptcl_flags: u16::from_le_bytes([bytes[o + 0x14], bytes[o + 0x15]]),
                count: u16::from_le_bytes([bytes[o + 0x16], bytes[o + 0x17]]),
            })
        };
        let surf = if read_count(bytes, data_offset + 0x58, "stage surface")? == 0 {
            None
        } else {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x5C)?,
                0x20,
                "stage surface",
            )?;
            let b = |k: usize| bytes[o + k];
            Some(StageSurfRec {
                light_dir: [
                    read_f32(bytes, o)?,
                    -read_f32(bytes, o + 4)?,
                    -read_f32(bytes, o + 8)?,
                ],
                specular: read_f32(bytes, o + 0xC)?,
                fade_dist: read_f32(bytes, o + 0x10)?,
                anim_speed: u16::from_le_bytes([b(0x14), b(0x15)]),
                color_a: [b(0x17), b(0x18), b(0x19), b(0x1A)],
                color_b: [b(0x1B), b(0x1C), b(0x1D), b(0x1E)],
            })
        };
        let (light_colors, shadow_color) =
            if read_count(bytes, data_offset + 0x78, "stage light colours")? == 0 {
                (None, None)
            } else {
                let o = resolve_relative(
                    bytes,
                    data_offset,
                    read_u32(bytes, data_offset + 0x7C)?,
                    12,
                    "stage light colours",
                )?;
                (
                    Some(bytes[o..o + 8].try_into().unwrap()),
                    Some(bytes[o + 8..o + 12].try_into().unwrap()),
                )
            };
        let tint = if read_count(bytes, data_offset + 0x80, "stage tint")? == 0 {
            None
        } else {
            let o = resolve_relative(
                bytes,
                data_offset,
                read_u32(bytes, data_offset + 0x84)?,
                8,
                "stage tint",
            )?;
            Some(bytes[o..o + 8].try_into().unwrap())
        };
        let geysers = parse_mef_list(bytes, 0)?
            .into_iter()
            .map(|o| {
                let h = |k: usize| u16::from_le_bytes([bytes[o + k], bytes[o + k + 1]]);
                Ok(StageGeyser {
                    pos: [
                        read_f32(bytes, o)?,
                        -read_f32(bytes, o + 4)?,
                        -read_f32(bytes, o + 8)?,
                    ],
                    width: read_f32(bytes, o + 0xC)?,
                    height: read_f32(bytes, o + 0x10)?,
                    scroll: read_f32(bytes, o + 0x14)?,
                    hold: h(0x18),
                    fade: h(0x1A),
                    wait: h(0x1C),
                    rgba: [
                        bytes[o + 0x1E],
                        bytes[o + 0x1F],
                        bytes[o + 0x20],
                        bytes[o + 0x21],
                    ],
                })
            })
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self {
            stage_id,
            flags,
            change_kind,
            stage_flags,
            water_level,
            object_count,
            objects,
            zone_count,
            bounds,
            starting_placements,
            alternate_starting_placements,
            places,
            light_dir,
            paths,
            dust,
            water_fx,
            smoke,
            geysers,
            weather,
            surf,
            light_colors,
            shadow_color,
            tint,
            wind,
        })
    }
}

/// The 0x30-byte records of list `n` of the stage file's MEF0 block (pack entry 0x13 when it is not empty,
/// `BtlStage_BindMef`: magic, a word, then count and offset of lists 0..2 from the block); their offsets.
fn parse_mef_list(bytes: &[u8], n: usize) -> io::Result<Vec<usize>> {
    let (start, end) = (read_u32(bytes, 0x4C)?, read_u32(bytes, 0x50)?);
    if start == end {
        return Ok(Vec::new());
    }
    let mef = (start & !3) as usize;
    if bytes.get(mef..mef + 4) != Some(b"MEF0".as_slice()) {
        return Ok(Vec::new());
    }
    let count = read_count(bytes, mef + 8 + n * 8, "stage MEF0 list")?;
    let list = mef + read_u32(bytes, mef + 0xC + n * 8)? as usize;
    checked_range(bytes, list, count * 0x30, "stage MEF0 list")?;
    Ok((0..count).map(|i| list + i * 0x30).collect())
}

/// `BtlStage_BindPaths` / `StgPathTable_Relocate`: the table at file word 0x18 (offsets from the table); none
/// when the word is 0. A path keeps its points up to `end` (at least the first, which the camera reads).
fn parse_paths(bytes: &[u8]) -> io::Result<Vec<StagePath>> {
    let table = align_word(read_u32(bytes, 0x60)?)?;
    if table == 0 {
        return Ok(Vec::new());
    }
    let count = read_count(bytes, table + 8, "stage path")?;
    let list = table + align_word(read_u32(bytes, table + 0xC)?)?;
    let mut paths = Vec::new();
    for i in 0..count {
        let rec = list + i * 0x10;
        let end = read_i32(bytes, rec)?;
        let points_at = table + align_word(read_u32(bytes, rec + 4)?)?;
        let first = read_i32(bytes, rec + 8)?;
        let n = usize::try_from(end.max(1)).unwrap_or(1);
        let mut points = Vec::new();
        for k in 0..n {
            let p = points_at + k * 0x30;
            let mut point = [0.0; 12];
            for (j, v) in point.iter_mut().enumerate() {
                *v = read_f32(bytes, p + j * 4)?;
            }
            points.push(point);
        }
        paths.push(StagePath { end, first, points });
    }
    Ok(paths)
}

fn parse_placements(
    bytes: &[u8],
    data_offset: usize,
    count_offset: usize,
    pointer_offset: usize,
    label: &str,
) -> io::Result<Vec<StagePlacement>> {
    let count = read_count(bytes, data_offset + count_offset, label)?;
    let size = count
        .checked_mul(PLACEMENT_SIZE)
        .ok_or_else(|| invalid_data("stage placement table size overflow"))?;
    let offset = resolve_relative(
        bytes,
        data_offset,
        read_u32(bytes, data_offset + pointer_offset)?,
        size,
        label,
    )?;
    let mut placements = Vec::new();
    placements
        .try_reserve_exact(count)
        .map_err(|_| io::Error::other("could not allocate stage placements"))?;
    for index in 0..count {
        let placement_offset = offset + index * PLACEMENT_SIZE;
        placements.push(StagePlacement {
            position: read_game_position(bytes, placement_offset)?,
            target: read_game_position(bytes, placement_offset + 0x10)?,
        });
    }
    Ok(placements)
}

fn read_game_position(bytes: &[u8], offset: usize) -> io::Result<[f32; 3]> {
    Ok([
        read_f32(bytes, offset)?,
        -read_f32(bytes, offset + 4)?,
        -read_f32(bytes, offset + 8)?,
    ])
}

fn read_count(bytes: &[u8], offset: usize, label: &str) -> io::Result<usize> {
    usize::try_from(read_i32(bytes, offset)?)
        .map_err(|_| invalid_data(&format!("{label} count is negative")))
}

fn resolve_relative(
    bytes: &[u8],
    base: usize,
    relative: u32,
    size: usize,
    label: &str,
) -> io::Result<usize> {
    let offset = base
        .checked_add(usize::try_from(relative).map_err(|_| invalid_data("stage offset overflow"))?)
        .ok_or_else(|| invalid_data("stage offset overflow"))?;
    checked_range(bytes, offset, size, label)?;
    Ok(offset)
}

fn resolve_word_relative(
    bytes: &[u8],
    base: usize,
    relative_words: u32,
    size: usize,
    label: &str,
) -> io::Result<usize> {
    let relative_bytes = usize::try_from(relative_words)
        .ok()
        .and_then(|words| words.checked_mul(4))
        .ok_or_else(|| invalid_data("stage word offset overflow"))?;
    let offset = base
        .checked_add(relative_bytes)
        .ok_or_else(|| invalid_data("stage offset overflow"))?;
    checked_range(bytes, offset, size, label)?;
    Ok(offset)
}

fn checked_range(bytes: &[u8], offset: usize, size: usize, label: &str) -> io::Result<()> {
    offset
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| invalid_data(&format!("{label} exceeds stage resource")))?;
    Ok(())
}

fn align_word(offset: u32) -> io::Result<usize> {
    usize::try_from(offset & !3).map_err(|_| invalid_data("stage offset exceeds platform size"))
}

fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("stage field exceeds resource"))?;
    Ok(u32::from_le_bytes(field.try_into().unwrap()))
}

fn read_i32(bytes: &[u8], offset: usize) -> io::Result<i32> {
    Ok(i32::from_le_bytes(read_u32(bytes, offset)?.to_le_bytes()))
}

fn read_f32(bytes: &[u8], offset: usize) -> io::Result<f32> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::StageResource;

    #[test]
    fn parses_stage_metadata_and_applies_game_coordinate_signs() {
        let mut bytes = vec![0; 0x400];
        put_u32(&mut bytes, 4, 0x80);
        put_u32(&mut bytes, 8, 0x100);
        put_i32(&mut bytes, 0x80 + 0x38, 4);
        put_u32(&mut bytes, 0x80 + 0x40, 0x34);
        put_f32(&mut bytes, 0x150, 1200.0);
        put_f32(&mut bytes, 0x154, -400.0);
        put_f32(&mut bytes, 0x158, 600.0);
        put_i32(&mut bytes, 0x100 + 8, 7);
        put_u32(&mut bytes, 0x100 + 0xC, 0x140);
        put_i32(&mut bytes, 0x240 + 0x40, 30);
        put_i32(&mut bytes, 0x240 + 0x4C, 0);
        put_f32(&mut bytes, 0x240 + 0x64, 15.0);
        put_i32(&mut bytes, 0x100 + 0x10, 1);
        put_u32(&mut bytes, 0x100 + 0x14, 0xA8);
        put_i32(&mut bytes, 0x100 + 0x28, 2);
        put_u32(&mut bytes, 0x100 + 0x2C, 0xC0);
        put_i32(&mut bytes, 0x100 + 0x30, 1);
        put_u32(&mut bytes, 0x100 + 0x34, 0x100);
        put_i32(&mut bytes, 0x1A8, 0);
        put_i32(&mut bytes, 0x1AC, 2);
        put_f32(&mut bytes, 0x1B0, -32.0);
        let first = 0x1C0;
        put_f32(&mut bytes, first, 10.0);
        put_f32(&mut bytes, first + 4, -20.0);
        put_f32(&mut bytes, first + 8, 30.0);
        put_f32(&mut bytes, first + 0x10, 40.0);
        put_f32(&mut bytes, first + 0x14, -50.0);
        put_f32(&mut bytes, first + 0x18, 60.0);
        put_f32(&mut bytes, 0x200, 70.0);
        put_f32(&mut bytes, 0x204, -80.0);
        put_f32(&mut bytes, 0x208, 90.0);
        put_f32(&mut bytes, 0x210, 100.0);
        put_f32(&mut bytes, 0x214, -110.0);
        put_f32(&mut bytes, 0x218, 120.0);
        // One water record whose first word has bit 0.
        put_i32(&mut bytes, 0x100 + 0x90, 1);
        put_u32(&mut bytes, 0x100 + 0x94, 0x280);
        put_u32(&mut bytes, 0x380, 3);
        // A MEF0 block (pack entry 0x13 at 0x300, entry 0x14 after it) with one smoke emitter in list 1.
        put_u32(&mut bytes, 0x4C, 0x300);
        put_u32(&mut bytes, 0x50, 0x3C0);
        bytes[0x300..0x304].copy_from_slice(b"MEF0");
        put_i32(&mut bytes, 0x310, 1);
        put_u32(&mut bytes, 0x314, 0x40);
        put_f32(&mut bytes, 0x344, 5.0);
        put_f32(&mut bytes, 0x34C, 0.9);
        put_u32(&mut bytes, 0x358, 0x0003_0040);
        bytes[0x35C] = 77;

        let stage = StageResource::parse(&bytes).unwrap();

        assert_eq!(stage.stage_id, 0);
        assert_eq!(stage.flags, 2);
        assert_eq!(stage.water_level, Some(32.0));
        assert!(stage.water_fx);
        assert_eq!(stage.smoke.len(), 1);
        assert!(stage.geysers.is_empty());
        assert_eq!(stage.weather, None);
        let smoke = stage.smoke[0];
        assert_eq!((smoke.pos[1], smoke.damp), (-5.0, 0.9));
        assert_eq!(
            (smoke.life_base, smoke.rate, smoke.ambient[0]),
            (0x40, 3, 77)
        );
        assert_eq!(stage.object_count, 7);
        assert_eq!(stage.objects[1].hp, 30);
        assert_eq!(stage.objects[1].position, [0.0, -15.0, 0.0]);
        assert_eq!(stage.zone_count, 4);
        assert_eq!(stage.bounds.radius, 1200.0);
        assert_eq!(stage.bounds.top, -400.0);
        assert_eq!(stage.bounds.bottom, 600.0);
        assert_eq!(stage.starting_placements[0].position, [10.0, 20.0, -30.0]);
        assert_eq!(stage.starting_placements[0].target, [40.0, 50.0, -60.0]);
        assert_eq!(
            stage.alternate_starting_placements[0].position,
            [70.0, 80.0, -90.0]
        );
        assert_eq!(
            stage.alternate_starting_placements[0].target,
            [100.0, 110.0, -120.0]
        );
    }

    #[test]
    fn rejects_offsets_outside_the_stage_file() {
        let mut bytes = vec![0; 0x80];
        put_u32(&mut bytes, 4, 0x80);
        put_u32(&mut bytes, 8, 0x100);

        assert!(StageResource::parse(&bytes).is_err());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
        put_u32(bytes, offset, value as u32);
    }

    fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_u32(bytes, offset, value.to_bits());
    }
}
