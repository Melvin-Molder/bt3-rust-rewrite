use std::io;

use crate::invalid_data;

/// A sound-effect bank file as `Snd_LoadBank` (src/sys/snd.c) reads it: five words `{?, bodyOfs, hdOfs, seqOfs,
/// endOfs}`, then the ADPCM sample body (BD), the Sony HD header and the SE sequences (SQ). SOUNDS.IRX hands the HD to
/// modhsyn and the SQ to modsesq2.
pub struct SeBank<'a> {
    /// The BD section: SPU2 ADPCM blocks, uploaded to sound RAM as is.
    pub body: &'a [u8],
    /// `Vagi` chunk: the samples.
    pub vags: Vec<Vag>,
    /// `Setb` chunk: timbre sets, each a list of timbres (None where the offset is -1).
    pub timbre_sets: Vec<Option<Vec<Option<Timbre>>>>,
    /// `Sesq` chunk.
    pub sesq: Sesq,
}

/// A `Vagi` parameter record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vag {
    /// Byte offset into the body.
    pub offset: u32,
    pub sample_rate: u16,
    pub attribute: u8,
}

/// An SE timbre: the tones for notes `note_min..=note_max`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timbre {
    pub note_min: u8,
    pub note_max: u8,
    pub tones: Vec<Tone>,
}

/// One 50-byte SE timbre entry, as modhsyn's SE note-on (0x8714) and voice setup (0x5a14, 0x5c30) read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tone {
    /// Index into `vags`; above the last one the voice plays noise.
    pub vag: u16,
    /// Velocity curve: low nibble selects the function, high nibble a custom table.
    pub velocity_curve: u8,
    pub volume: u8,
    /// 64 = centre.
    pub pan: i8,
    pub semitone: i8,
    /// 1/128 semitone.
    pub fine: i8,
    pub exclusive_group: u8,
    pub group_max: u8,
    pub priority: u8,
    /// Byte 12: bit 0 = surround panning (fold with phase inversion).
    pub pan_mode: u8,
    /// Byte 13: bits 0-3 dry L / dry R / effect L / effect R, bits 4-5 core.
    pub mix: u8,
    pub adsr1: u16,
    pub adsr2: u16,
    /// Byte 49: low nibble pitch LFO mode, high nibble amplitude LFO mode (`& 0x77 == 0` = off).
    pub lfo_mode: u8,
}

