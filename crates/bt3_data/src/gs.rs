use std::io;

use crate::tex::TextureEntry;

const GS_PAGE_BYTES: usize = 8192;
const PSMCT32_PAGE_WIDTH: usize = 64;
const PSMCT32_PAGE_HEIGHT: usize = 32;
const PSMT4_PAGE_WIDTH: usize = 128;
const PSMT4_PAGE_HEIGHT: usize = 128;
const PSMT8_PAGE_WIDTH: usize = 128;
const PSMT8_PAGE_HEIGHT: usize = 64;
const PSM_T8: u8 = 0x13;
const PSM_T4: u8 = 0x14;

pub struct RgbaTexture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode_sprite_texture(entry: &TextureEntry<'_>) -> io::Result<RgbaTexture> {
    let pixel_packet = entry
        .pixel_data
        .ok_or_else(|| invalid_data("sprite texture has no pixel packet"))?;
    let clut_packet = entry
        .clut_data
        .ok_or_else(|| invalid_data("sprite texture has no CLUT packet"))?;

    let texture_psm = ((entry.texture_register >> 20) & 0x3f) as u8;
    let texture_alpha = ((entry.texture_register >> 34) & 1) as u8;
    let clut_psm = ((entry.texture_register >> 51) & 0xf) as u8;
    let clut_mode = ((entry.texture_register >> 55) & 1) as u8;
    let clut_offset = ((entry.texture_register >> 56) & 0x1f) as u8;
    if !matches!(texture_psm, PSM_T4 | PSM_T8)
        || texture_alpha != 0
        || clut_psm != 0
        || clut_mode != 0
        || clut_offset != 0
    {
        return Err(invalid_data("unsupported sprite texture or CLUT mode"));
    }

    let width = 1usize << ((entry.texture_register >> 26) & 0xf);
    let height = 1usize << ((entry.texture_register >> 30) & 0xf);
    let target_buffer_width = ((entry.texture_register >> 14) & 0x3f) as usize;
    if target_buffer_width < 2 || target_buffer_width % 2 != 0 || width > target_buffer_width * 64 {
        return Err(invalid_data(
            "unsupported indexed texture dimensions or stride",
        ));
    }

    let pixel_transfer = find_image_transfer(pixel_packet)?;
    let pixel_dbw = (entry.pixel_blt & 0x3f) as usize;
    let pixel_psm = ((entry.pixel_blt >> 8) & 0x3f) as u8;
    if pixel_psm != 0 || pixel_dbw == 0 || pixel_transfer.direction != 0 {
        return Err(invalid_data(
            "sprite pixels are not a GIF-to-PSMCT32 transfer",
        ));
    }
    if pixel_transfer.source_x != 0
        || pixel_transfer.source_y != 0
        || pixel_transfer.destination_x != 0
        || pixel_transfer.destination_y != 0
    {
        return Err(invalid_data(
            "offset PSMCT32 sprite transfers are unsupported",
        ));
    }
    if pixel_transfer.width == 0
        || pixel_transfer.width > pixel_dbw * PSMCT32_PAGE_WIDTH
        || pixel_transfer.image.len() != pixel_transfer.width * pixel_transfer.height * 4
    {
        return Err(invalid_data("unexpected PSMCT32 sprite transfer geometry"));
    }

    let (clut_width, clut_height, colors) = if texture_psm == PSM_T4 {
        (8, 2, 16)
    } else {
        (16, 16, 256)
    };
    let clut_transfer = find_image_transfer(clut_packet)?;
    if clut_transfer.direction != 0
        || clut_transfer.source_x != 0
        || clut_transfer.source_y != 0
        || clut_transfer.destination_x != 0
        || clut_transfer.destination_y != 0
        || clut_transfer.width != clut_width
        || clut_transfer.height != clut_height
        || clut_transfer.image.len() != colors * 4
    {
        return Err(invalid_data("unexpected PSMCT32 sprite CLUT transfer"));
    }

    let source_pages_wide = pixel_dbw;
    let source_pages_high = pixel_transfer.height.div_ceil(PSMCT32_PAGE_HEIGHT);
    let mut local_memory = vec![0; source_pages_wide * source_pages_high * GS_PAGE_BYTES];
    for y in 0..pixel_transfer.height {
        for x in 0..pixel_transfer.width {
            let page = (y / PSMCT32_PAGE_HEIGHT) * source_pages_wide + x / PSMCT32_PAGE_WIDTH;
            let local_x = x % PSMCT32_PAGE_WIDTH;
            let local_y = y % PSMCT32_PAGE_HEIGHT;
            let destination = page * GS_PAGE_BYTES + psmct32_byte_offset(local_x, local_y);
            let source = (y * pixel_transfer.width + x) * 4;
            local_memory[destination..destination + 4]
                .copy_from_slice(&pixel_transfer.image[source..source + 4]);
        }
    }

    let target_pages_wide = target_buffer_width / 2;
    if target_pages_wide != source_pages_wide {
        return Err(invalid_data("texture and upload page widths differ"));
    }
    let output_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| invalid_data("decoded texture size overflow"))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(output_len)
        .map_err(|_| io::Error::other("could not allocate RGBA texture"))?;
    pixels.resize(output_len, 0);

    for y in 0..height {
        for x in 0..width {
            let palette_index = if texture_psm == PSM_T4 {
                let (byte, shift) = psmt4_location(x, y, target_pages_wide);
                usize::from((*local_memory.get(byte).ok_or_else(texel_outside)? >> shift) & 0x0f)
            } else {
                let index = *local_memory
                    .get(psmt8_location(x, y, target_pages_wide))
                    .ok_or_else(texel_outside)?;
                csm1_t32_i8_transfer_index(index)
            };
            let rgba = &clut_transfer.image[palette_index * 4..palette_index * 4 + 4];
            let output = (y * width + x) * 4;
            pixels[output] = rgba[0];
            pixels[output + 1] = rgba[1];
            pixels[output + 2] = rgba[2];
            pixels[output + 3] = (u16::from(rgba[3]) * 2).min(255) as u8;
        }
    }

    Ok(RgbaTexture {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

fn texel_outside() -> io::Error {
    invalid_data("texel address exceeds uploaded GS memory")
}

struct ImageTransfer<'a> {
    image: &'a [u8],
    width: usize,
    height: usize,
    source_x: usize,
    source_y: usize,
    destination_x: usize,
    destination_y: usize,
    direction: u8,
}

#[derive(Default)]
struct TransferRegisters {
    position: Option<u64>,
    region: Option<u64>,
    direction: Option<u64>,
}

fn find_image_transfer(packet: &[u8]) -> io::Result<ImageTransfer<'_>> {
    let mut offset = 0usize;
    let mut registers = TransferRegisters::default();

    while offset
        .checked_add(16)
        .is_some_and(|end| end <= packet.len())
    {
        let tag = read_u64(packet, offset)?;
        let loop_count = usize::from((tag & 0x7fff) as u16);
        let format = ((tag >> 58) & 0x3) as u8;
        let register_count = match ((tag >> 60) & 0xf) as usize {
            0 => 16,
            count => count,
        };
        let descriptor = read_u64(packet, offset + 8)?;
        let body = offset + 16;

        if format == 2 || format == 3 {
            let byte_count = loop_count
                .checked_mul(16)
                .ok_or_else(|| invalid_data("GIF image transfer size overflow"))?;
            let end = body
                .checked_add(byte_count)
                .filter(|end| *end <= packet.len())
                .ok_or_else(|| invalid_data("GIF image transfer exceeds packet"))?;
            let position = registers
                .position
                .ok_or_else(|| invalid_data("GIF image transfer lacks TRXPOS"))?;
            let region = registers
                .region
                .ok_or_else(|| invalid_data("GIF image transfer lacks TRXREG"))?;
            let direction = registers
                .direction
                .ok_or_else(|| invalid_data("GIF image transfer lacks TRXDIR"))?;
            return Ok(ImageTransfer {
                image: &packet[body..end],
                width: (region & 0xfff) as usize,
                height: ((region >> 32) & 0xfff) as usize,
                source_x: (position & 0x7ff) as usize,
                source_y: ((position >> 16) & 0x7ff) as usize,
                destination_x: ((position >> 32) & 0x7ff) as usize,
                destination_y: ((position >> 48) & 0x7ff) as usize,
                direction: (direction & 0x3) as u8,
            });
        }

        let body_size = match format {
            0 => loop_count
                .checked_mul(register_count)
                .and_then(|words| words.checked_mul(16)),
            1 => loop_count
                .checked_mul(register_count)
                .and_then(|words| words.checked_mul(8))
                .map(|size| (size + 15) & !15),
            _ => return Err(invalid_data("unsupported GIF transfer format")),
        }
        .ok_or_else(|| invalid_data("GIF register transfer size overflow"))?;
        let end = body
            .checked_add(body_size)
            .filter(|end| *end <= packet.len())
            .ok_or_else(|| invalid_data("GIF register transfer exceeds packet"))?;

        if format == 0 {
            for loop_index in 0..loop_count {
                for register_index in 0..register_count {
                    let slot = body + (loop_index * register_count + register_index) * 16;
                    let value = read_u64(packet, slot)?;
                    let register = (descriptor >> (register_index * 4)) & 0xf;
                    if register == 0xe {
                        let address = read_u64(packet, slot + 8)? & 0xff;
                        match address {
                            0x51 => registers.position = Some(value),
                            0x52 => registers.region = Some(value),
                            0x53 => registers.direction = Some(value),
                            _ => {}
                        }
                    }
                }
            }
        }

        offset = end;
    }

    Err(invalid_data("GIF packet contains no image transfer"))
}

