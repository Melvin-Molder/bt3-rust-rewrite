//! The memory card (src/sys/memcard.c over Sony's libmc). The card layer's operations are polled as the original
//! polls them, one libmc call or `sceMcSync` per frame. Below them `Libmc` emulates the library over a folder of
//! the host: port 0 (MEMORY CARD slot 1) holds a formatted 8 MB card whose directories and files are the folder's
//! own; port 1 is empty. Every call finishes at once, so the `sceMcSync` that follows it reports its result.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// `sceMcGetInfo`'s card type of a PS2 memory card.
pub const TYPE_PS2: i32 = 2;
/// Clusters (1 KiB) a formatted 8 MB card has free.
const CARD_CLUSTERS: i64 = 8000;
/// Clusters a directory takes (its entries).
const DIR_CLUSTERS: i64 = 2;

// libmc results.
const MC_NOT_FOUND: i32 = -4;
const MC_NO_CARD: i32 = -10;
const MC_BAD_FD: i32 = -5;

// sceMcOpen modes.
const O_READ: i32 = 1;
const O_WRITE: i32 = 2;
const O_CREAT: i32 = 0x200;

/// An open file of the emulated card.
struct OpenFile {
    file: fs::File,
}

/// Sony's libmc over a folder (`root`, created on the first write). `fresh`: the next `sceMcGetInfo` reports a
/// newly inserted card (-1), as libmc does after it starts.
pub struct Libmc {
    root: Option<PathBuf>,
    files: Vec<Option<OpenFile>>,
    pending: Option<i32>,
    fresh: bool,
}