/// The `Sesq` chunk of the SQ section (modsesq2 sceSESq2Load / Play).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sesq {
    pub volume: u8,
    pub pan: u8,
    pub rate: u16,
    pub sets: Vec<Option<SeSet>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeSet {
    pub volume: u8,
    pub pan: u8,
    pub rate: u16,
    pub sequences: Vec<Option<SeSequence>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeSequence {
    pub volume: u8,
    pub pan: u8,
    pub rate: u16,
    /// Event data: VLQ delta times and events, up to the end of the SQ section (the interpreter stops at 0xFF).
    pub data: Vec<u8>,
}

impl<'a> SeBank<'a> {
    pub fn parse(file: &'a [u8]) -> io::Result<Self> {
        let body_ofs = u32_at(file, 4)? as usize;
        let hd_ofs = u32_at(file, 8)? as usize;
        let sq_ofs = u32_at(file, 12)? as usize;
        let end_ofs = u32_at(file, 16)? as usize;
        if !(body_ofs <= hd_ofs && hd_ofs <= sq_ofs && sq_ofs <= end_ofs && end_ofs <= file.len()) {
            return Err(invalid_data("SE bank sections are out of order"));
        }
        let hd = &file[hd_ofs..sq_ofs];
        let sq = &file[sq_ofs..end_ofs];
        Ok(Self {
            body: &file[body_ofs..hd_ofs],
            vags: parse_vags(hd)?,
            timbre_sets: parse_timbres(hd)?,
            sesq: parse_sesq(sq)?,
        })
    }

    /// The tone `set`/`timbre`/`note` selects, as 0x8714 looks it up.
    pub fn tone(&self, set: usize, timbre: usize, note: u8) -> Option<&Tone> {
        let timbre = self.timbre_sets.get(set)?.as_ref()?.get(timbre)?.as_ref()?;
        if note < timbre.note_min || note > timbre.note_max {
            return None;
        }
        timbre.tones.get(usize::from(note - timbre.note_min))
    }
}

/// Checks a chunk's "IECS" creator and its byte-reversed type tag.
fn check_chunk(data: &[u8], ofs: usize, tag: &[u8; 4]) -> io::Result<()> {
    let head = data
        .get(ofs..ofs + 8)
        .ok_or_else(|| invalid_data("SE bank chunk is outside its section"))?;
    if &head[..4] != b"IECS" || &head[4..] != tag {
        return Err(invalid_data("unexpected SE bank chunk"));
    }
    Ok(())
}

/// Chunk offset `n` of the HD `Head` chunk (at 0x10), or None for -1.
fn head_offset(hd: &[u8], field: usize) -> io::Result<Option<usize>> {
    check_chunk(hd, 0x10, b"daeH")?;
    let value = u32_at(hd, 0x10 + field)?;
    Ok((value != u32::MAX).then_some(value as usize))
}

fn parse_vags(hd: &[u8]) -> io::Result<Vec<Vag>> {
    let Some(chunk) = head_offset(hd, 0x20)? else {
        return Ok(Vec::new());
    };
    check_chunk(hd, chunk, b"igaV")?;
    let max = u32_at(hd, chunk + 0xC)? as usize;
    (0..=max)
        .map(|i| {
            let param = chunk + u32_at(hd, chunk + 0x10 + i * 4)? as usize;
            Ok(Vag {
                offset: u32_at(hd, param)?,
                sample_rate: u16_at(hd, param + 4)?,
                attribute: byte(hd, param + 6)?,
            })
        })
        .collect()
}

/// `Setb`, only read when the HD version byte (0x0E) is at least 2.
fn parse_timbres(hd: &[u8]) -> io::Result<Vec<Option<Vec<Option<Timbre>>>>> {
    if byte(hd, 0x0E)? < 2 {
        return Ok(Vec::new());
    }
    let Some(chunk) = head_offset(hd, 0x24)? else {
        return Ok(Vec::new());
    };
    check_chunk(hd, chunk, b"bteS")?;
    let max_set = u32_at(hd, chunk + 0xC)? as usize;
    (0..=max_set)
        .map(|s| {
            let set_ofs = u32_at(hd, chunk + 0x10 + s * 4)?;
            if set_ofs == u32::MAX {
                return Ok(None);
            }
            let set = chunk + set_ofs as usize;
            let max_timbre = u32_at(hd, set)? as usize;
            (0..=max_timbre)
                .map(|t| {
                    let timbre_ofs = u32_at(hd, set + 4 + t * 4)?;
                    if timbre_ofs == u32::MAX {
                        return Ok(None);
                    }
                    let timbre = chunk + timbre_ofs as usize;
                    let entries = timbre + u32_at(hd, timbre)? as usize;
                    let size = usize::from(byte(hd, timbre + 4)?);
                    let (note_min, note_max) = (byte(hd, timbre + 5)?, byte(hd, timbre + 6)?);
                    if note_max < note_min || size < 50 {
                        return Err(invalid_data("bad SE timbre header"));
                    }
                    let tones = (0..=usize::from(note_max - note_min))
                        .map(|n| parse_tone(hd, entries + n * size))
                        .collect::<io::Result<_>>()?;
                    Ok(Some(Timbre {
                        note_min,
                        note_max,
                        tones,
                    }))
                })
                .collect::<io::Result<Vec<_>>>()
                .map(Some)
        })
        .collect()
}

fn parse_tone(hd: &[u8], e: usize) -> io::Result<Tone> {
    Ok(Tone {
        vag: u16_at(hd, e)?,
        velocity_curve: byte(hd, e + 2)?,
        volume: byte(hd, e + 3)?,
        pan: byte(hd, e + 4)? as i8,
        semitone: byte(hd, e + 5)? as i8,
        fine: byte(hd, e + 6)? as i8,
        exclusive_group: byte(hd, e + 7)?,
        group_max: byte(hd, e + 8)?,
        priority: byte(hd, e + 9)?,
        pan_mode: byte(hd, e + 12)?,
        mix: byte(hd, e + 13)?,
        adsr1: u16_at(hd, e + 14)?,
        adsr2: u16_at(hd, e + 16)?,
        lfo_mode: byte(hd, e + 49)?,
    })
}

/// `Vers` gives the `Sequ` offset; `Sequ` +0x18 the `Sesq` offset (all relative to the SQ section).
fn parse_sesq(sq: &[u8]) -> io::Result<Sesq> {
    check_chunk(sq, 0, b"sreV")?;
    let sequ = u32_at(sq, 8)? as usize;
    check_chunk(sq, sequ, b"uqeS")?;
    let sesq_ofs = u32_at(sq, sequ + 0x18)?;
    if sesq_ofs == u32::MAX {
        return Err(invalid_data("SE bank has no Sesq chunk"));
    }
    let sesq = sesq_ofs as usize;
    check_chunk(sq, sesq, b"qseS")?;
    let max_set = u32_at(sq, sesq + 0xC)? as usize;
    let set_table = sesq + 0x10 + u32_at(sq, sesq + 0x10)? as usize;
    let sets = (0..=max_set)
        .map(|s| {
            let set_ofs = u32_at(sq, set_table + s * 4)?;
            if set_ofs == u32::MAX {
                return Ok(None);
            }
            let set = sesq + set_ofs as usize;
            let max_seq = u32_at(sq, set)? as usize;
            let seq_table = set + 4 + u32_at(sq, set + 4)? as usize;
            let sequences = (0..=max_seq)
                .map(|q| {
                    let seq_ofs = u32_at(sq, seq_table + q * 4)?;
                    if seq_ofs == u32::MAX {
                        return Ok(None);
                    }
                    let seq = sesq + seq_ofs as usize;
                    let data_ofs = u32_at(sq, seq)?;
                    if data_ofs == u32::MAX {
                        return Ok(None);
                    }
                    let data = sq.get(seq + data_ofs as usize..).ok_or_else(|| {
                        invalid_data("SE sequence data is outside the SQ section")
                    })?;
                    Ok(Some(SeSequence {
                        volume: byte(sq, seq + 4)?,
                        pan: byte(sq, seq + 5)?,
                        rate: u16_at(sq, seq + 6)?,
                        data: data.to_vec(),
                    }))
                })
                .collect::<io::Result<_>>()?;
            Ok(Some(SeSet {
                volume: byte(sq, set + 8)?,
                pan: byte(sq, set + 9)?,
                rate: u16_at(sq, set + 0xA)?,
                sequences,
            }))
        })
        .collect::<io::Result<_>>()?;
    Ok(Sesq {
        volume: byte(sq, sesq + 0x14)?,
        pan: byte(sq, sesq + 0x15)?,
        rate: u16_at(sq, sesq + 0x16)?,
        sets,
    })
}

fn byte(data: &[u8], ofs: usize) -> io::Result<u8> {
    data.get(ofs)
        .copied()
        .ok_or_else(|| invalid_data("SE bank read outside its section"))
}

fn u16_at(data: &[u8], ofs: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes([byte(data, ofs)?, byte(data, ofs + 1)?]))
}

