use std::io;

use crate::{Iso9660, invalid_data};

const AFS_HEADER_SIZE: u64 = 8;
const AFS_ENTRY_SIZE: u64 = 8;
const AFS_MAX_FILES: u32 = 65_536;

pub struct AfsArchive {
    path: String,
    file_size: u64,
    entry_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfsEntry {
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionEntry {
    pub partition: u8,
    pub index: u32,
}

impl AfsArchive {
    pub fn open(iso: &mut Iso9660, path: impl Into<String>) -> io::Result<Self> {
        let path = path.into();
        let file_size = iso.file_size(&path)?;
        let header = iso.read_file_range(&path, 0, AFS_HEADER_SIZE as usize)?;
        if &header[..4] != b"AFS\0" {
            return Err(invalid_data("missing AFS signature"));
        }

        let entry_count = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if entry_count > AFS_MAX_FILES {
            return Err(invalid_data("AFS entry count exceeds the format limit"));
        }
        let table_size = AFS_HEADER_SIZE
            .checked_add(
                u64::from(entry_count + 1)
                    .checked_mul(AFS_ENTRY_SIZE)
                    .ok_or_else(|| invalid_data("AFS table size overflow"))?,
            )
            .ok_or_else(|| invalid_data("AFS table size overflow"))?;
        if table_size > file_size {
            return Err(invalid_data("AFS entry table exceeds archive size"));
        }

        Ok(Self {
            path,
            file_size,
            entry_count,
        })
    }

    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    pub fn entry(&self, iso: &mut Iso9660, index: u32) -> io::Result<AfsEntry> {
        if index >= self.entry_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("AFS entry index {index} exceeds count {}", self.entry_count),
            ));
        }

        let table_offset = AFS_HEADER_SIZE + u64::from(index) * AFS_ENTRY_SIZE;
        let row = iso.read_file_range(&self.path, table_offset, AFS_ENTRY_SIZE as usize)?;
        let entry = AfsEntry {
            offset: u32::from_le_bytes(row[0..4].try_into().unwrap()),
            size: u32::from_le_bytes(row[4..8].try_into().unwrap()),
        };
        let end = u64::from(entry.offset)
            .checked_add(u64::from(entry.size))
            .filter(|end| *end <= self.file_size)
            .ok_or_else(|| invalid_data("AFS entry exceeds archive size"))?;
        let _ = end;
        Ok(entry)
    }

    pub fn read_entry(&self, iso: &mut Iso9660, index: u32) -> io::Result<Vec<u8>> {
        let entry = self.entry(iso, index)?;
        let size = usize::try_from(entry.size)
            .map_err(|_| invalid_data("AFS entry is too large for this platform"))?;
        iso.read_file_range(&self.path, u64::from(entry.offset), size)
    }

    pub fn read_entry_prefix(
        &self,
        iso: &mut Iso9660,
        index: u32,
        max_size: usize,
    ) -> io::Result<Vec<u8>> {
        let entry = self.entry(iso, index)?;
        let size = usize::try_from(entry.size)
            .map_err(|_| invalid_data("AFS entry is too large for this platform"))?
            .min(max_size);
        iso.read_file_range(&self.path, u64::from(entry.offset), size)
    }
}

pub fn partition_entry_for_id(file_id: u32) -> PartitionEntry {
    if file_id == 0 {
        PartitionEntry {
            partition: 0,
            index: 0,
        }
    } else if file_id < 0xD48 {
        PartitionEntry {
            partition: 1,
            index: file_id - 1,
        }
    } else {
        PartitionEntry {
            partition: 2,
            index: file_id - 0xD48,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PartitionEntry, partition_entry_for_id};

    #[test]
    fn maps_global_file_ids_to_afs_partitions() {
        assert_eq!(
            partition_entry_for_id(0),
            PartitionEntry {
                partition: 0,
                index: 0
            }
        );
        assert_eq!(
            partition_entry_for_id(2),
            PartitionEntry {
                partition: 1,
                index: 1
            }
        );
        assert_eq!(
            partition_entry_for_id(0xD47),
            PartitionEntry {
                partition: 1,
                index: 0xD46
            }
        );
        assert_eq!(
            partition_entry_for_id(0xD48),
            PartitionEntry {
                partition: 2,
                index: 0
            }
        );
    }
}