impl Libmc {
    /// A card stored in `root`, or an empty slot without one.
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            files: Vec::new(),
            pending: None,
            fresh: true,
        }
    }

    fn finish(&mut self, result: i32) -> i32 {
        self.pending = Some(result);
        0
    }

    /// A card path (`/dir/file` or `dir`) inside the folder; none for one that leaves it.
    fn host(&self, name: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let mut path = root.clone();
        for part in name.split('/').filter(|p| !p.is_empty()) {
            if part == "." || part == ".." || part.contains(['\\', ':']) {
                return None;
            }
            path.push(part);
        }
        Some(path)
    }

    /// Clusters in use: each file's size rounded up, and each directory's entries.
    fn used(&self) -> i64 {
        let Some(root) = &self.root else {
            return 0;
        };
        let Ok(dirs) = fs::read_dir(root) else {
            return 0;
        };
        let mut used = 0;
        for dir in dirs.flatten() {
            used += DIR_CLUSTERS;
            if let Ok(files) = fs::read_dir(dir.path()) {
                for f in files.flatten() {
                    let len = f.metadata().map_or(0, |m| m.len()) as i64;
                    used += (len + 0x3FF) / 0x400;
                }
            }
        }
        used
    }

    /// `sceMcGetInfo`: the type, free clusters and format state (any of them may be left out); the result is
    /// 0, -1 for a newly inserted card, or -10 for none.
    pub fn get_info(&mut self, port: usize, info: Option<(&mut i32, &mut i32, &mut i32)>) -> i32 {
        let present = port == 0 && self.root.is_some();
        if let Some((card_type, free, format)) = info {
            *card_type = if present { TYPE_PS2 } else { 0 };
            *free = if present {
                (CARD_CLUSTERS - self.used()).max(0) as i32
            } else {
                0
            };
            *format = i32::from(present);
        }
        let result = if !present {
            MC_NO_CARD
        } else if std::mem::take(&mut self.fresh) {
            -1
        } else {
            0
        };
        self.finish(result)
    }

    /// `sceMcSync(1, ...)`: 1 with the result of the call that finished, -1 when none was pending.
    pub fn sync(&mut self, result: &mut i32) -> i32 {
        match self.pending.take() {
            Some(r) => {
                *result = r;
                1
            }
            None => -1,
        }
    }

    /// `sceMcMkdir`: 0, or -4 when the directory is there already.
    pub fn mkdir(&mut self, name: &str) -> i32 {
        let Some(path) = self.host(name) else {
            return self.finish(MC_NO_CARD);
        };
        let result = if path.is_dir() {
            MC_NOT_FOUND
        } else if fs::create_dir_all(&path).is_ok() {
            0
        } else {
            MC_NO_CARD
        };
        self.finish(result)
    }

    /// `sceMcOpen`: a descriptor, or -4 for a file (or directory) that is not there without `O_CREAT`.
    pub fn open(&mut self, name: &str, mode: i32) -> i32 {
        let Some(path) = self.host(name) else {
            return self.finish(MC_NO_CARD);
        };
        let exists = path.is_file();
        if !exists && (mode & O_CREAT == 0 || !path.parent().is_some_and(Path::is_dir)) {
            return self.finish(MC_NOT_FOUND);
        }
        let mut opts = fs::OpenOptions::new();
        opts.read(mode & 3 != O_WRITE)
            .write(mode & 3 != O_READ)
            .create(mode & O_CREAT != 0);
        let result = match opts.open(&path) {
            Ok(file) => {
                let slot = self
                    .files
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or_else(|| {
                        self.files.push(None);
                        self.files.len() - 1
                    });
                self.files[slot] = Some(OpenFile { file });
                slot as i32
            }
            Err(_) => MC_NOT_FOUND,
        };
        self.finish(result)
    }

    fn file(&mut self, fd: i32) -> Option<&mut fs::File> {
        self.files
            .get_mut(usize::try_from(fd).ok()?)?
            .as_mut()
            .map(|f| &mut f.file)
    }

    /// `sceMcClose`.
    pub fn close(&mut self, fd: i32) -> i32 {
        let result = match usize::try_from(fd).ok().and_then(|i| self.files.get_mut(i)) {
            Some(slot @ Some(_)) => {
                *slot = None;
                0
            }
            _ => MC_BAD_FD,
        };
        self.finish(result)
    }

    /// `sceMcSeek`: the new position (origin 0 the start, 1 the position, 2 the end).
    pub fn seek(&mut self, fd: i32, offset: i32, origin: i32) -> i32 {
        let from = match origin {
            0 => SeekFrom::Start(offset.max(0) as u64),
            1 => SeekFrom::Current(i64::from(offset)),
            _ => SeekFrom::End(i64::from(offset)),
        };
        let result = match self.file(fd) {
            Some(f) => f.seek(from).map_or(MC_BAD_FD, |p| p as i32),
            None => MC_BAD_FD,
        };
        self.finish(result)
    }

    /// `sceMcRead`: the bytes read.
    pub fn read(&mut self, fd: i32, buf: &mut [u8]) -> i32 {
        let result = match self.file(fd) {
            Some(f) => {
                let mut n = 0;
                while n < buf.len() {
                    match f.read(&mut buf[n..]) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => n += k,
                    }
                }
                n as i32
            }
            None => MC_BAD_FD,
        };
        self.finish(result)
    }

    /// `sceMcWrite`: the bytes written.
    pub fn write(&mut self, fd: i32, data: &[u8]) -> i32 {
        let result = match self.file(fd) {
            Some(f) => f
                .write_all(data)
                .and_then(|()| f.flush())
                .map_or(MC_NO_CARD, |()| data.len() as i32),
            None => MC_BAD_FD,
        };
        self.finish(result)
    }

    /// `sceMcGetDir(port, 0, name, 0, -1, NULL)` in the root: the number of entries named `name` (`*`: all).
    pub fn get_dir(&mut self, name: &str) -> i32 {
        let Some(root) = self.root.clone() else {
            return self.finish(MC_NO_CARD);
        };
        let count = fs::read_dir(&root).map_or(0, |dirs| {
            dirs.flatten()
                .filter(|d| name == "*" || d.file_name().to_string_lossy() == name)
                .count()
        });
        self.finish(count as i32)
    }

    /// `sceMcFormat`: an empty card.
    pub fn format(&mut self) -> i32 {
        let Some(root) = self.root.clone() else {
            return self.finish(MC_NO_CARD);
        };
        self.files.clear();
        let _ = fs::remove_dir_all(&root);
        let result = if fs::create_dir_all(&root).is_ok() {
            0
        } else {
            MC_NO_CARD
        };
        self.finish(result)
    }
}

/// `McCardPort`: one port's state as the operations leave it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Port {
    /// `MCCARD_F_*`: bit 0 the size read differs, bit 1 it is the size asked.
    pub flags: i32,
    pub ret: i32,
    pub result: i32,
    pub card_type: i32,
    pub free: i32,
    pub format: i32,
    pub fd: i32,
    /// Clusters a save still needs (`McCard_CalcNeed`).
    pub size: i32,
}

