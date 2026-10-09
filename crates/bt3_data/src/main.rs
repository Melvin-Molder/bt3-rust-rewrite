use std::path::PathBuf;

use bt3_data::Iso9660;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::gs::decode_sprite_texture;
use bt3_data::stage::StageResource;
use bt3_data::tex::TextureResource;

#[derive(Clone, Debug)]
struct TextureCandidate {
    file_id: u32,
    partition: u8,
    index: u32,
    packed_size: usize,
    decoded_size: usize,
    texture_count: usize,
    sample_dimensions: String,
    alpha_pixels: usize,
}

fn file_id_for_partition_entry(partition: u8, index: u32) -> u32 {
    match partition {
        0 => index,
        1 => index + 1,
        2 => 0xD48 + index,
        _ => panic!("unsupported AFS partition {partition}"),
    }
}

fn print_afs_inventory(image: &mut Iso9660) -> Result<(), Box<dyn std::error::Error>> {
    for partition in 0..3u8 {
        let archive_path = format!("DATA/PZS3US{partition}.AFS");
        let Ok(archive) = AfsArchive::open(image, &archive_path) else {
            continue;
        };

        let mut members = Vec::new();
        for index in 0..archive.entry_count() {
            let entry = archive.entry(image, index)?;
            members.push((
                index,
                entry.size,
                file_id_for_partition_entry(partition, index),
            ));
        }
        members.sort_by(|left, right| right.1.cmp(&left.1));

        println!(
            "AFS partition {partition}: {} entries, largest=0x{:X} bytes",
            archive.entry_count(),
            members.first().map_or(0, |(_, size, _)| *size)
        );
        for (index, size, global_id) in members.iter().take(8) {
            println!(
                "  entry {index} -> global file ID 0x{global_id:X} ({} bytes)",
                size
            );
        }
    }

    Ok(())
}

