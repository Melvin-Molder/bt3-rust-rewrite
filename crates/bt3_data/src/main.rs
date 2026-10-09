use std::path::PathBuf;

use bt3_data::Iso9660;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso")
        });

    let mut image = Iso9660::open(&image_path)?;
    let system_config = image.read_root_file("SYSTEM.CNF")?;

    println!("Image: {}", image_path.display());
    println!("Volume: {}", image.volume_id());
    println!("Volume blocks: {}", image.volume_blocks());
    println!("SYSTEM.CNF:");
    print!("{}", String::from_utf8_lossy(&system_config));

    Ok(())
}
