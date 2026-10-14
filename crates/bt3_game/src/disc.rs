//! The DVD drive as the disc swap of Disc Fusion sees it (`Disc_GetDriveState` / `Disc_Identify`, src/sys/file.c):
//! the tray, the disc in it, the drive's status and which game's disc it is, found by the boot file the root
//! directory holds (`gDiscExeNameTbl`). The console's drive is the host's here: the tray opens and closes with a
//! key and the discs that can be put in are this game's image and the ones named with `--disc`.

use std::path::PathBuf;

use bt3_data::Iso9660;

/// `Disc_GetDriveState` results (`DISC_*`): stopped, tray open, paused, no disc.
pub const STOPPED: i32 = 0;
pub const TRAY_OPEN: i32 = 1;
pub const PAUSED: i32 = 4;
pub const NO_DISC: i32 = 7;
/// `Disc_Identify`: the disc type is not known yet.
pub const BUSY: i32 = -2;

/// `gDiscExeNameTbl`: the boot files of Budokai Tenkaichi, 2 and 3 (USA), in the order `Disc_Identify` returns.
const EXE_NAMES: [&str; 3] = ["SLUS_212.27", "SLUS_214.41", "SLUS_216.78"];
/// Frames a closed tray reads as stopped, then as busy, while the disc spins up.
const SPIN_UP: u32 = 60;
const READ_TOC: u32 = 30;

/// The drive.
#[derive(Debug)]
pub struct DiscDrive {
    /// The discs to choose from (`None` an empty tray); 0 is this game's.
    discs: Vec<Option<PathBuf>>,
    /// The disc in the tray.
    current: usize,
    tray_open: bool,
    /// Frames since the tray closed.
    closed: u32,
    /// `Disc_Identify` of the disc in the tray, read once.
    ident: Option<i32>,
}

impl DiscDrive {
    /// This game's image in a closed drive; `others` the discs that can be put in instead.
    pub fn new(own: PathBuf, others: Vec<PathBuf>) -> Self {
        let mut discs = vec![Some(own)];
        discs.extend(others.into_iter().map(Some));
        discs.push(None);
        Self {
            discs,
            current: 0,
            tray_open: false,
            closed: SPIN_UP + READ_TOC,
            ident: None,
        }
    }

    /// Opens or closes the tray.
    pub fn toggle_tray(&mut self) {
        self.tray_open = !self.tray_open;
        self.closed = 0;
        self.ident = None;
    }

    /// With the tray open, puts the next disc in (after the last, none).
    pub fn next_disc(&mut self) {
        if self.tray_open {
            self.current = (self.current + 1) % self.discs.len();
        }
    }

    /// What is in the tray, for the host's messages.
    pub fn current(&self) -> Option<&PathBuf> {
        self.discs[self.current].as_ref()
    }

    pub fn is_open(&self) -> bool {
        self.tray_open
    }

    /// One frame of the drive.
    pub fn tick(&mut self) {
        if !self.tray_open {
            self.closed = self.closed.saturating_add(1);
        }
    }

    /// `Disc_GetDriveState`: the tray open, a closed drive stopped while it spins up and then paused, no disc when
    /// the tray is empty.
    pub fn state(&self) -> i32 {
        if self.tray_open {
            return TRAY_OPEN;
        }
        if self.current().is_none() {
            return NO_DISC;
        }
        if self.closed < SPIN_UP {
            STOPPED
        } else {
            PAUSED
        }
    }

    /// `Disc_Identify`: busy while the disc's type is not read yet, then the index of the boot file the disc holds
    /// (0 Budokai Tenkaichi, 1 the second, 2 this game), -1 for another disc and -3 for none.
    pub fn identify(&mut self) -> i32 {
        let Some(path) = self.current().cloned() else {
            return -3;
        };
        if self.tray_open || self.closed < SPIN_UP + READ_TOC {
            return BUSY;
        }
        *self.ident.get_or_insert_with(|| {
            let Ok(mut image) = Iso9660::open(&path) else {
                return -1;
            };
            EXE_NAMES
                .iter()
                .position(|name| image.file_size(name).is_ok())
                .map_or(-1, |i| i as i32)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game's own image from `rom/`, when it is there, is identified as disc 2 once it has spun up; an empty
    /// tray reads as no disc.
    #[test]
    fn the_drive_identifies_this_game() {
        let iso = bt3_data::rom::for_tests();
        if !iso.exists() {
            return;
        }
        let mut drive = DiscDrive::new(iso, Vec::new());
        assert_eq!(drive.state(), PAUSED);
        assert_eq!(drive.identify(), 2);
        drive.toggle_tray();
        assert_eq!(drive.state(), TRAY_OPEN);
        drive.next_disc();
        drive.toggle_tray();
        assert_eq!(drive.state(), NO_DISC);
        drive.toggle_tray();
        drive.next_disc();
        drive.toggle_tray();
        assert_eq!(drive.state(), STOPPED);
        assert_eq!(drive.identify(), BUSY);
        for _ in 0..SPIN_UP + READ_TOC {
            drive.tick();
        }
        assert_eq!(drive.state(), PAUSED);
        assert_eq!(drive.identify(), 2);
    }
}
