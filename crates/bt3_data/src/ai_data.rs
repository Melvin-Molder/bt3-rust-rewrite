//! The CPU player's data: member 4 of common file 2 (`AiThink_BindData`): the action table (the scripted
//! actions' steps, per action its step range, pick function, timeout and flags, per fighter motion a class and a
//! flag byte), eight rule lists and 32 rate profiles; and the rule-condition tables of the executable.

use std::io;

use crate::elf::Elf;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Actions (sequence ids) the action table describes.
pub const ACTION_COUNT: usize = 0x80;
/// Fighter motion ids the class and flag tables cover.
pub const STATE_COUNT: usize = 0x200;

/// `BtlAiSeqStep`: what one step of a scripted action presses and what ends it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiStep {
    /// Index of the step handler that says "finished".
    pub handler: i8,
    /// Steps of one action with the same group are alternatives.
    pub group: i8,
    /// `BtlAiPad_Set`'s special argument.
    pub special: i16,
    /// The state class the fighter is in once the input took effect, -1 none.
    pub cls: i16,
    /// AI button bits sent every frame / every other frame / once.
    pub hold: u32,
    pub press: u32,
    pub once: u32,
}

/// `AiThRule` (0x1C bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiRule {
    /// Situation group: a bit of the sensed situation word.
    pub group: i16,
    /// Condition ids (0xFF ends, 0xFE repeats the result at that position of the previous rule) and arguments.
    pub cond: [u8; 8],
    pub arg: [u8; 8],
    /// Up to four actions (0xFF ends); the first is also the rule's kind (0 stop, 4 choose the next list).
    pub acts: [u8; 4],
    /// Their argument bytes; for kind 4 the first is the list choice + 0x7F.
    pub args: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AiData {
    pub steps: Vec<AiStep>,
    pub first: [u16; ACTION_COUNT],
    pub count: [u8; ACTION_COUNT],
    pub pick: [i8; ACTION_COUNT],
    pub timeout: [i8; ACTION_COUNT],
    pub flags: [u16; ACTION_COUNT],
    pub state_class: [i8; STATE_COUNT],
    pub state_flags: [u8; STATE_COUNT],
    pub rules: [Vec<AiRule>; 8],
    /// Per AI type: the profile bytes (`{ s32; rates at level 0; rates at level 29 }`), `None` where the size is 0.
    pub profiles: Vec<Option<Vec<u8>>>,
}

