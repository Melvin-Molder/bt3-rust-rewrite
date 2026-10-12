//! GSC script files (`GSCF`, read by the script engine of src/sys/gsc.c): the battle scripts of story mode.
//!
//! Every chunk is a 0x10-byte header `{tag, headSize, dataSize, arg}`, `dataSize` bytes and a 0x10-byte `EOFC`
//! trailer, so the next one starts `0x20 + dataSize` on. The file chunk `GSCF` holds `GSHD` (unread), `GSCD` (a
//! run of `GSAC` actions, `arg` the action id, -1 the main one, data the script words) and `GSDT` (4-byte operand
//! values: ints, floats, and strings whose bytes start at their value).

use std::io;

/// Low nibble of a script word.
pub const WORD_END: u32 = 0;
/// Byte 1 = plain operand count, u16 at +2 = command id.
pub const WORD_COMMAND: u32 = 1;
/// s16 at +2 = action: start it and go on.
pub const WORD_CALL: u32 = 2;
/// s16 at +2 = action: start it and wait for it.
pub const WORD_CALL_WAIT: u32 = 3;
/// Byte 1 = option letter, byte 2 = operand count.
pub const WORD_OPTION: u32 = 8;
/// Bits 8..31 = index of a 4-byte value in `GSDT`.
pub const WORD_OPERAND: u32 = 10;
/// Set in every word that belongs to the command before it.
pub const WORD_ARG: u32 = 8;

const TAG_GSAC: &[u8; 4] = b"GSAC";

/// One action (`GSAC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GscAction {
    pub id: i32,
    pub words: Vec<u32>,
}

/// A script file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GscScript {
    pub actions: Vec<GscAction>,
    /// `GSDT`'s values.
    pub data: Vec<u8>,
}

fn u32_at(b: &[u8], o: usize) -> io::Result<u32> {
    b.get(o..o + 4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "GSC chunk is cut short"))
}

/// The chunk at `at`: its tag, head size, data size, arg.
fn chunk(b: &[u8], at: usize) -> io::Result<([u8; 4], usize, usize, i32)> {
    let tag = b
        .get(at..at + 4)
        .map(|t| [t[0], t[1], t[2], t[3]])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "GSC chunk is cut short"))?;
    Ok((
        tag,
        u32_at(b, at + 4)? as usize,
        u32_at(b, at + 8)? as usize,
        u32_at(b, at + 12)? as i32,
    ))
}

/// `GSC_NEXT_CHUNK`: past the header, the data and the trailer.
fn next_chunk(at: usize, data_size: usize) -> usize {
    at + 0x20 + data_size
}

impl GscScript {
    /// `Gsc_LoadFile`'s view of the file: the head after the file chunk's header, the code and the data after it.
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let (tag, head_size, _, _) = chunk(file, 0)?;
        if &tag != b"GSCF" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a GSC file"));
        }
        let head = head_size;
        let (_, _, head_data, _) = chunk(file, head)?;
        let code = next_chunk(head, head_data);
        let (_, code_head, code_data, _) = chunk(file, code)?;
        let data = next_chunk(code, code_data);
        let (_, data_head, data_size, _) = chunk(file, data)?;
        let values = file
            .get(data + data_head..data + data_head + data_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "GSDT is cut short"))?
            .to_vec();
        // The actions: GSAC chunks from the code's data on, as long as the tag is GSAC.
        let mut actions = Vec::new();
        let mut at = code + code_head;
        while let Ok((tag, head, size, id)) = chunk(file, at) {
            if &tag != TAG_GSAC {
                break;
            }
            let words = file
                .get(at + head..at + head + size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "GSAC is cut short"))?
                .chunks_exact(4)
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                .collect();
            actions.push(GscAction { id, words });
            at = next_chunk(at, size);
        }
        Ok(Self {
            actions,
            data: values,
        })
    }

    /// `Gsc_FindAction`: the first action with this id.
    pub fn action(&self, id: i32) -> Option<usize> {
        self.actions.iter().position(|a| a.id == id)
    }

    /// Word `at` of an action (0, the end, past its last).
    pub fn word(&self, action: usize, at: usize) -> u32 {
        self.actions
            .get(action)
            .and_then(|a| a.words.get(at))
            .copied()
            .unwrap_or(WORD_END)
    }

    /// The 4-byte value an operand word points at.
    pub fn value(&self, word: u32) -> Option<[u8; 4]> {
        let o = (word >> 8) as usize * 4;
        self.data.get(o..o + 4).map(|v| [v[0], v[1], v[2], v[3]])
    }

    /// The bytes of a string operand, up to its NUL.
    pub fn string(&self, word: u32) -> &[u8] {
        let o = (word >> 8) as usize * 4;
        let s = self.data.get(o..).unwrap_or(&[]);
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        &s[..end]
    }

    /// `Gsc_NextCommand`: the word after a command, its plain operands and its options.
    pub fn next_command(&self, action: usize, at: usize) -> usize {
        let mut count = ((self.word(action, at) >> 8) & 0xFF) as usize;
        let mut w = at + count + 1;
        loop {
            let word = self.word(action, w);
            if word & WORD_ARG == 0 {
                break;
            }
            if word & 0xF == WORD_OPTION {
                let n = ((word >> 16) & 0xFF) as usize;
                count += n + 1;
                w += n + 1;
            } else {
                w += 1;
            }
        }
        at + count + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_bytes(tag: &[u8; 4], arg: i32, data: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend(0x10u32.to_le_bytes());
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(arg.to_le_bytes());
        v.extend(data);
        v.extend(b"EOFC");
        v.extend([0; 12]);
        v
    }

    #[test]
    fn a_file_gives_its_actions_and_values() {
        let words = |w: &[u32]| w.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        // Action -1: command 7 with one operand and option -l with one, then end.
        let main = words(&[
            (7 << 16) | (1 << 8) | WORD_COMMAND,
            WORD_OPERAND,
            (1 << 16) | (u32::from(b'l') << 8) | WORD_OPTION,
            (1 << 8) | WORD_OPERAND,
            WORD_END,
        ]);
        let mut gsac = chunk_bytes(b"GSAC", -1, &main);
        gsac.extend(chunk_bytes(b"GSAC", 1000, &words(&[WORD_END])));
        let code = chunk_bytes(b"GSCD", 0, &gsac);
        let mut values = 5i32.to_le_bytes().to_vec();
        values.extend(b"hi\0\0");
        let mut body = chunk_bytes(b"GSHD", 0, &[]);
        body.extend(code);
        body.extend(chunk_bytes(b"GSDT", 0, &values));
        // The file chunk's data is its children (headSize 0x10, so they start right after its header).
        let mut file = b"GSCF".to_vec();
        file.extend(0x10u32.to_le_bytes());
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(0i32.to_le_bytes());
        file.extend(body);
        let s = GscScript::parse(&file).unwrap();
        assert_eq!(s.actions.len(), 2);
        assert_eq!(
            (s.action(-1), s.action(1000), s.action(3)),
            (Some(0), Some(1), None)
        );
        assert_eq!(s.next_command(0, 0), 4);
        assert_eq!(s.value(WORD_OPERAND), Some(5i32.to_le_bytes()));
        assert_eq!(s.string((1 << 8) | WORD_OPERAND), b"hi");
        assert_eq!(s.word(0, 99), WORD_END);
    }
}
