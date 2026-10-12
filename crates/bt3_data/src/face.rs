//! The face resources of a battle character file (`BtlObj_BindTables`): the mouth meshes that replace the
//! mouth part (entries 4..11, one per mouth shape, VIF chains like the model's parts), the eye texture table
//! (entry 0xD), the texture tables of mouth shapes 5..8 (entries 0xE..0x11) and the 100 lip-sync tracks of each
//! voice set (entries 0x35.. for the first, 0x99.. for the second).

use std::io;

use crate::model::{Batch, parse_chain};

/// Entries 4..11: the mouth meshes of shapes 1..8 (`mdl->mouthA`).
pub const MOUTH_CHAIN_FIRST: usize = 4;
/// Entry 0xD: the eye textures (`mdl->eyes`).
pub const EYE_TABLE: usize = 0xD;
/// Entries 0xE..0x11: the texture tables of mouth shapes 5..8 (`mdl->mouthB[4..8]`).
pub const MOUTH_TEX_FIRST: usize = 0xE;
/// The lip tracks of the first voice set; the second set (save flag bit 0, as a fresh save has it) is 0x64 on.
pub const LIP_FIRST: usize = 0x35;
pub const LIP_SECOND_SET: usize = 0x64;
pub const LIP_TRACKS: usize = 100;

/// `BObjLipKey`: from `frame` on the mouth talks (`code` other than 0) or is shut (0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LipKey {
    pub frame: u16,
    pub code: u16,
}

/// `BObjLipData`: a record count at +8 and the records from +0x10.
pub fn parse_lip_track(bytes: &[u8]) -> io::Result<Vec<LipKey>> {
    let count = bytes
        .get(8..10)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| invalid_data("lip track header is truncated"))?;
    let keys = bytes
        .get(0x10..0x10 + count * 4)
        .ok_or_else(|| invalid_data("lip track records exceed the entry"))?;
    Ok(keys
        .chunks_exact(4)
        .map(|k| LipKey {
            frame: u16::from_le_bytes([k[0], k[1]]),
            code: u16::from_le_bytes([k[2], k[3]]),
        })
        .collect())
}

/// What a character file gives the face besides its textures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceData {
    /// Per mouth shape 1..8, the mesh drawn in place of the mouth part; `None` for an empty entry.
    pub mouth_chains: [Option<Vec<Batch>>; 8],
    /// The voice set's lip tracks by line, `None` for an empty entry.
    pub lips: Vec<Option<Vec<LipKey>>>,
}

impl FaceData {
    /// From a character file; `second_set` picks the lip tracks of the second voice set.
    pub fn from_file(file: &[u8], second_set: bool) -> io::Result<Self> {
        let mut mouth_chains: [Option<Vec<Batch>>; 8] = Default::default();
        for (i, chain) in mouth_chains.iter_mut().enumerate() {
            if let Some(stream) = crate::pack_entry(file, MOUTH_CHAIN_FIRST + i) {
                *chain = Some(parse_chain(stream)?);
            }
        }
        let base = LIP_FIRST + if second_set { LIP_SECOND_SET } else { 0 };
        let lips = (0..LIP_TRACKS)
            .map(|i| {
                crate::pack_entry(file, base + i)
                    .map(parse_lip_track)
                    .transpose()
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { mouth_chains, lips })
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lip_track_reads_its_records() {
        let mut b = vec![0u8; 0x18];
        b[0..4].copy_from_slice(b"LIPS");
        b[8] = 2;
        b[0x10..0x14].copy_from_slice(&[0, 0, 1, 0]);
        b[0x14..0x18].copy_from_slice(&[30, 0, 0, 0]);
        assert_eq!(
            parse_lip_track(&b).unwrap(),
            vec![LipKey { frame: 0, code: 1 }, LipKey { frame: 30, code: 0 }]
        );
        b[8] = 3;
        assert!(parse_lip_track(&b).is_err());
    }
}
