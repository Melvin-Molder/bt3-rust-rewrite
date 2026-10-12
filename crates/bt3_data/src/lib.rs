use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub mod adx;
pub mod afs;
pub mod ai_data;
pub mod bpe;
pub mod burst;
pub mod cam_anim;
pub mod chara_data;
pub mod eft_mesh;
pub mod eft_pack;
pub mod elf;
pub mod face;
pub mod fod;
pub mod font;
pub mod gs;
pub mod model;
pub mod motion;
pub mod snd;
pub mod stage;
pub mod stage_anim;
pub mod stage_col;
pub mod stage_model;
pub mod stage_surf;
pub mod survival;
pub mod tex;

const SECTOR_SIZE: u64 = 2048;
const PRIMARY_VOLUME_DESCRIPTOR_LBA: u64 = 16;

pub struct Iso9660 {
    file: File,
    file_size: u64,
    block_size: u16,
    volume_blocks: u32,
    volume_id: String,
    root_extent: u32,
    root_size: u32,
}

impl Iso9660 {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let file_size = file.metadata()?.len();
        let descriptor = read_at(
            &mut file,
            PRIMARY_VOLUME_DESCRIPTOR_LBA * SECTOR_SIZE,
            SECTOR_SIZE as usize,
            file_size,
        )?;

        if descriptor[0] != 1 || &descriptor[1..6] != b"CD001" || descriptor[6] != 1 {
            return Err(invalid_data("missing ISO9660 primary volume descriptor"));
        }

        let volume_blocks = read_both_endian_u32(&descriptor[80..88])?;
        let block_size = read_both_endian_u16(&descriptor[128..132])?;
        if u64::from(block_size) != SECTOR_SIZE {
            return Err(invalid_data("unsupported ISO9660 logical block size"));
        }

        let root_record = parse_directory_record(&descriptor[156..])?;
        if root_record.flags & 0x02 == 0 {
            return Err(invalid_data("ISO9660 root record is not a directory"));
        }

        let volume_id = String::from_utf8_lossy(&descriptor[40..72])
            .trim_matches([' ', '\0'])
            .to_owned();

        Ok(Self {
            file,
            file_size,
            block_size,
            volume_blocks,
            volume_id,
            root_extent: root_record.extent,
            root_size: root_record.size,
        })
    }

    pub fn volume_id(&self) -> &str {
        &self.volume_id
    }

    pub fn volume_blocks(&self) -> u32 {
        self.volume_blocks
    }

    pub fn read_file(&mut self, path: &str) -> io::Result<Vec<u8>> {
        let record = self.resolve_file(path)?;
        self.read_extent(record.extent, record.size)
    }

    pub fn file_size(&mut self, path: &str) -> io::Result<u64> {
        Ok(u64::from(self.resolve_file(path)?.size))
    }

    pub fn read_file_range(&mut self, path: &str, offset: u64, size: usize) -> io::Result<Vec<u8>> {
        let record = self.resolve_file(path)?;
        let end = offset
            .checked_add(u64::try_from(size).map_err(|_| invalid_data("range is too large"))?)
            .filter(|end| *end <= u64::from(record.size))
            .ok_or_else(|| invalid_data("requested range exceeds ISO9660 file"))?;
        let _ = end;
        let base = u64::from(record.extent)
            .checked_mul(u64::from(self.block_size))
            .ok_or_else(|| invalid_data("ISO9660 extent offset overflow"))?;
        let absolute_offset = base
            .checked_add(offset)
            .ok_or_else(|| invalid_data("ISO9660 file offset overflow"))?;
        read_at(&mut self.file, absolute_offset, size, self.file_size)
    }

    fn resolve_file(&mut self, path: &str) -> io::Result<DirectoryRecord> {
        let components: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
        if components.is_empty() {
            return Err(invalid_data("ISO9660 path is empty"));
        }

        let mut parent_extent = self.root_extent;
        let mut parent_size = self.root_size;
        for (index, component) in components.iter().enumerate() {
            let record = self.find_child(parent_extent, parent_size, component)?;
            let is_last = index + 1 == components.len();
            if is_last {
                if record.flags & 0x02 != 0 {
                    return Err(invalid_data("ISO9660 path resolves to a directory"));
                }
                return Ok(record);
            }
            if record.flags & 0x02 == 0 {
                return Err(invalid_data("ISO9660 path component is not a directory"));
            }
            parent_extent = record.extent;
            parent_size = record.size;
        }

        Err(invalid_data("invalid ISO9660 path"))
    }

    fn find_child(
        &mut self,
        directory_extent: u32,
        directory_size: u32,
        requested_name: &str,
    ) -> io::Result<DirectoryRecord> {
        let directory = self.read_extent(directory_extent, directory_size)?;
        let mut offset = 0usize;

        while offset < directory.len() {
            let record_length = usize::from(directory[offset]);
            if record_length == 0 {
                let next_sector =
                    (offset as u64 / u64::from(self.block_size) + 1) * u64::from(self.block_size);
                offset = next_sector.min(directory.len() as u64) as usize;
                continue;
            }

            let end = offset
                .checked_add(record_length)
                .filter(|end| *end <= directory.len())
                .ok_or_else(|| invalid_data("ISO9660 directory record exceeds directory"))?;
            let record = parse_directory_record(&directory[offset..end])?;

            if iso_name_matches(&record.name, requested_name) {
                if record.flags & 0x80 != 0 {
                    return Err(invalid_data("multi-extent ISO9660 entries are unsupported"));
                }
                return Ok(record);
            }

            offset = end;
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{requested_name} was not found in the ISO9660 directory"),
        ))
    }

    fn read_extent(&mut self, extent: u32, size: u32) -> io::Result<Vec<u8>> {
        let offset = u64::from(extent)
            .checked_mul(u64::from(self.block_size))
            .ok_or_else(|| invalid_data("ISO9660 extent offset overflow"))?;
        let size = usize::try_from(size).map_err(|_| invalid_data("file is too large"))?;
        read_at(&mut self.file, offset, size, self.file_size)
    }
}

