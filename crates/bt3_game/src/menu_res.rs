//! Texture files of an unpacked menu screen pack (`MPACK_AT(res, n)`, `MTEX(res, i)`).

use std::collections::BTreeMap;
use std::error::Error;

use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use crate::logo::TILES_PER_PICTURE;

/// A texture file: (section, part of the section's own pack, or 0 for the section itself).
pub type FileKey = (usize, usize);

/// A movie image's texture: entry `.1` of file `.0`.
pub type ImageTexture = Option<(FileKey, usize)>;

/// `MTEX(MPACK_AT(res, section), entry)` of the screen's own sections.
pub const fn tex(section: usize, entry: usize) -> ImageTexture {
    Some(((section, 0), entry))
}

/// Every entry of every file the image tables name.
pub fn decode_files<'a>(
    res: &[u8],
    tables: impl IntoIterator<Item = &'a [ImageTexture]>,
) -> Result<Vec<(FileKey, Vec<RgbaTexture>)>, Box<dyn Error>> {
    let mut files = BTreeMap::new();
    for (key, _) in tables.into_iter().flatten().flatten() {
        if files.contains_key(key) {
            continue;
        }
        let section = pack_section(res, key.0)?;
        let file = if key.1 == 0 {
            section
        } else {
            pack_section(section, key.1)?
        };
        files.insert(*key, decode_file(file)?);
    }
    Ok(files.into_iter().collect())
}

/// A `Sprite_DrawPicture` picture: one section of eight tiles.
pub fn decode_picture(res: &[u8], section: usize) -> Result<Vec<RgbaTexture>, Box<dyn Error>> {
    let tiles = decode_file(pack_section(res, section)?)?;
    if tiles.len() != TILES_PER_PICTURE {
        return Err(format!("picture section {section} has {} tiles", tiles.len()).into());
    }
    Ok(tiles)
}

/// Every entry of a texture file; an entry that only uploads a palette (no pixels, as the item pages' extra CLUTs)
/// keeps its place as an empty texture.
pub fn decode_file(data: &[u8]) -> Result<Vec<RgbaTexture>, Box<dyn Error>> {
    Ok(TextureResource::parse(data)?
        .entries
        .iter()
        .map(|e| match e.pixel_data {
            None => Ok(RgbaTexture {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }),
            Some(_) => decode_sprite_texture(e),
        })
        .collect::<Result<Vec<_>, _>>()?)
}

/// Texture indices per image; `first(file)` is where entry 0 of a file was registered, entry n is `2n` further.
pub fn image_indices(
    table: &[ImageTexture],
    first: &impl Fn(FileKey) -> usize,
) -> Vec<Option<usize>> {
    table
        .iter()
        .map(|t| t.map(|(file, entry)| first(file) + 2 * entry))
        .collect()
}
