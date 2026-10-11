//! The game's executable (`SYSTEM.CNF`'s `BOOT2`, an ELF32 MIPS file): constant tables are read from it by
//! their load address, through the program headers.

use std::io;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The loadable segments of the executable.
pub struct Elf {
    bytes: Vec<u8>,
    /// (virtual address, file offset, file size) of each `PT_LOAD` segment.
    segments: Vec<(u32, usize, usize)>,
}

impl Elf {
    pub fn parse(bytes: Vec<u8>) -> io::Result<Self> {
        if bytes.get(0..4) != Some(b"\x7FELF")
            || bytes.get(4) != Some(&1)
            || bytes.get(5) != Some(&1)
        {
            return Err(invalid_data("not a little-endian ELF32 file"));
        }
        let u16_at = |o: usize| {
            bytes
                .get(o..o + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| invalid_data("ELF header is truncated"))
        };
        let u32_at = |o: usize| {
            bytes
                .get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("ELF header is truncated"))
        };
        let phoff = u32_at(0x1C)? as usize;
        let phentsize = usize::from(u16_at(0x2A)?);
        let phnum = usize::from(u16_at(0x2C)?);
        let mut segments = Vec::new();
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if u32_at(ph)? == 1 {
                let offset = u32_at(ph + 4)? as usize;
                let size = u32_at(ph + 0x10)? as usize;
                if offset.checked_add(size).is_none_or(|end| end > bytes.len()) {
                    return Err(invalid_data("ELF segment exceeds the file"));
                }
                segments.push((u32_at(ph + 8)?, offset, size));
            }
        }
        Ok(Self { bytes, segments })
    }

    /// `len` bytes loaded at `addr`.
    pub fn read(&self, addr: u32, len: usize) -> io::Result<&[u8]> {
        self.segments
            .iter()
            .find_map(|&(vaddr, offset, size)| {
                let rel = addr.checked_sub(vaddr)? as usize;
                (rel + len <= size).then(|| &self.bytes[offset + rel..offset + rel + len])
            })
            .ok_or_else(|| invalid_data("address is outside the loaded segments"))
    }

    pub fn f32s(&self, addr: u32, count: usize) -> io::Result<Vec<f32>> {
        Ok(self
            .read(addr, count * 4)?
            .chunks_exact(4)
            .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
            .collect())
    }

    pub fn i32s(&self, addr: u32, count: usize) -> io::Result<Vec<i32>> {
        Ok(self
            .read(addr, count * 4)?
            .chunks_exact(4)
            .map(|w| i32::from_le_bytes(w.try_into().unwrap()))
            .collect())
    }
}

/// The ISO path of the executable `SYSTEM.CNF` boots (`BOOT2 = cdrom0:\SLUS_216.78;1`).
pub fn boot_path(system_cnf: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(system_cnf);
    let line = text.lines().find(|l| l.trim_start().starts_with("BOOT2"))?;
    let value = line.split_once('=')?.1.trim();
    let name = value.rsplit(['\\', ':']).next()?;
    Some(name.split(';').next()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_path_is_the_file_name_of_boot2() {
        let cnf = b"BOOT2 = cdrom0:\\SLUS_216.78;1\r\nVER = 1.00\r\n";
        assert_eq!(boot_path(cnf).as_deref(), Some("SLUS_216.78"));
    }
}