fn psmct32_byte_offset(x: usize, y: usize) -> usize {
    let block_x = x >> 3;
    let block_y = y >> 3;
    let block = (block_x & 1)
        | ((block_y & 1) << 1)
        | ((block_x & 2) << 1)
        | ((block_y & 2) << 2)
        | ((block_x & 4) << 2);
    let pixel_x = x & 7;
    let pixel_y = y & 7;
    let pixel = (pixel_x & 1)
        | ((pixel_y & 1) << 1)
        | ((pixel_x & 2) << 1)
        | ((pixel_y & 2) << 3)
        | ((pixel_x & 4) << 1)
        | ((pixel_y & 4) << 3);
    block * 256 + pixel * 4
}

fn psmt4_location(x: usize, y: usize, pages_wide: usize) -> (usize, u8) {
    const COLUMN_EVEN: [u8; 16] = [0, 1, 4, 5, 8, 9, 12, 13, 2, 3, 6, 7, 10, 11, 14, 15];
    const COLUMN_ODD: [u8; 16] = [8, 9, 12, 13, 0, 1, 4, 5, 10, 11, 14, 15, 2, 3, 6, 7];

    let page = (y / PSMT4_PAGE_HEIGHT) * pages_wide + x / PSMT4_PAGE_WIDTH;
    let local_x = x & (PSMT4_PAGE_WIDTH - 1);
    let local_y = y & (PSMT4_PAGE_HEIGHT - 1);
    let block_x = local_x >> 5;
    let block_y = local_y >> 4;
    let block = (block_y & 1)
        | ((block_x & 1) << 1)
        | ((block_y & 2) << 1)
        | ((block_x & 2) << 2)
        | ((block_y & 4) << 2);
    let column = (local_y & 0xf) >> 2;
    let column_y = local_y & 3;
    let pattern = if ((column_y >> 1) ^ (column & 1)) != 0 {
        &COLUMN_ODD
    } else {
        &COLUMN_EVEN
    };
    let column_word = usize::from(pattern[(column_y & 1) * 8 + (local_x & 7)]);
    let nibble = (column_y >> 1) + (((local_x & 0x1f) >> 3) * 2);
    let byte = page * GS_PAGE_BYTES + block * 256 + column * 64 + column_word * 4 + nibble / 2;
    (byte, ((nibble & 1) * 4) as u8)
}