/// `MCCARD_F_DIFF`.
pub const F_DIFF: i32 = 1;
const F_SAME: i32 = 2;

// MCCARD_CMD_*.
const CMD_INFO: i32 = 10;
const CMD_FORMAT: i32 = 12;
const CMD_CREATE: i32 = 14;
const CMD_SAVE: i32 = 15;
const CMD_READ: i32 = 16;
const CMD_CHECK: i32 = 18;
const CMD_FIND: i32 = 23;
const CMD_INFO_PROBE: i32 = 25;
const CMD_NEED: i32 = 26;

/// `McCardFile`: the names of a save and what its icon.sys and icon get.
#[derive(Clone, Debug, Default)]
pub struct SaveFile {
    pub dir: String,
    pub sys_path: String,
    pub icon_path: String,
    pub data_path: String,
    /// The icon.sys title (Shift-JIS) and the byte offset of its line break.
    pub title: Vec<u8>,
    pub title_break: u16,
    pub icon_name: String,
    pub icon: Vec<u8>,
}

impl SaveFile {
    /// `McCardFile_SetNames`: the directory "BASLUS-21678" + `name` (+ two digits for `num` >= 0), its icon.sys,
    /// its icon (dbzsmr.ico for a replay, else dbzsm.ico) and the data file named as the directory.
    pub fn set_names(&mut self, name: &str, num: i32, replay: bool) {
        self.dir = if num >= 0 {
            format!("BASLUS-21678{name}{num:02}")
        } else {
            format!("BASLUS-21678{name}")
        };
        self.sys_path = format!("/{}/icon.sys", self.dir);
        self.icon_name = if replay { "dbzsmr.ico" } else { "dbzsm.ico" }.to_string();
        self.icon_path = format!("/{}/{}", self.dir, self.icon_name);
        self.data_path = format!("/{}/{}", self.dir, self.dir);
    }

