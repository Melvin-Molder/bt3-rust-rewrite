use std::io;

const FILE_HEADER_SIZE: usize = 0x10;
const ENTRY_SIZE: usize = 0x40;

pub struct TextureResource<'a> {
    pub entries: Vec<TextureEntry<'a>>,
}

pub struct TextureEntry<'a> {
    pub pixel_data: Option<&'a [u8]>,
    pub clut_data: Option<&'a [u8]>,
    pub pixel_size: i32,
    pub clut_size: i32,
    pub pixel_blt: u32,
    pub clut_blt: u32,
    /// `tbpOfs` / `cbpOfs`: block offsets of the pixels / CLUT from the file's first texture.
    pub tbp_offset: i32,
    pub cbp_offset: i32,
    pub texture_register: u64,
}

impl<'a> TextureResource<'a> {
    pub fn parse(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.len() < FILE_HEADER_SIZE {
            return Err(invalid_data("texture resource header is truncated"));
        }

        let count = read_u32(bytes, 0)? as usize;
        let entries_offset = (read_u32(bytes, 4)? as usize)
            .checked_mul(4)
            .ok_or_else(|| invalid_data("texture entry offset overflow"))?;
        let entries_size = count
            .checked_mul(ENTRY_SIZE)
            .ok_or_else(|| invalid_data("texture entry table size overflow"))?;
        let entries_end = entries_offset
            .checked_add(entries_size)
            .filter(|end| entries_offset >= FILE_HEADER_SIZE && *end <= bytes.len())
            .ok_or_else(|| invalid_data("texture entry table exceeds resource"))?;

        let mut entries = Vec::new();
        entries
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("could not allocate texture entry table"))?;

        for entry_start in (entries_offset..entries_end).step_by(ENTRY_SIZE) {
            let entry_end = entry_start + ENTRY_SIZE;
            let entry = &bytes[entry_start..entry_end];
            let pixel_size = read_i32(entry, 8)?;
            let clut_size = read_i32(entry, 12)?;
            let pixel_data = resolve_payload(bytes, read_u32(entry, 0)?, pixel_size, "pixel")?;
            let clut_data = resolve_payload(bytes, read_u32(entry, 4)?, clut_size, "CLUT")?;

            entries.push(TextureEntry {
                pixel_data,
                clut_data,
                pixel_size,
                clut_size,
                pixel_blt: read_u32(entry, 24)?,
                clut_blt: read_u32(entry, 28)?,
                tbp_offset: read_i32(entry, 32)?,
                cbp_offset: read_i32(entry, 36)?,
                texture_register: read_u64(entry, 48)?,
            });
        }

        Ok(Self { entries })
    }
}

fn resolve_payload<'a>(
    resource: &'a [u8],
    offset_words: u32,
    size: i32,
    label: &str,
) -> io::Result<Option<&'a [u8]>> {
    if offset_words == 0 {
        return Ok(None);
    }
    let size = usize::try_from(size)
        .map_err(|_| invalid_data(&format!("texture {label} size is negative")))?;
    let offset = usize::try_from(offset_words)
        .ok()
        .and_then(|words| words.checked_mul(4))
        .ok_or_else(|| invalid_data(&format!("texture {label} offset overflow")))?;
    let end = offset
        .checked_add(size)
        .filter(|end| *end <= resource.len())
        .ok_or_else(|| invalid_data(&format!("texture {label} data exceeds resource")))?;
    Ok(Some(&resource[offset..end]))
}

fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let word = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("texture field exceeds resource"))?;
    Ok(u32::from_le_bytes(word.try_into().unwrap()))
}

fn read_i32(bytes: &[u8], offset: usize) -> io::Result<i32> {
    let word = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("texture field exceeds resource"))?;
    Ok(i32::from_le_bytes(word.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: usize) -> io::Result<u64> {
    let word = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| invalid_data("texture field exceeds resource"))?;
    Ok(u64::from_le_bytes(word.try_into().unwrap()))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::TextureResource;

    #[test]
    fn parses_word_relative_table_and_payloads() {
        let mut bytes = vec![0; 88];
        put_u32(&mut bytes, 0, 1);
        put_u32(&mut bytes, 4, 4);
        put_u32(&mut bytes, 16, 20);
        put_u32(&mut bytes, 20, 21);
        put_u32(&mut bytes, 24, 4);
        put_u32(&mut bytes, 28, 4);
        put_u64(&mut bytes, 64, 0x1122_3344_5566_7788);
        bytes[80..84].copy_from_slice(&[1, 2, 3, 4]);
        bytes[84..88].copy_from_slice(&[5, 6, 7, 8]);

        let resource = TextureResource::parse(&bytes).unwrap();

        assert_eq!(resource.entries.len(), 1);
        assert_eq!(resource.entries[0].pixel_data, Some(&[1, 2, 3, 4][..]));
        assert_eq!(resource.entries[0].clut_data, Some(&[5, 6, 7, 8][..]));
        assert_eq!(resource.entries[0].texture_register, 0x1122_3344_5566_7788);
    }

    #[test]
    fn rejects_payloads_outside_resource() {
        let mut bytes = vec![0; 80];
        put_u32(&mut bytes, 0, 1);
        put_u32(&mut bytes, 4, 4);
        put_u32(&mut bytes, 16, 20);
        put_u32(&mut bytes, 24, 8);

        assert!(TextureResource::parse(&bytes).is_err());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}