impl AiData {
    /// Reads the AI data of common file 2 (its header word 4 is the byte offset of the member).
    pub fn parse(common: &[u8]) -> io::Result<Self> {
        let word = |o: usize| -> io::Result<u32> {
            common
                .get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("AI data is truncated"))
        };
        let base = (word(0x10)? & !3) as usize;
        let mut size = [0usize; 41];
        for (i, s) in size.iter_mut().enumerate() {
            *s = word(base + i * 4)? as usize;
        }
        let bytes = |o: usize, n: usize| -> io::Result<&[u8]> {
            common
                .get(o..o + n)
                .ok_or_else(|| invalid_data("AI data is truncated"))
        };
        let act = base + 0x148;
        let table = bytes(act, size[0])?;
        if table.len() < 0x788 {
            return Err(invalid_data("AI action table is too small"));
        }
        let u16_at = |o: usize| u16::from_le_bytes([table[o], table[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(table[o..o + 4].try_into().unwrap());
        let body = 8;
        let first = std::array::from_fn(|i| u16_at(body + i * 2));
        let count = std::array::from_fn(|i| table[body + 0x100 + i]);
        let pick = std::array::from_fn(|i| table[body + 0x180 + i] as i8);
        let timeout = std::array::from_fn(|i| table[body + 0x200 + i] as i8);
        let flags = std::array::from_fn(|i| u16_at(body + 0x280 + i * 2));
        let state_class = std::array::from_fn(|i| table[body + 0x380 + i] as i8);
        let state_flags = std::array::from_fn(|i| table[body + 0x580 + i]);
        let steps = table[0x788..]
            .chunks_exact(0x14)
            .enumerate()
            .map(|(i, _)| {
                let o = 0x788 + i * 0x14;
                AiStep {
                    handler: table[o] as i8,
                    group: table[o + 1] as i8,
                    special: u16_at(o + 4) as i16,
                    cls: u16_at(o + 6) as i16,
                    hold: u32_at(o + 8),
                    press: u32_at(o + 0xC),
                    once: u32_at(o + 0x10),
                }
            })
            .collect();
        let mut p = act + size[0];
        let mut rules: [Vec<AiRule>; 8] = Default::default();
        for (i, list) in rules.iter_mut().enumerate() {
            let section = bytes(p, size[1 + i])?;
            let n = i16::from_le_bytes([section[0], section[1]]).max(0) as usize;
            let records = section
                .get(8..8 + n * 0x1C)
                .ok_or_else(|| invalid_data("AI rule list is truncated"))?;
            *list = records
                .chunks_exact(0x1C)
                .map(|r| AiRule {
                    group: i16::from_le_bytes([r[0], r[1]]),
                    cond: r[4..0xC].try_into().unwrap(),
                    arg: r[0xC..0x14].try_into().unwrap(),
                    acts: r[0x14..0x18].try_into().unwrap(),
                    args: r[0x18..0x1C].try_into().unwrap(),
                })
                .collect();
            p += size[1 + i];
        }
        let mut profiles = Vec::with_capacity(32);
        for i in 0..32 {
            let n = size[9 + i];
            profiles.push(
                (n != 0)
                    .then(|| bytes(p, n).map(<[u8]>::to_vec))
                    .transpose()?,
            );
            p += n;
        }
        Ok(Self {
            steps,
            first,
            count,
            pick,
            timeout,
            flags,
            state_class,
            state_flags,
            rules,
            profiles,
        })
    }

    /// The class of a fighter motion (`actClass` / `stateClass`), 0 outside the table.
    pub fn class_of(&self, motion: i32) -> i8 {
        usize::try_from(motion)
            .ok()
            .and_then(|m| self.state_class.get(m))
            .copied()
            .unwrap_or(0)
    }

    /// The flag byte of a fighter motion, 0 outside the table.
    pub fn flags_of(&self, motion: i32) -> u8 {
        usize::try_from(motion)
            .ok()
            .and_then(|m| self.state_flags.get(m))
            .copied()
            .unwrap_or(0)
    }
}

/// The executable's tables of the rule evaluator (0x2EDA70..0x2EDF08): per condition id the index of its
/// condition function, and per weight table the condition code (id - 35 or id - 40) of each column. The column
/// tables are kept as the one block they are in memory: a row index past a table's end reads the next table.
#[derive(Clone, Debug, PartialEq)]
pub struct AiTables {
    pub cond_index: [i32; 128],
    pub cols: Vec<u8>,
}

impl AiTables {
    /// Offsets in `cols` of the tables basic [14][8], A [14][8], B [14][8], act [24], C [14][4], D [20][8],
    /// E [14][4] and F [4][8].
    pub const BASIC: usize = 0;
    pub const A: usize = 0x70;
    pub const B: usize = 0xE0;
    pub const ACT: usize = 0x150;
    pub const C: usize = 0x168;
    pub const D: usize = 0x1A0;
    pub const E: usize = 0x240;
    pub const F: usize = 0x278;

    pub fn read(elf: &Elf) -> io::Result<Self> {
        let index = elf.i32s(0x2EDA70, 128)?;
        Ok(Self {
            cond_index: index.try_into().unwrap(),
            cols: elf.read(0x2EDC70, 0x2EDF08 - 0x2EDC70)?.to_vec(),
        })
    }

    /// The condition function of a condition id, 0 (never) outside the table.
    pub fn func(&self, cond: i32) -> i32 {
        usize::try_from(cond)
            .ok()
            .and_then(|c| self.cond_index.get(c))
            .copied()
            .unwrap_or(0)
    }

    /// A byte of the column block, 0 past its end.
    pub fn col(&self, at: usize) -> u8 {
        self.cols.get(at).copied().unwrap_or(0)
    }
}
