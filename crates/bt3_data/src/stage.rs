use std::io;

const STAGE_HEADER_SIZE: usize = 0x60;
const STAGE_DATA_SIZE: usize = 0xA8;
const BOUNDS_SIZE: usize = 0x0C;
const ENV_SIZE: usize = 0x10;
const PLACEMENT_SIZE: usize = 0x20;

#[derive(Clone, Debug, PartialEq)]
pub struct StageResource {
    pub stage_id: i32,
    pub flags: i32,
    pub water_level: Option<f32>,
    pub object_count: usize,
    pub zone_count: usize,
    pub bounds: StageBounds,
    pub starting_placements: Vec<StagePlacement>,
    pub alternate_starting_placements: Vec<StagePlacement>,
    /// `StgLight.dir` of the first light record (`BtlStage_GetLightVecB`, with `BtlStage_Relocate`'s y / z sign
    /// flip): the direction `BtlObj_UpdateAll` gives the battle objects' light every frame. `None` when the
    /// stage has no light.
    pub light_dir: Option<[f32; 3]>,
    /// The stage paths (`StgPathTable`, file word 0x18; `BtlStage_GetPath`): the clash C exchanges are placed on
    /// them and its camera looks along them.
    pub paths: Vec<StagePath>,
}

/// `StgPath`: points `first .. end` are the usable ones; each point is 12 floats (x, y, z, ?, then the angles at
/// 4 / 5 the clash camera looks along).
#[derive(Clone, Debug, PartialEq)]
pub struct StagePath {
    pub end: i32,
    pub first: i32,
    pub points: Vec<[f32; 12]>,
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
        let stored_water_level = read_f32(bytes, environment_offset + 8)?;
        let water_level = (flags & 2 != 0).then_some(-stored_water_level);

        let starting_placements =
            parse_placements(bytes, data_offset, 0x28, 0x2C, "stage placements")?;
        let alternate_starting_placements =
            parse_placements(bytes, data_offset, 0x30, 0x34, "alternate stage placements")?;
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

        let paths = parse_paths(bytes)?;

        Ok(Self {
            stage_id,
            flags,
            water_level,
            object_count,
            zone_count,
            bounds,
            starting_placements,
            alternate_starting_placements,
            light_dir,
            paths,
        })
    }
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
        let mut bytes = vec![0; 0x240];
        put_u32(&mut bytes, 4, 0x80);
        put_u32(&mut bytes, 8, 0x100);
        put_i32(&mut bytes, 0x80 + 0x38, 4);
        put_u32(&mut bytes, 0x80 + 0x40, 0x34);
        put_f32(&mut bytes, 0x150, 1200.0);
        put_f32(&mut bytes, 0x154, -400.0);
        put_f32(&mut bytes, 0x158, 600.0);
        put_i32(&mut bytes, 0x100 + 8, 7);
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

        let stage = StageResource::parse(&bytes).unwrap();

        assert_eq!(stage.stage_id, 0);
        assert_eq!(stage.flags, 2);
        assert_eq!(stage.water_level, Some(32.0));
        assert_eq!(stage.object_count, 7);
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