fn scan_texture_candidates(
    image: &mut Iso9660,
) -> Result<Vec<TextureCandidate>, Box<dyn std::error::Error>> {
    let mut candidates = Vec::new();

    for partition in [1u8, 2u8] {
        let archive_path = format!("DATA/PZS3US{partition}.AFS");
        let archive = AfsArchive::open(image, &archive_path)?;

        let mut most_likely = Vec::new();
        for index in 0..archive.entry_count() {
            let entry = archive.entry(image, index)?;
            most_likely.push((index, entry.size));
        }
        most_likely.sort_by(|left, right| right.1.cmp(&left.1));

        let mut sample_indices = most_likely
            .iter()
            .take(48)
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        if partition == 1 {
            sample_indices.extend((0..archive.entry_count().min(300)).collect::<Vec<_>>());
        }
        sample_indices.sort();
        sample_indices.dedup();

        for index in sample_indices {
            let file_id = file_id_for_partition_entry(partition, index);
            if matches!(file_id, 0x3C1 | 0x3C2 | 0x3C3) {
                continue;
            }
            let packed = match archive.read_entry(image, index) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let decoded = match bpe::decode(&packed) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let resource = match TextureResource::parse(&decoded) {
                Ok(res) => res,
                Err(_) => continue,
            };
            if resource.entries.is_empty() {
                continue;
            }

            let mut texture_count = 0usize;
            let mut alpha_pixels = 0usize;
            let mut sample_dimensions = String::new();
            for texture in &resource.entries {
                let Ok(rgba) = decode_sprite_texture(texture) else {
                    continue;
                };
                texture_count += 1;
                alpha_pixels += rgba
                    .pixels
                    .chunks_exact(4)
                    .filter(|pixel| pixel[3] != 0)
                    .count();
                if sample_dimensions.is_empty() {
                    sample_dimensions = format!("{}x{}", rgba.width, rgba.height);
                }
            }
            if texture_count == 0 {
                continue;
            }

            candidates.push(TextureCandidate {
                file_id,
                partition,
                index,
                packed_size: packed.len(),
                decoded_size: decoded.len(),
                texture_count,
                sample_dimensions,
                alpha_pixels,
            });
        }
    }

    candidates.sort_by(|left, right| {
        right
            .texture_count
            .cmp(&left.texture_count)
            .then(right.packed_size.cmp(&left.packed_size))
            .then(right.file_id.cmp(&left.file_id))
    });
    Ok(candidates)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso")
        });

    let mut image = Iso9660::open(&image_path)?;
    let system_config = image.read_file("SYSTEM.CNF")?;
    println!("Image: {}", image_path.display());
    println!("Volume: {}", image.volume_id());
    println!("Volume blocks: {}", image.volume_blocks());
    println!("SYSTEM.CNF:");
    print!("{}", String::from_utf8_lossy(&system_config));
    println!();
    print_afs_inventory(&mut image)?;

    let stage_id = 0;
    let stage_file_id = 0x171 + stage_id;
    let stage_location = partition_entry_for_id(stage_file_id);
    let stage_archive = AfsArchive::open(
        &mut image,
        format!("DATA/PZS3US{}.AFS", stage_location.partition),
    )?;
    let stage_bytes = stage_archive.read_entry(&mut image, stage_location.index)?;
    let stage = StageResource::parse(&stage_bytes)?;
    println!(
        "\nStage {stage_id}, file ID 0x{stage_file_id:X}: {} bytes, {} objects, {} zones, bounds radius/top/bottom = {:.2}/{:.2}/{:.2}",
        stage_bytes.len(),
        stage.object_count,
        stage.zone_count,
        stage.bounds.radius,
        stage.bounds.top,
        stage.bounds.bottom
    );
    println!(
        "  stage header reports ID {}, flags 0x{:X}, water level {:?}",
        stage.stage_id, stage.flags, stage.water_level
    );
    for (index, placement) in stage.starting_placements.iter().enumerate() {
        println!(
            "  start {index}: position {:?}, target {:?}",
            placement.position, placement.target
        );
    }
    for (index, placement) in stage.alternate_starting_placements.iter().enumerate() {
        println!(
            "  alternate start {index}: position {:?}, target {:?}",
            placement.position, placement.target
        );
    }

    println!("\nTexture-resource candidates from real AFS data:");
    let candidates = scan_texture_candidates(&mut image)?;
    for candidate in candidates.iter().take(12) {
        println!(
            "  file 0x{:X} (partition {}, entry {}): {} packed -> {} decoded bytes, {} textures, sample {}, {} alpha pixels",
            candidate.file_id,
            candidate.partition,
            candidate.index,
            candidate.packed_size,
            candidate.decoded_size,
            candidate.texture_count,
            candidate.sample_dimensions,
            candidate.alpha_pixels
        );
    }
    if candidates.is_empty() {
        println!(
            "  No additional valid texture resources were identified in the sampled AFS members."
        );
    }

    for screen_type in 0..3u32 {
        let file_id = 0x3C1 + screen_type;
        let location = partition_entry_for_id(file_id);
        let archive_path = format!("DATA/PZS3US{}.AFS", location.partition);
        let archive = AfsArchive::open(&mut image, archive_path)?;
        let entry = archive.entry(&mut image, location.index)?;
        let packed = archive.read_entry(&mut image, location.index)?;
        let decoded = bpe::decode(&packed)?;
        let resource = TextureResource::parse(&decoded)?;

        println!(
            "Loading-screen type {screen_type}, file ID 0x{file_id:X}: partition {}, entry {} of {} ({} packed -> {} decoded bytes; {} RGBA textures)",
            location.partition,
            location.index,
            archive.entry_count(),
            entry.size,
            decoded.len(),
            resource.entries.len()
        );
        for (texture_index, texture) in resource.entries.iter().enumerate() {
            let rgba = decode_sprite_texture(texture)?;
            println!(
                "  texture {texture_index}: {}x{} RGBA ({} pixels with alpha)",
                rgba.width,
                rgba.height,
                rgba.pixels
                    .chunks_exact(4)
                    .filter(|pixel| pixel[3] != 0)
                    .count()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::file_id_for_partition_entry;

    #[test]
    fn keeps_global_file_ids_in_sync_with_afs_partition_layout() {
        assert_eq!(file_id_for_partition_entry(0, 0), 0);
        assert_eq!(file_id_for_partition_entry(1, 0), 1);
        assert_eq!(file_id_for_partition_entry(1, 0xD46), 0xD47);
        assert_eq!(file_id_for_partition_entry(2, 0), 0xD48);
    }
}