/// Byte address of a PSMT8 texel: its 16x16 block is one PSMCT32 8x8 block, four texels per word.
fn psmt8_location(x: usize, y: usize, pages_wide: usize) -> usize {
    let page = (y / PSMT8_PAGE_HEIGHT) * pages_wide + x / PSMT8_PAGE_WIDTH;
    let local_x = x % PSMT8_PAGE_WIDTH;
    let local_y = y % PSMT8_PAGE_HEIGHT;
    let swap = ((local_y + 2) >> 2 & 1) * 4;
    let word_y = (((local_y & !3) >> 1) + (local_y & 1)) & 7;
    let word_x = (local_x + swap) & 7;
    let byte = ((local_y >> 1) & 1) + ((local_x >> 2) & 2);
    let ct32_x = (local_x >> 4) * 8 + word_x;
    let ct32_y = (local_y >> 4) * 8 + word_y;
    page * GS_PAGE_BYTES + psmct32_byte_offset(ct32_x, ct32_y) + byte
}

/// Uploading a 16x16 PSMCT32 CLUT and reading it with CSM1 swaps index bits 3 and 4.
fn csm1_t32_i8_transfer_index(index: u8) -> usize {
    usize::from((index & 0xE7) | ((index & 0x08) << 1) | ((index & 0x10) >> 1))
}

