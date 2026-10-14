//! Finding the user's disc image: the port is made for one release, and runs whatever image of it is in `rom/`.

use std::io;
use std::path::{Path, PathBuf};

use crate::Iso9660;
use crate::elf::boot_path;

/// The release the port reads: the USA disc (English / Japanese), product code SLUS-21678.
pub const SERIAL: &str = "SLUS-21678";
/// Its executable, as `SYSTEM.CNF`'s `BOOT2` names it.
pub const BOOT_FILE: &str = "SLUS_216.78";

/// The folder searched when no image is given: `rom/` under the working directory.
pub const DIR: &str = "rom";

/// The executable an image boots (`SYSTEM.CNF`'s `BOOT2`), if it is a PS2 disc.
pub fn boot_file(image: &Path) -> Option<String> {
    let mut iso = Iso9660::open(image).ok()?;
    boot_path(&iso.read_file("SYSTEM.CNF").ok()?)
}

/// Whether an image is the release the port reads.
pub fn is_supported(image: &Path) -> bool {
    boot_file(image).as_deref() == Some(BOOT_FILE)
}

/// The disc image in `dir`, whatever its name: the only `.iso` there, or of several the one of this release.
pub fn find_in(dir: &Path) -> io::Result<PathBuf> {
    let mut images: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "cannot read {}: {e}; put your disc image there",
                    dir.display()
                ),
            )
        })?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("iso")))
        .collect();
    images.sort();
    match images.len() {
        0 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "no .iso in {}: put your disc image of {SERIAL} there, or give its path",
                dir.display()
            ),
        )),
        1 => Ok(images.remove(0)),
        _ => {
            let mut ours: Vec<PathBuf> =
                images.iter().filter(|p| is_supported(p)).cloned().collect();
            match ours.len() {
                1 => Ok(ours.remove(0)),
                n => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} images in {} and {n} of them {SERIAL}: give the path of the one to run",
                        images.len(),
                        dir.display()
                    ),
                )),
            }
        }
    }
}

/// The image in the workspace's `rom/` for the tests that read the real game; an empty path (which no reader
/// opens, so the test is skipped) when there is none.
pub fn for_tests() -> PathBuf {
    find_in(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(DIR),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_missing_folder_says_where_the_image_goes() {
        let dir = std::env::temp_dir().join(format!("bt3-rom-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = find_in(&dir).unwrap_err();
        assert_eq!(empty.kind(), io::ErrorKind::NotFound);
        assert!(empty.to_string().contains(SERIAL), "{empty}");
        // One image is taken whatever its name (it is not opened).
        std::fs::write(dir.join("my disc.ISO"), b"").unwrap();
        assert_eq!(find_in(&dir).unwrap(), dir.join("my disc.ISO"));
        // Two that are not this release: the user picks.
        std::fs::write(dir.join("other.iso"), b"").unwrap();
        assert_eq!(
            find_in(&dir).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(find_in(&dir).is_err());
    }

    /// The image in `rom/`, when there is one, is this release.
    #[test]
    fn real_image_is_the_supported_release() {
        let image = for_tests();
        if Iso9660::open(&image).is_err() {
            return;
        }
        assert_eq!(boot_file(&image).as_deref(), Some(BOOT_FILE));
    }
}
