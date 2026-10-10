use std::path::PathBuf;

use bt3_data::Iso9660;
use bt3_data::adx;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::gs::decode_sprite_texture;
use bt3_data::pack_section;
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

fn probe_title(image: &mut Iso9660) -> Result<(), Box<dyn std::error::Error>> {
    use bt3_data::fod::{self, Action, Fod, FrameTag};

    // Title_Init(1): section 1 of archive baseFile + 1, BPE-packed into an inner pack.
    let id = 0x1C1 + 1;
    let location = partition_entry_for_id(id);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    let pack = archive.read_entry(image, location.index)?;
    let res = bpe::decode(pack_section(&pack, 1)?)?;
    println!(
        "\nTitle archive 0x{id:X}: {} bytes, section 1 unpacked to {} bytes",
        pack.len(),
        res.len()
    );
    for section in [1usize, 2, 3, 5, 6, 7, 8, 9, 10, 14] {
        let textures = TextureResource::parse(pack_section(&res, section)?)?;
        let formats: Vec<String> = textures
            .entries
            .iter()
            .map(|t| {
                let tex0 = t.texture_register;
                format!(
                    "{}x{}/psm{:X}{}",
                    1u32 << ((tex0 >> 26) & 0xF),
                    1u32 << ((tex0 >> 30) & 0xF),
                    (tex0 >> 20) & 0x3F,
                    if t.pixel_data.is_none() {
                        "/clut-only"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let decoded: Vec<String> = textures
            .entries
            .iter()
            .map(|t| match decode_sprite_texture(t) {
                Ok(_) => "ok".to_owned(),
                Err(error) => error.to_string(),
            })
            .collect();
        println!(
            "  section {section:2}: {} textures {} decode {decoded:?}",
            textures.entries.len(),
            formats.join(" ")
        );
    }

    let movie = Fod::parse(pack_section(&res, 11)?)?;
    let data = movie.bytes();
    let mut ofs = 0;
    let mut tags = Vec::new();
    loop {
        let tag = movie.header(ofs)?;
        tags.push(format!("{}x{}", tag.code, tag.count));
        if tag.code == 0 {
            break;
        }
        ofs += 8 + tag.size as usize;
    }
    println!("  section 11 FOD tags (code x records): {}", tags.join(" "));
    for index in 0..movie.record_count(3)? {
        let rec = movie.record(3, index)?;
        print!(
            "{}image {index}: {}x{}",
            if index == 0 { "  " } else { ", " },
            fod::u16_at(data, rec + 8)?,
            fod::u16_at(data, rec + 10)?
        );
    }
    println!();

    let mut flat = 0;
    let mut quad_flags = std::collections::BTreeSet::new();
    let mut quad_colors = std::collections::BTreeSet::new();
    for index in 0..movie.record_count(4)? {
        let rec = movie.record(4, index)?;
        for quad in 0..fod::u16_at(data, rec + 8)? as usize {
            let q = rec + 10 + quad * 0x18;
            if fod::i16_at(data, q)? < 0 {
                flat += 1;
            }
            quad_flags.insert(fod::u16_at(data, q + 2)?);
            quad_colors.insert(format!("{:08X}", fod::u32_at(data, q + 4)?));
        }
    }
    println!("  shape quad colours (RRGGBBAA): {quad_colors:?}");
    let mut tfx = std::collections::BTreeSet::new();
    for section in [1usize, 2, 3, 5, 6, 7, 8, 9, 10, 14] {
        for t in TextureResource::parse(pack_section(&res, section)?)?.entries {
            tfx.insert((t.texture_register >> 35) & 3);
        }
    }
    println!("  TEX0 TFX values in the title textures: {tfx:?}");
    println!(
        "  shapes: {} records, {flat} flat quads, unk02 values {quad_flags:?}",
        movie.record_count(4)?
    );

    let mut timelines = vec![("root".to_owned(), movie.record(7, 0)?)];
    for index in 0..movie.record_count(6)? {
        timelines.push((format!("sprite {index}"), movie.record(6, index)?));
    }
    let (mut skew, mut flip, mut frames, mut unknown) =
        (0, 0, 0, std::collections::BTreeSet::new());
    let mut urls = std::collections::BTreeSet::new();
    let mut labels = Vec::new();
    for (name, rec) in &timelines {
        let mut block = rec + 8;
        while let Some(frame) = fod::parse_frame(data, block)? {
            frames += 1;
            if !frame.label.is_empty() {
                labels.push(format!("{name}:{}", String::from_utf8_lossy(&frame.label)));
            }
            for tag in &frame.tags {
                let matrix = match tag {
                    FrameTag::Place { matrix, .. } | FrameTag::PlaceNew { matrix, .. } => {
                        Some(*matrix)
                    }
                    FrameTag::Move { matrix, .. } => *matrix,
                    FrameTag::Actions(actions) => {
                        for action in actions {
                            match action {
                                Action::GetUrl(url, arg) => {
                                    urls.insert(format!(
                                        "{} {}",
                                        String::from_utf8_lossy(url),
                                        String::from_utf8_lossy(arg)
                                    ));
                                }
                                Action::Other(code) => {
                                    unknown.insert(format!("action 0x{code:X}"));
                                }
                                _ => {}
                            }
                        }
                        None
                    }
                    FrameTag::Unknown(code) => {
                        unknown.insert(format!("tag {code}"));
                        None
                    }
                    _ => None,
                };
                if let Some(m) = matrix {
                    if m[1] != 0.0 || m[3] != 0.0 {
                        skew += 1;
                    }
                    if m[0] < 0.0 || m[4] < 0.0 {
                        flip += 1;
                    }
                }
            }
            block = fod::skip_named(data, block, 1)?.ok_or("frame list ends unexpectedly")?;
        }
    }
    println!(
        "  timelines: {}, frames: {frames}, matrices with skew/rotation: {skew}, mirrored: {flip}, unknown: {unknown:?}",
        timelines.len()
    );
    println!("  GetURL commands: {urls:?}");
    println!("  labels: {}", labels.join(" "));
    Ok(())
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

    // FirstRun_Main: archive 0 is file baseFile (0x1C1) + 0x19; its section 3 is BPE-packed.
    let logo_file_id = 0x1C1 + 0x19;
    let logo_location = partition_entry_for_id(logo_file_id);
    let logo_archive = AfsArchive::open(
        &mut image,
        format!("DATA/PZS3US{}.AFS", logo_location.partition),
    )?;
    let logo_pack = logo_archive.read_entry(&mut image, logo_location.index)?;
    println!(
        "\nLogo archive 0x{logo_file_id:X}: {} bytes",
        logo_pack.len()
    );
    let logos = bpe::decode(pack_section(&logo_pack, 3)?)?;
    println!("  section 3 unpacked: {} bytes", logos.len());
    for picture in [6, 7, 8, 3, 4, 5] {
        let resource = TextureResource::parse(pack_section(&logos, picture)?)?;
        let mut opaque = 0usize;
        for texture in &resource.entries {
            let rgba = decode_sprite_texture(texture)?;
            opaque += rgba.pixels.chunks_exact(4).filter(|p| p[3] == 255).count();
        }
        let tex0 = resource.entries[0].texture_register;
        println!(
            "  picture section {picture}: {} tiles of {}x{}, PSM 0x{:X}, {opaque} opaque texels",
            resource.entries.len(),
            1u32 << ((tex0 >> 26) & 0xF),
            1u32 << ((tex0 >> 30) & 0xF),
            (tex0 >> 20) & 0x3F,
        );
    }

    // Movie_Open streams DATA/ZS3USOP.ADX beside the video; Bgm_Play's tracks start at file 0x10B16.
    let movie_audio = image.read_file("DATA/ZS3USOP.ADX")?;
    let music_id = 0x10B16;
    let music_location = partition_entry_for_id(music_id);
    let music_archive = AfsArchive::open(
        &mut image,
        format!("DATA/PZS3US{}.AFS", music_location.partition),
    )?;
    let music = music_archive.read_entry(&mut image, music_location.index)?;
    for (label, bytes) in [
        ("DATA/ZS3USOP.ADX".to_owned(), &movie_audio),
        (format!("music file 0x{music_id:X}"), &music),
    ] {
        let (header, pcm) = adx::decode(bytes)?;
        let frames = pcm.len() / header.channels;
        let peak = pcm.iter().map(|s| i32::from(*s).abs()).max().unwrap_or(0);
        println!(
            "\n{label}: {} Hz, {} ch, cutoff {}, version {}, flags {}, loop {:?}, coefficients {:?}; {frames} of {} samples decoded ({:.1} s), peak {peak}",
            header.sample_rate,
            header.channels,
            header.cutoff,
            header.version,
            header.flags,
            header.loop_samples,
            adx::prediction_coefficients(header.cutoff, header.sample_rate),
            header.total_samples,
            frames as f64 / f64::from(header.sample_rate),
        );
    }
    if let Some(path) = std::env::var_os("BT3_WAV") {
        let (header, pcm) = adx::decode(&music)?;
        std::fs::write(
            &path,
            adx::to_wav(header.sample_rate, header.channels as u16, &pcm),
        )?;
        println!(
            "  wrote music file 0x{music_id:X} to {}",
            PathBuf::from(path).display()
        );
    }

    // Movie_Open: the opening video is DATA/ZS3USOP.PSS; probe its first 4 MiB through ffprobe's stdin.
    let pss_size = image.file_size("DATA/ZS3USOP.PSS")?;
    let pss_head =
        image.read_file_range("DATA/ZS3USOP.PSS", 0, (4 << 20).min(pss_size as usize))?;
    let mut probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-f", "mpeg", "-i", "pipe:0", "-show_entries"])
        .arg("stream=index,codec_type,codec_name,width,height,r_frame_rate,avg_frame_rate")
        .args(["-of", "compact"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut stdin = probe.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = std::io::Write::write_all(&mut stdin, &pss_head);
    });
    let output = probe.wait_with_output()?;
    let _ = writer.join();
    println!("\nDATA/ZS3USOP.PSS: {pss_size} bytes");
    print!("{}", String::from_utf8_lossy(&output.stdout));

    probe_title(&mut image)?;

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
