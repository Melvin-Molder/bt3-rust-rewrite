use std::io;

use crate::invalid_data;

/// An ADX stream header as `ADXB_DecodeHeaderAdx` (0x287480) reads it; all fields are big-endian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdxHeader {
    pub data_offset: usize,
    pub encoding: u8,
    pub block_size: usize,
    pub bits: u8,
    pub channels: usize,
    pub sample_rate: u32,
    pub total_samples: u32,
    pub cutoff: u16,
    pub version: u8,
    pub flags: u8,
}

impl AdxHeader {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 0x14 || be16(bytes, 0) != 0x8000 {
            return Err(invalid_data("missing ADX header"));
        }
        let header = Self {
            data_offset: usize::from(be16(bytes, 2)) + 4,
            encoding: bytes[4],
            block_size: usize::from(bytes[5]),
            bits: bytes[6],
            channels: usize::from(bytes[7]),
            sample_rate: be32(bytes, 8),
            total_samples: be32(bytes, 0xC),
            cutoff: be16(bytes, 0x10),
            version: bytes[0x12],
            flags: bytes[0x13],
        };
        if header.encoding != 3
            || header.bits != 4
            || header.block_size != 18
            || header.channels == 0
            || header.sample_rate == 0
            || header.data_offset > bytes.len()
        {
            return Err(invalid_data("unsupported ADX stream layout"));
        }
        Ok(header)
    }

    /// `(block_size * 8 - 16) / bits`, as func_00289588 computes it.
    pub fn samples_per_block(&self) -> usize {
        (self.block_size * 8 - 16) / usize::from(self.bits)
    }
}

/// func_00289300: the two prediction coefficients, in 1/4096 units, computed in single precision.
pub fn prediction_coefficients(cutoff: u16, sample_rate: u32) -> (i16, i16) {
    let two_pi = f32::from_bits(0x40C9_0FDA);
    let sqrt2 = 2.0f32.sqrt();
    let a = sqrt2 - (f32::from(cutoff) * two_pi / sample_rate as f32).cos();
    let b = sqrt2 - 1.0;
    let c = (a - ((a + b) * (a - b)).sqrt()) / b;
    // cvt.w.s truncates toward zero, as Rust's `as` does.
    (((c + c) * 4096.0) as i16, (-c * c * 4096.0) as i16)
}

/// Decodes a whole ADX stream to interleaved 16-bit PCM, block by block as func_0028E2B0 does.
/// The executable's default ADX key (D_002E484C) is zero, so the key schedule never changes a scale.
pub fn decode(bytes: &[u8]) -> io::Result<(AdxHeader, Vec<i16>)> {
    let header = AdxHeader::parse(bytes)?;
    let (coef1, coef2) = prediction_coefficients(header.cutoff, header.sample_rate);
    let (coef1, coef2) = (i32::from(coef1), i32::from(coef2));
    let per_block = header.samples_per_block();
    let frame_size = header.block_size * header.channels;
    let total = header.total_samples as usize;

    let mut pcm = Vec::new();
    pcm.try_reserve_exact(total * header.channels)
        .map_err(|_| io::Error::other("could not allocate ADX output"))?;
    let mut history = vec![(0i32, 0i32); header.channels];
    let mut frames = 0usize;

    'stream: for frame in bytes[header.data_offset..].chunks_exact(frame_size) {
        let mut block_samples = vec![[0i16; 32]; header.channels];
        for (channel, block) in frame.chunks_exact(header.block_size).enumerate() {
            let block_header = be16(block, 0);
            if block_header & 0x8000 != 0 {
                break 'stream;
            }
            let scale = i32::from(block_header & 0x1FFF) + 1;
            let (hist1, hist2) = &mut history[channel];
            for (index, sample) in block_samples[channel]
                .iter_mut()
                .enumerate()
                .take(per_block)
            {
                let byte = block[2 + index / 2];
                let nibble = if index % 2 == 0 {
                    byte as i8 >> 4
                } else {
                    (byte << 4) as i8 >> 4
                };
                let prediction = (coef1 * *hist1 + coef2 * *hist2) >> 12;
                let value = (prediction + i32::from(nibble) * scale).clamp(-0x8000, 0x7FFF);
                *hist2 = *hist1;
                *hist1 = value;
                *sample = value as i16;
            }
        }
        for index in 0..per_block {
            if frames * per_block + index >= total {
                break 'stream;
            }
            pcm.extend(block_samples.iter().map(|block| block[index]));
        }
        frames += 1;
    }
    Ok((header, pcm))
}

/// Wraps interleaved 16-bit PCM in a minimal RIFF/WAVE container.
pub fn to_wav(sample_rate: u32, channels: u16, pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    pcm.iter()
        .for_each(|sample| out.extend_from_slice(&sample.to_le_bytes()));
    out
}

fn be16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::{AdxHeader, decode, prediction_coefficients};

    fn stream(channels: u8, total: u32, blocks: &[[u8; 18]]) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x24];
        bytes[0..2].copy_from_slice(&0x8000u16.to_be_bytes());
        bytes[2..4].copy_from_slice(&0x20u16.to_be_bytes());
        bytes[4] = 3;
        bytes[5] = 18;
        bytes[6] = 4;
        bytes[7] = channels;
        bytes[8..12].copy_from_slice(&44100u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&total.to_be_bytes());
        bytes[16..18].copy_from_slice(&500u16.to_be_bytes());
        bytes[18] = 4;
        for block in blocks {
            bytes.extend_from_slice(block);
        }
        bytes
    }

    #[test]
    fn coefficients_follow_the_single_precision_formula() {
        let (c1, c2) = prediction_coefficients(500, 44100);
        let a = 2f64.sqrt() - (2.0 * std::f64::consts::PI * 500.0 / 44100.0).cos();
        let b = 2f64.sqrt() - 1.0;
        let c = (a - ((a + b) * (a - b)).sqrt()) / b;
        assert!((f64::from(c1) - c * 2.0 * 4096.0).abs() < 2.0);
        assert!((f64::from(c2) + c * c * 4096.0).abs() < 2.0);
    }

    #[test]
    fn scale_is_the_masked_header_plus_one_and_high_nibble_comes_first() {
        let mut block = [0u8; 18];
        block[0..2].copy_from_slice(&0x0009u16.to_be_bytes());
        block[2] = 0x1F;
        let (header, pcm) = decode(&stream(1, 2, &[block])).unwrap();
        assert_eq!(header.samples_per_block(), 32);
        let (c1, _) = prediction_coefficients(500, 44100);
        assert_eq!(pcm[0], 10);
        assert_eq!(pcm[1], ((i32::from(c1) * 10) >> 12) as i16 - 10);
    }

    #[test]
    fn samples_are_clamped_and_interleaved() {
        let mut left = [0u8; 18];
        left[0..2].copy_from_slice(&0x1FFFu16.to_be_bytes());
        left[2] = 0x77;
        let mut right = [0u8; 18];
        right[0..2].copy_from_slice(&0x1FFFu16.to_be_bytes());
        right[2] = 0x88;
        let (_, pcm) = decode(&stream(2, 1, &[left, right])).unwrap();
        assert_eq!(pcm, [32767, -32768]);
    }

    #[test]
    fn an_end_block_stops_the_stream() {
        let mut end = [0u8; 18];
        end[0] = 0x80;
        let (_, pcm) = decode(&stream(1, 64, &[[0; 18], end])).unwrap();
        assert_eq!(pcm.len(), 32);
    }

    #[test]
    fn rejects_non_adx_data() {
        assert!(AdxHeader::parse(&[0; 0x20]).is_err());
    }
}