    /// `McCardIconSys` as `McCard_CreateSave` fills it (0x3C4 bytes).
    fn icon_sys(&self) -> Vec<u8> {
        let mut b = vec![0u8; 0x3C4];
        b[..4].copy_from_slice(b"PS2D");
        b[6..8].copy_from_slice(&self.title_break.to_le_bytes());
        b[0xC..0x10].copy_from_slice(&0x60i32.to_le_bytes());
        let bg = [
            [0xFF, 0x80, 0, 0],
            [0xFF, 0x80, 0, 0],
            [0x80, 0x80, 0x80, 0],
            [0x80, 0x80, 0x80, 0],
        ];
        for (i, v) in bg.iter().flatten().enumerate() {
            b[0x10 + i * 4..0x14 + i * 4].copy_from_slice(&(*v as i32).to_le_bytes());
        }
        let floats: [f32; 28] = [
            0.5, 0.5, 0.5, 0.0, 0.5, 0.5, 0.5, 0.0, 0.5, 0.5, 0.5, 0.0, // light directions
            0.3, 0.3, 0.03, 0.0, 0.3, 0.2, 0.2, 0.0, 0.14, 0.14, 0.2, 0.0, // light colours
            0.25, 0.25, 0.25, 0.0, // ambient
        ];
        for (i, v) in floats.iter().enumerate() {
            b[0x50 + i * 4..0x54 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let title = &self.title[..self.title.len().min(0x43)];
        b[0xC0..0xC0 + title.len()].copy_from_slice(title);
        for at in [0x104, 0x144, 0x184] {
            let name = self.icon_name.as_bytes();
            b[at..at + name.len()].copy_from_slice(name);
        }
        b
    }
}

/// `McCard_IsFatalError`: the codes the flows give up on.
pub fn is_fatal_error(code: i32) -> bool {
    matches!(code, -19..=-11 | -49..=-40 | -59..=-50 | -79..=-70)
}

/// `McCard_Checksum`: over the bytes after the first 8 (odd ones times 256, kept to 16 bits), its high byte in
/// word 0 and its low byte in word 1. `write` stores it (and gives false); else true means a mismatch (both words
/// must differ, as the original tests).
pub fn checksum(buf: &mut [u8], write: bool) -> bool {
    if buf.len() < 8 {
        return true;
    }
    if write {
        buf[..8].fill(0);
    }
    let mut sum: u32 = 0;
    for (i, &b) in buf[8..].iter().enumerate() {
        sum += if i & 1 != 0 {
            u32::from(b) << 8
        } else {
            u32::from(b)
        };
        if sum > 0xFFFF {
            sum -= 0x1_0000;
        }
    }
    let word = |b: &[u8], i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
    if write {
        buf[..4].copy_from_slice(&(sum >> 8).to_le_bytes());
        buf[4..8].copy_from_slice(&(sum & 0xFF).to_le_bytes());
        false
    } else {
        word(buf, 0) != sum >> 8 && word(buf, 1) != sum & 0xFF
    }
}

/// The card layer: `gMcCardStep`, `gMcCardCmd` and the ports, over the emulated library.
pub struct McCard {
    pub lib: Libmc,
    step: i32,
    cmd: i32,
    pub port: [Port; 2],
}

impl McCard {
    /// `McCard_Init`: the ports cleared and port 0 probed (which takes libmc's "new card").
    pub fn new(lib: Libmc) -> Self {
        let mut card = Self {
            lib,
            step: 0,
            cmd: 0,
            port: [Port::default(); 2],
        };
        card.probe(0);
        card
    }

    /// `McCard_ResetStep`.
    pub fn reset_step(&mut self) {
        self.step = 0;
    }

    /// `MCCARD_ENTER`: an operation entered while another's command is set starts over.
    fn enter(&mut self, cmd: i32) {
        if self.cmd != 0 && self.cmd != cmd {
            self.step = 0;
        }
    }

    /// `sceMcSync(1)` into the port's result: false while the call runs (never, here).
    fn wait(&mut self, port: usize) -> bool {
        let mut r = self.port[port].result;
        let done = self.lib.sync(&mut r) != 0;
        self.port[port].result = r;
        done
    }

    /// `MCCARD_WAIT_OK`.
    fn wait_ok(&mut self, port: usize) -> bool {
        self.wait(port) && self.port[port].result >= 0
    }

    /// `MCCARD_CALL`: the call's return in `ret`; true when it was accepted.
    fn call(&mut self, port: usize, ret: i32) -> bool {
        self.port[port].ret = ret;
        ret >= 0
    }

    /// `McCard_GetInfo`: type, free space and format; true once finished.
    pub fn get_info(&mut self, port: usize) -> bool {
        self.enter(CMD_INFO);
        match self.step {
            0 => {
                self.port[port].result = 0;
                self.cmd = CMD_INFO;
                let p = &mut self.port[port];
                let ret = self
                    .lib
                    .get_info(port, Some((&mut p.card_type, &mut p.free, &mut p.format)));
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                let mut r = 0;
                if self.lib.sync(&mut r) == 1 {
                    self.port[port].result = r;
                    self.step += 1;
                    return true;
                }
                false
            }
            _ => true,
        }
    }

    /// `McCard_Probe`: the card state's result code only, waited for.
    pub fn probe(&mut self, port: usize) -> i32 {
        self.cmd = CMD_INFO_PROBE;
        self.port[port].result = 0;
        self.port[port].ret = self.lib.get_info(port, None);
        let mut r = 0;
        let s = self.lib.sync(&mut r);
        self.port[port].result = r;
        s
    }

    /// `McCard_Format`: true once the format finished without error.
    pub fn format(&mut self, port: usize) -> bool {
        self.enter(CMD_FORMAT);
        match self.step {
            0 => {
                self.port[port].result = 0;
                self.cmd = CMD_FORMAT;
                let ret = self.lib.format();
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                if !self.wait(port) || self.port[port].result < 0 {
                    return false;
                }
                self.step = 0;
                self.cmd = 0;
                true
            }
            _ => {
                self.step = 0;
                self.cmd = 0;
                true
            }
        }
    }

    /// `McCard_CreateSave`: the directory (an existing one is fine), icon.sys and the icon file.
    pub fn create_save(&mut self, port: usize, file: &SaveFile) -> bool {
        self.enter(CMD_CREATE);
        let step = self.step;
        let advance = |c: &mut Self| {
            c.step += 1;
            false
        };
        match step {
            0 => {
                self.port[port].result = 0;
                self.cmd = CMD_CREATE;
                let ret = self.lib.mkdir(&file.dir);
                if self.call(port, ret) {
                    return advance(self);
                }
                false
            }
            1 => {
                if !self.wait(port) {
                    return false;
                }
                if self.port[port].result < 0 {
                    if self.port[port].result != MC_NOT_FOUND {
                        return false;
                    }
                    self.port[port].result = 0;
                }
                advance(self)
            }
            2 | 8 => {
                let path = if step == 2 {
                    &file.sys_path
                } else {
                    &file.icon_path
                };
                let ret = self.lib.open(path, O_CREAT | O_WRITE);
                if self.call(port, ret) {
                    return advance(self);
                }
                false
            }
            3 | 9 => {
                if !self.wait_ok(port) {
                    return false;
                }
                self.port[port].fd = self.port[port].result;
                advance(self)
            }
            4 | 10 => {
                let data = if step == 4 {
                    file.icon_sys()
                } else {
                    file.icon.clone()
                };
                let ret = self.lib.write(self.port[port].fd, &data);
                if self.call(port, ret) {
                    return advance(self);
                }
                false
            }
            5 | 7 | 11 => {
                if !self.wait_ok(port) {
                    return false;
                }
                advance(self)
            }
            6 | 12 => {
                let ret = self.lib.close(self.port[port].fd);
                if self.call(port, ret) {
                    return advance(self);
                }
                false
            }
            13 => {
                if !self.wait_ok(port) {
                    return false;
                }
                self.step += 1;
                true
            }
            _ => {
                self.step = 0;
                self.cmd = 0;
                true
            }
        }
    }

    /// `McCard_WriteSave`: the data file (created if needed) with `data`, which must all be written (-8
    /// otherwise); then the icon file, created when missing and rewritten when its size differs.
    pub fn write_save(&mut self, port: usize, data: &[u8], file: &SaveFile) -> bool {
        self.enter(CMD_SAVE);
        match self.step {
            0 => {
                self.port[port].result = 0;
                self.cmd = CMD_SAVE;
                let ret = self.lib.open(&file.data_path, O_CREAT | O_WRITE);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                if !self.wait(port) {
                    return false;
                }
                if self.port[port].result < 0 {
                    self.port[port].result = 0;
                    self.step = 2;
                } else {
                    self.port[port].fd = self.port[port].result;
                    self.step = 4;
                }
                false
            }
            2 => {
                let ret = self.lib.open(&file.data_path, O_WRITE);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            3 => {
                if self.wait_ok(port) {
                    self.port[port].fd = self.port[port].result;
                    self.step += 1;
                }
                false
            }
            4 => {
                let ret = self.lib.write(self.port[port].fd, data);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            5 => {
                if self.wait_ok(port) {
                    if self.port[port].result != data.len() as i32 {
                        self.port[port].result = -8;
                        return false;
                    }
                    self.step += 1;
                }
                false
            }
            6 | 18 => {
                let ret = self.lib.close(self.port[port].fd);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            7 | 15 | 17 => {
                if self.wait_ok(port) {
                    self.step += 1;
                }
                false
            }
            8 => {
                let ret = self.lib.open(&file.icon_path, 3);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            9 => {
                if !self.wait(port) {
                    return false;
                }
                if self.port[port].result < 0 {
                    if self.port[port].result == MC_NOT_FOUND {
                        self.port[port].result = 0;
                        self.step += 1;
                    }
                    return false;
                }
                self.port[port].fd = self.port[port].result;
                self.step = 12;
                false
            }
            10 => {
                let ret = self.lib.open(&file.icon_path, O_CREAT | O_WRITE);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            11 => {
                if self.wait_ok(port) {
                    self.port[port].fd = self.port[port].result;
                    self.step = 16;
                }
                false
            }
            12 | 14 => {
                let origin = if self.step == 12 { 2 } else { 0 };
                let ret = self.lib.seek(self.port[port].fd, 0, origin);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            13 => {
                if self.wait_ok(port) {
                    self.step = if file.icon.len() as i32 != self.port[port].result {
                        14
                    } else {
                        18
                    };
                }
                false
            }
            16 => {
                let ret = self.lib.write(self.port[port].fd, &file.icon);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            19 => {
                if !self.wait_ok(port) {
                    return false;
                }
                self.step += 1;
                true
            }
            _ => {
                self.step = 0;
                self.cmd = 0;
                true
            }
        }
    }

    /// `McCard_ReadFile`: `buf.len()` bytes of the data file; the port's flags say whether they all came. A
    /// failed step starts the operation over.
    pub fn read_file(&mut self, port: usize, buf: &mut [u8], file: &SaveFile) -> bool {
        self.enter(CMD_READ);
        match self.step {
            0 => {
                self.port[port].flags = 0;
                self.port[port].result = 0;
                self.cmd = CMD_READ;
                let ret = self.lib.open(&file.data_path, O_READ);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                if self.wait(port) {
                    if self.port[port].result < 0 {
                        self.step = 0;
                    } else {
                        self.port[port].fd = self.port[port].result;
                        self.step += 1;
                    }
                }
                false
            }
            2 => {
                let ret = self.lib.read(self.port[port].fd, buf);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            3 => {
                if self.wait(port) {
                    if self.port[port].result < 0 {
                        self.step = 0;
                        return false;
                    }
                    self.port[port].flags |= if self.port[port].result != buf.len() as i32 {
                        F_DIFF
                    } else {
                        F_SAME
                    };
                    self.step += 1;
                }
                false
            }
            4 => {
                let ret = self.lib.close(self.port[port].fd);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            5 => {
                if !self.wait(port) {
                    return false;
                }
                if self.port[port].result < 0 {
                    self.step = 0;
                    return false;
                }
                self.step += 1;
                true
            }
            _ => true,
        }
    }

    /// `McCard_CheckFiles`: the data file, icon.sys and the icon can all be opened.
    pub fn check_files(&mut self, port: usize, file: &SaveFile) -> bool {
        self.enter(CMD_CHECK);
        let path = |step: i32| match step / 4 {
            0 => &file.data_path,
            1 => &file.sys_path,
            _ => &file.icon_path,
        };
        match self.step {
            0 | 4 | 8 => {
                if self.step == 0 {
                    self.port[port].result = 0;
                    self.cmd = CMD_CHECK;
                }
                let ret = self.lib.open(path(self.step), O_READ);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 | 5 | 9 => {
                let mut r = self.port[port].result;
                if self.lib.sync(&mut r) != 1 {
                    return false;
                }
                self.port[port].result = r;
                if r >= 0 {
                    self.port[port].fd = r;
                    self.step += 1;
                }
                false
            }
            2 | 6 | 10 => {
                let ret = self.lib.close(self.port[port].fd);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            3 | 7 => {
                if self.wait_ok(port) {
                    self.step += 1;
                }
                false
            }
            11 => {
                if !self.wait_ok(port) {
                    return false;
                }
                self.step += 1;
                true
            }
            _ => true,
        }
    }

    /// `McCard_FindSystemSave` / `McCard_FindSave`: how many root entries the directory `dir` has (1 when
    /// it is there).
    pub fn find_dir(&mut self, port: usize, dir: &str) -> bool {
        self.enter(CMD_FIND);
        match self.step {
            0 => {
                self.port[port].result = 0;
                self.cmd = CMD_FIND;
                let ret = self.lib.get_dir(dir);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                let mut r = 0;
                if self.lib.sync(&mut r) == 1 {
                    self.port[port].result = r;
                    self.step += 1;
                    return true;
                }
                false
            }
            _ => true,
        }
    }

    /// `McCard_CalcNeed`: the clusters a save still needs (`size`): for each of its files, the file's budget
    /// (system save 16 / 1 / 40, replay 107 / 1 / 48) less what it takes. A missing file stops at its step.
    pub fn calc_need(&mut self, port: usize, file: &SaveFile, replay: bool) -> bool {
        let need = if replay {
            [0x6B, 1, 0x30]
        } else {
            [0x10, 1, 0x28]
        };
        self.enter(CMD_NEED);
        if self.step >= 24 {
            return true;
        }
        let k = (self.step / 8) as usize;
        match self.step % 8 {
            0 => {
                if self.step == 0 {
                    self.port[port].size = 0;
                    self.port[port].result = 0;
                    self.cmd = CMD_NEED;
                }
                let path = [&file.data_path, &file.sys_path, &file.icon_path][k.min(2)];
                let ret = self.lib.open(path, O_READ);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            1 => {
                let mut r = self.port[port].result;
                if self.lib.sync(&mut r) != 1 {
                    return false;
                }
                self.port[port].result = r;
                if r >= 0 {
                    self.port[port].fd = r;
                    self.step += 1;
                }
                false
            }
            2 | 4 => {
                let origin = if self.step % 8 == 2 { 2 } else { 0 };
                let ret = self.lib.seek(self.port[port].fd, 0, origin);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            3 => {
                if self.wait_ok(port) {
                    self.port[port].size +=
                        need[k.min(2)] - (self.port[port].result + 0x3FF) / 0x400;
                    self.step += 1;
                }
                false
            }
            5 => {
                if self.wait_ok(port) {
                    self.step += 1;
                }
                false
            }
            6 => {
                let ret = self.lib.close(self.port[port].fd);
                if self.call(port, ret) {
                    self.step += 1;
                }
                false
            }
            _ => {
                if !self.wait_ok(port) {
                    return false;
                }
                self.step += 1;
                self.step == 24
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str) -> (McCard, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("bt3-card-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (McCard::new(Libmc::new(Some(root.clone()))), root)
    }

    fn run(mut f: impl FnMut() -> bool) {
        for _ in 0..64 {
            if f() {
                return;
            }
        }
        panic!("the operation never finished");
    }

    #[test]
    fn a_save_is_created_written_found_and_read_back() {
        let (mut c, root) = card("save");
        // McCard_Init took the "new card"; the next query is the plain one.
        run(|| c.get_info(0));
        assert_eq!(
            (c.port[0].card_type, c.port[0].result, c.port[0].format),
            (TYPE_PS2, 0, 1)
        );
        assert_eq!(c.port[0].free, 8000);
        c.reset_step();
        let mut f = SaveFile {
            icon: vec![7; 100],
            title: b"TITLE".to_vec(),
            title_break: 2,
            ..SaveFile::default()
        };
        f.set_names("DBZT3R", 3, true);
        assert_eq!(f.data_path, "/BASLUS-21678DBZT3R03/BASLUS-21678DBZT3R03");
        assert_eq!(f.icon_path, "/BASLUS-21678DBZT3R03/dbzsmr.ico");
        // Nothing there yet: the check stops at its first open with -4.
        for _ in 0..4 {
            assert!(!c.check_files(0, &f));
        }
        assert_eq!(c.port[0].result, MC_NOT_FOUND);
        c.reset_step();
        run(|| c.find_dir(0, &f.dir));
        assert_eq!(c.port[0].result, 0);
        c.reset_step();
        run(|| c.create_save(0, &f));
        run(|| c.write_save(0, &[1, 2, 3, 4, 5], &f));
        c.reset_step();
        run(|| c.check_files(0, &f));
        c.reset_step();
        run(|| c.find_dir(0, &f.dir));
        assert_eq!(c.port[0].result, 1);
        let sys = fs::read(root.join(&f.dir).join("icon.sys")).unwrap();
        assert_eq!((&sys[..4], sys.len()), (&b"PS2D"[..], 0x3C4));
        assert_eq!(&sys[0xC0..0xC5], b"TITLE");
        c.reset_step();
        let mut buf = [0u8; 5];
        run(|| c.read_file(0, &mut buf, &f));
        assert_eq!((buf, c.port[0].flags), ([1, 2, 3, 4, 5], F_SAME));
        // The replay budget less the three files' clusters (1 each).
        c.reset_step();
        run(|| c.calc_need(0, &f, true));
        assert_eq!(c.port[0].size, 0x6B + 1 + 0x30 - 3);
        c.reset_step();
        run(|| c.get_info(0));
        assert_eq!(c.port[0].free, 8000 - 2 - 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_slot_has_no_card() {
        let mut c = McCard::new(Libmc::new(None));
        run(|| c.get_info(0));
        assert_eq!((c.port[0].card_type, c.port[0].result), (0, MC_NO_CARD));
        assert!(!is_fatal_error(MC_NO_CARD) && is_fatal_error(-11));
    }

    #[test]
    fn the_checksum_round_trips_and_catches_a_change() {
        let mut b: Vec<u8> = (0..64u8).collect();
        assert!(!checksum(&mut b, true));
        assert!(!checksum(&mut b, false));
        b[20] ^= 0xFF;
        assert!(checksum(&mut b, false));
    }
}
