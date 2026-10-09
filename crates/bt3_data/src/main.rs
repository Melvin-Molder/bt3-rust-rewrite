use std::path::PathBuf;

use bt3_data::Iso9660;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;

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

    for screen_type in 0..3u32 {
        let file_id = 0x3C1 + screen_type;
        let location = partition_entry_for_id(file_id);
        let archive_path = format!("DATA/PZS3US{}.AFS", location.partition);
        let archive = AfsArchive::open(&mut image, archive_path)?;
        let entry = archive.entry(&mut image, location.index)?;
        let packed = archive.read_entry(&mut image, location.index)?;
        let decoded = bpe::decode(&packed)?;

        println!(
            "Loading-screen type {screen_type}, file ID 0x{file_id:X}: partition {}, entry {} of {} ({} packed bytes -> {} decoded bytes)",
            location.partition,
            location.index,
            archive.entry_count(),
            entry.size,
            decoded.len()
        );
    }

    Ok(())
}