fn u32_at(data: &[u8], ofs: usize) -> io::Result<u32> {
    data.get(ofs..ofs + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid_data("SE bank read outside its section"))
}

#[cfg(test)]
mod tests {
    use super::SeBank;

    fn chunk(tag: &[u8; 4], size: u32) -> Vec<u8> {
        let mut c = b"IECS".to_vec();
        c.extend_from_slice(tag);
        c.extend_from_slice(&size.to_le_bytes());
        c
    }

    fn words(out: &mut Vec<u8>, values: &[u32]) {
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    /// The common bank's layout (file 0x14B) cut down to one sample, one tone and one sequence.
    fn bank() -> Vec<u8> {
        let mut hd = chunk(b"sreV", 0x10);
        words(&mut hd, &[0x0002_0000]);
        hd.extend(chunk(b"daeH", 0x40));
        words(
            &mut hd,
            &[0, 0x10, u32::MAX, u32::MAX, u32::MAX, 0x50, 0x70],
        );
        hd.resize(0x50, 0xFF);
        hd.extend(chunk(b"igaV", 0x20));
        words(&mut hd, &[0, 0x14, 0]);
        hd.extend_from_slice(&[0x80, 0x3E, 0, 0xFF]);
        hd.resize(0x70, 0xFF);
        hd.extend(chunk(b"bteS", 0x60));
        words(&mut hd, &[0, 0x14, 0, 0x1C, 8]);
        hd.extend_from_slice(&[0x32, 0, 0, 0xFF]);
        let mut tone = vec![0u8; 50];
        tone[3] = 0x64;
        tone[4] = 0x40;
        tone[9] = 0x0A;
        tone[12] = 1;
        tone[13] = 3;
        tone[14..16].copy_from_slice(&0x80FFu16.to_le_bytes());
        tone[16..18].copy_from_slice(&0x5FC0u16.to_le_bytes());
        hd.extend(tone);

        let mut sq = chunk(b"sreV", 0x10);
        words(&mut sq, &[0x0002_0000]);
        sq.extend(chunk(b"uqeS", 0x20));
        words(&mut sq, &[0x60, u32::MAX, u32::MAX, 0x30, u32::MAX]);
        sq.extend(chunk(b"qseS", 0x30));
        words(&mut sq, &[0, 0x10]);
        sq.extend_from_slice(&[0x7F, 0x40, 0xE8, 0x03]);
        words(&mut sq, &[u32::MAX, u32::MAX, 0x24, 0, 0xC]);
        sq.extend_from_slice(&[0x64, 0x40, 0xE8, 0x03]);
        words(&mut sq, &[u32::MAX, 0x38, 0x10]);
        sq.extend_from_slice(&[0x64, 0x40, 0xE8, 0x03]);
        words(&mut sq, &[7, 0]);
        sq.extend_from_slice(&[0, 0x90, 0, 0, 0x64, 0, 0xFF]);

        let body = vec![0u8; 16];
        let mut file = Vec::new();
        let hd_ofs = 0x20 + body.len() as u32;
        let sq_ofs = hd_ofs + hd.len() as u32;
        words(
            &mut file,
            &[3, 0x20, hd_ofs, sq_ofs, sq_ofs + sq.len() as u32],
        );
        file.resize(0x20, 0);
        file.extend(body);
        file.extend(hd);
        file.extend(sq);
        file
    }

    #[test]
    fn reads_samples_tones_and_sequences() {
        let file = bank();
        let bank = SeBank::parse(&file).unwrap();
        assert_eq!(bank.body.len(), 16);
        assert_eq!(bank.vags.len(), 1);
        assert_eq!(bank.vags[0].sample_rate, 16000);
        let tone = bank.tone(0, 0, 0).unwrap();
        assert_eq!((tone.volume, tone.pan, tone.priority), (0x64, 0x40, 0x0A));
        assert_eq!((tone.adsr1, tone.adsr2, tone.mix), (0x80FF, 0x5FC0, 3));
        assert!(bank.tone(0, 0, 1).is_none());
        assert_eq!(
            (bank.sesq.volume, bank.sesq.pan, bank.sesq.rate),
            (0x7F, 0x40, 1000)
        );
        let set = bank.sesq.sets[0].as_ref().unwrap();
        assert_eq!(set.volume, 0x64);
        let seq = set.sequences[0].as_ref().unwrap();
        assert_eq!(&seq.data, &[0, 0x90, 0, 0, 0x64, 0, 0xFF]);
    }
}
