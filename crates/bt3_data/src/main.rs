use std::path::PathBuf;

use bt3_data::Iso9660;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso")
        });

    let mut image = Iso9660::open(&image_path)?;
    let system_config = image.read_file("SYSTEM.CNF")?;
    let file_id = 2;
    let location = partition_entry_for_id(file_id);
    let archive_path = format!("DATA/PZS3US{}.AFS", location.partition);
    let archive = AfsArchive::open(&mut image, archive_path)?;
    let entry = archive.entry(&mut image, location.index)?;
    let prefix = archive.read_entry_prefix(&mut image, location.index, 16)?;

    println!("Image: {}", image_path.display());
    println!("Volume: {}", image.volume_id());
    println!("Volume blocks: {}", image.volume_blocks());
    println!("SYSTEM.CNF:");
    print!("{}", String::from_utf8_lossy(&system_config));
    println!(
        "Common file ID {file_id}: partition {}, entry {} of {} ({} bytes; read {} prefix bytes)",
        location.partition,
        location.index,
        archive.entry_count(),
        entry.size,
        prefix.len()
    );

    Ok(())
}