#[cfg(test)]
fn csm1_t32_i4_index(index: u8) -> usize {
    usize::from((index & 1) | ((index & 2) << 1) | ((index & 4) << 1) | ((index & 8) >> 2))
}

fn read_u64(bytes: &[u8], offset: usize) -> io::Result<u64> {
    let word = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| invalid_data("GIF field exceeds transfer packet"))?;
    Ok(u64::from_le_bytes(word.try_into().unwrap()))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::{
        GS_PAGE_BYTES, PSMCT32_PAGE_HEIGHT, PSMCT32_PAGE_WIDTH, RgbaTexture, csm1_t32_i4_index,
        csm1_t32_i8_transfer_index, decode_sprite_texture, psmct32_byte_offset, psmt4_location,
        psmt8_location,
    };
    use crate::tex::TextureEntry;

    #[test]
    fn psmt8_addressing_matches_the_gs_column_table() {
        let first_rows = [
            (1, 0, 4),
            (2, 0, 16),
            (4, 0, 32),
            (8, 0, 2),
            (0, 1, 8),
            (0, 2, 33),
            (4, 2, 1),
            (0, 4, 96),
        ];
        for (x, y, byte) in first_rows {
            assert_eq!(psmt8_location(x, y, 1), byte, "texel ({x}, {y})");
        }
        let mut addresses: Vec<usize> = (0..64)
            .flat_map(|y| (0..128).map(move |x| psmt8_location(x, y, 1)))
            .collect();
        addresses.sort_unstable();
        addresses.dedup();
        assert_eq!(addresses.len(), GS_PAGE_BYTES);
    }

    #[test]
    fn csm1_256_colour_lookup_swaps_index_bits_three_and_four() {
        assert_eq!(csm1_t32_i8_transfer_index(0x07), 0x07);
        assert_eq!(csm1_t32_i8_transfer_index(0x08), 0x10);
        assert_eq!(csm1_t32_i8_transfer_index(0x10), 0x08);
        assert_eq!(csm1_t32_i8_transfer_index(0xF8), 0xF8);
    }

    #[test]
    fn psmct32_addressing_covers_one_page() {
        assert_eq!(psmct32_byte_offset(0, 0), 0);
        assert_eq!(psmct32_byte_offset(8, 0), 256);
        assert_eq!(psmct32_byte_offset(0, 8), 512);
        assert_eq!(psmct32_byte_offset(1, 0), 4);
        assert_eq!(psmct32_byte_offset(2, 0), 16);
        assert_eq!(psmct32_byte_offset(4, 0), 32);
        assert_eq!(psmct32_byte_offset(0, 1), 8);
        assert_eq!(psmct32_byte_offset(0, 2), 64);

        let mut addresses = Vec::new();
        for y in 0..PSMCT32_PAGE_HEIGHT {
            for x in 0..PSMCT32_PAGE_WIDTH {
                addresses.push(psmct32_byte_offset(x, y));
            }
        }
        addresses.sort_unstable();
        addresses.dedup();
        assert_eq!(addresses.len(), GS_PAGE_BYTES / 4);
    }

    #[test]
    fn psmt4_addressing_covers_every_nibble_in_one_page() {
        let mut addresses = Vec::new();
        for y in 0..128 {
            for x in 0..128 {
                let (byte, shift) = psmt4_location(x, y, 1);
                addresses.push(byte * 2 + usize::from(shift / 4));
            }
        }
        addresses.sort_unstable();
        addresses.dedup();
        assert_eq!(addresses.len(), 128 * 128);
    }

    #[test]
    fn csm1_palette_order_swaps_the_middle_index_bits() {
        let indexes = (0..16).map(csm1_t32_i4_index).collect::<Vec<_>>();
        assert_eq!(
            indexes,
            [0, 1, 4, 5, 8, 9, 12, 13, 2, 3, 6, 7, 10, 11, 14, 15]
        );
    }

    #[test]
    fn palette_upload_and_csm1_lookup_preserve_transfer_order() {
        for index in 0u8..16 {
            let upload_x = usize::from(index % 8);
            let upload_y = usize::from(index / 8);
            assert_eq!(
                psmct32_byte_offset(upload_x, upload_y),
                csm1_t32_i4_index(index) * 4
            );
        }
    }

    #[test]
    fn decodes_one_gs_page_and_its_csm1_palette() {
        let pixel_image = vec![0x21; GS_PAGE_BYTES];
        let mut palette_image = vec![0; 16 * 4];
        for index in 0..16 {
            let color_index = index as u8;
            palette_image[index * 4..index * 4 + 4].copy_from_slice(&[
                color_index * 10,
                30,
                200 - color_index * 5,
                if index == 2 { 0 } else { 128 },
            ]);
        }
        let pixel_packet = image_packet(64, 32, &pixel_image);
        let clut_packet = image_packet(8, 2, &palette_image);
        let texture = TextureEntry {
            pixel_data: Some(&pixel_packet),
            clut_data: Some(&clut_packet),
            pixel_size: pixel_packet.len() as i32,
            clut_size: clut_packet.len() as i32,
            pixel_blt: 1,
            clut_blt: 0,
            texture_register: (2 << 14) | (0x14 << 20) | (7 << 26) | (7 << 30),
        };

        let decoded = decode_sprite_texture(&texture).unwrap();

        assert_eq!(decoded.width, 128);
        assert_eq!(decoded.height, 128);
        assert_eq!(pixel_at(&decoded, 0, 0), [10, 30, 195, 255]);
        assert_eq!(pixel_at(&decoded, 0, 2), [20, 30, 190, 0]);
    }

    fn image_packet(width: usize, height: usize, image: &[u8]) -> Vec<u8> {
        let mut packet = vec![0; 16 + 16 + 3 * 16 + 16];
        write_u64(&mut packet, 16, 3 | (1 << 15) | (1 << 60));
        write_u64(&mut packet, 24, 0xe);
        write_ad(&mut packet, 32, 0, 0x51);
        write_ad(&mut packet, 48, (height as u64) << 32 | width as u64, 0x52);
        write_ad(&mut packet, 64, 0, 0x53);

        let image_loops = image.len() / 16;
        write_u64(&mut packet, 80, image_loops as u64 | (1 << 15) | (2 << 58));
        packet.extend_from_slice(image);
        packet.extend_from_slice(&[0; 32]);
        packet
    }

    fn write_ad(packet: &mut [u8], offset: usize, value: u64, address: u64) {
        write_u64(packet, offset, value);
        write_u64(packet, offset + 8, address);
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn pixel_at(texture: &RgbaTexture, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * texture.width as usize + x) * 4;
        texture.pixels[offset..offset + 4].try_into().unwrap()
    }
}