struct DirectoryRecord {
    name: Vec<u8>,
    extent: u32,
    size: u32,
    flags: u8,
}

fn parse_directory_record(bytes: &[u8]) -> io::Result<DirectoryRecord> {
    if bytes.len() < 34 || usize::from(bytes[0]) > bytes.len() {
        return Err(invalid_data("invalid ISO9660 directory record"));
    }

    let record_length = usize::from(bytes[0]);
    let name_length = usize::from(bytes[32]);
    let name_end = 33usize
        .checked_add(name_length)
        .filter(|end| *end <= record_length)
        .ok_or_else(|| invalid_data("invalid ISO9660 identifier length"))?;

    Ok(DirectoryRecord {
        name: bytes[33..name_end].to_vec(),
        extent: read_both_endian_u32(&bytes[2..10])?
            .checked_add(u32::from(bytes[1]))
            .ok_or_else(|| invalid_data("ISO9660 extent overflow"))?,
        size: read_both_endian_u32(&bytes[10..18])?,
        flags: bytes[25],
    })
}

fn read_both_endian_u16(bytes: &[u8]) -> io::Result<u16> {
    if bytes.len() != 4 {
        return Err(invalid_data("invalid ISO9660 16-bit value"));
    }
    let little = u16::from_le_bytes([bytes[0], bytes[1]]);
    let big = u16::from_be_bytes([bytes[2], bytes[3]]);
    if little != big {
        return Err(invalid_data("inconsistent ISO9660 16-bit value"));
    }
    Ok(little)
}

fn read_both_endian_u32(bytes: &[u8]) -> io::Result<u32> {
    if bytes.len() != 8 {
        return Err(invalid_data("invalid ISO9660 32-bit value"));
    }
    let little = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let big = u32::from_be_bytes(bytes[4..].try_into().unwrap());
    if little != big {
        return Err(invalid_data("inconsistent ISO9660 32-bit value"));
    }
    Ok(little)
}

fn iso_name_matches(identifier: &[u8], requested_name: &str) -> bool {
    if identifier == [0] || identifier == [1] {
        return false;
    }

    let identifier = String::from_utf8_lossy(identifier);
    let name_without_version = identifier.split(';').next().unwrap_or(&identifier);
    name_without_version.eq_ignore_ascii_case(requested_name)
}

fn read_at(file: &mut File, offset: u64, size: usize, file_size: u64) -> io::Result<Vec<u8>> {
    offset
        .checked_add(u64::try_from(size).map_err(|_| invalid_data("read is too large"))?)
        .filter(|end| *end <= file_size)
        .ok_or_else(|| invalid_data("ISO9660 extent is outside the image"))?;

    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `BObjFile_GetEntry`: entry `n` of a pack, `None` when it is empty (its offset equals the next one's).
pub fn pack_entry(pack: &[u8], n: usize) -> Option<&[u8]> {
    let word = |i: usize| {
        pack.get(i * 4..i * 4 + 4)
            .map(|b| (u32::from_le_bytes(b.try_into().unwrap()) & !3) as usize)
    };
    let start = word(n)?;
    (word(n + 1)? != start).then(|| pack.get(start..)).flatten()
}

/// `MPACK_AT`: section `n` of a menu pack starts at header word `n`, rounded down to a word.
pub fn pack_section(pack: &[u8], n: usize) -> io::Result<&[u8]> {
    let word = pack
        .get(n * 4..n * 4 + 4)
        .ok_or_else(|| invalid_data("pack header is truncated"))?;
    let offset = (u32::from_le_bytes(word.try_into().unwrap()) & !3) as usize;
    pack.get(offset..)
        .ok_or_else(|| invalid_data("pack section starts outside the pack"))
}

#[cfg(test)]
mod tests {
    use super::{iso_name_matches, read_both_endian_u32};

    #[test]
    fn accepts_matching_both_endian_values() {
        assert_eq!(
            read_both_endian_u32(&[0x34, 0x12, 0, 0, 0, 0, 0x12, 0x34]).unwrap(),
            0x1234
        );
    }

    #[test]
    fn rejects_mismatched_both_endian_values() {
        assert!(read_both_endian_u32(&[1, 0, 0, 0, 0, 0, 0, 2]).is_err());
    }

    #[test]
    fn matches_iso_file_names_without_version_suffix() {
        assert!(iso_name_matches(b"SYSTEM.CNF;1", "system.cnf"));
        assert!(!iso_name_matches(b"SYSTEM.CNF;1", "BOOT.CNF"));
    }
}
