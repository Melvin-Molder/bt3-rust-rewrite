//! Sound effects as the game plays them: `Snd_PlaySeEx` (src/sys/snd.c) queues a PLAY that SOUNDS.IRX hands to
//! Sony's SE sequencer (MODSESQ2.IRX); the sequencer sends note messages to the synthesizer (MODHSYN.IRX), which
//! sets up SPU2 voices from the bank's SE timbres. Behaviour of the two Sony modules is taken from their
//! disassembly (addresses below are .text offsets of MODSESQ2 `M:` and MODHSYN `H:`); the SPU2 voice follows the
//! hardware documentation (psx-spx, "Sound Processing Unit").
//!
//! Only what the game's banks use is implemented: note-on / note-off events, SE timbres without LFO, transpose or
//! exclusive groups. Anything else in the data stops the sequence and is reported.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::audio::{Decodable, Source};
use bevy::prelude::*;
use bt3_data::snd::{SeBank, Sesq, Timbre, Vag};

/// SPU2 output rate; PITCH 0x1000 plays a sample at this rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// SOUNDS.IRX hard timer period (S:790): both modules tick every 4167 µs.
const TICK_USEC: u64 = 4167;
const VOICES: usize = 48;
const BANKS: usize = 8;
/// `gSndBankVolume`.
const BANK_VOLUME: [i32; BANKS] = [0xD8, 0xE0, 0x80, 0x80, 0x80, 0x80, 0x80, 0];
/// sceHSyn port volume: `Snd_SetMasterVolume(0x7F)` at boot sends it to every port (S:220c → hsyn #10).
const PORT_VOLUME: i32 = 0x7F;

/// A loaded bank: the sample body as sound RAM and the parsed tables.
pub struct LoadedBank {
    body: Vec<u8>,
    vags: Vec<Vag>,
    timbre_sets: Vec<Option<Vec<Option<Timbre>>>>,
    sesq: Sesq,
}

impl LoadedBank {
    pub fn parse(file: &[u8]) -> std::io::Result<Self> {
        let bank = SeBank::parse(file)?;
        Ok(Self {
            body: bank.body.to_vec(),
            vags: bank.vags,
            timbre_sets: bank.timbre_sets,
            sesq: bank.sesq,
        })
    }
}

/// One RPC PLAY after `Snd_PlaySeEx`'s scaling: volume 0..0x7F, pan 0..0x7F, pitch in cents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Play {
    pub slot: usize,
    pub id: u8,
    pub volume: u8,
    pub pan: u8,
    pub pitch: i16,
    /// The game's handle of the sound (`Snd_PlaySeEx`'s result), 0 for none.
    pub handle: u32,
}

/// A `SndCmd` of the per-frame queue.
enum Cmd {
    Play(Play),
    /// Type 2: stop by handle.
    Stop(u32),
    /// RPC 0xA (`Snd_StopBank`): the voices of the banks in the mask.
    StopBanks(u32),
}

/// The EE side of `snd.c`: bank slots and the per-frame queue.
pub struct SoundEffects {
    engine: Arc<Mutex<Engine>>,
    loaded: [bool; BANKS],
    queue: Vec<Cmd>,
}

impl SoundEffects {
    pub fn new(engine: Arc<Mutex<Engine>>) -> Self {
        Self {
            engine,
            loaded: [false; BANKS],
            queue: Vec::new(),
        }
    }

    /// `Snd_LoadBank`: nothing happens when the slot is in use.
    pub fn load_bank(&mut self, mask: u32, bank: LoadedBank) {
        let slot = bank_index(mask);
        if self.loaded[slot] {
            return;
        }
        self.loaded[slot] = true;
        self.engine.lock().unwrap().banks[slot] = Some(Arc::new(bank));
    }

    /// `Snd_ReloadBank`: the slot's bank replaced, its sounds stopped.
    pub fn reload_bank(&mut self, mask: u32, bank: LoadedBank) {
        let slot = bank_index(mask);
        self.loaded[slot] = true;
        let mut engine = self.engine.lock().unwrap();
        engine.stop_banks(1 << slot);
        engine.banks[slot] = Some(Arc::new(bank));
    }

    /// `Snd_UnloadBank`: the slot emptied, its sounds stopped.
    pub fn unload_bank(&mut self, mask: u32) {
        let slot = bank_index(mask);
        self.loaded[slot] = false;
        let mut engine = self.engine.lock().unwrap();
        engine.stop_banks(1 << slot);
        engine.banks[slot] = None;
    }

    /// `Snd_StopHandle`: queues a stop of the sound `Snd_PlaySeEx` gave this handle.
    pub fn stop_handle(&mut self, handle: u32) {
        if self.queue.len() < 32 {
            self.queue.push(Cmd::Stop(handle));
        }
    }

    /// `Snd_StopBank`: queues a stop of the voices of the banks in `mask`.
    pub fn stop_banks(&mut self, mask: u32) {
        if self.queue.len() < 32 {
            self.queue.push(Cmd::StopBanks(mask));
        }
    }

    /// `Snd_PlaySe(mask, id)` = `Snd_PlaySeEx(mask, id, 0x40, 0, 0)`.
    pub fn play_se(&mut self, mask: u32, id: u32) {
        self.play_se_ex(mask, id, 0x40, 0, 0, 0);
    }

    /// `Snd_PlaySeEx` + `Snd_QueuePlay`, the sound known by `handle` (0: none).
    pub fn play_se_ex(
        &mut self,
        mask: u32,
        id: u32,
        volume: i32,
        pan: i32,
        pitch: i32,
        handle: u32,
    ) {
        let slot = bank_index(mask);
        if !self.loaded[slot] || self.queue.len() >= 32 || id >= 0x100 {
            return;
        }
        // Snd_ScaleVolume: with the save's sound effect volume.
        let option = crate::sound::options().se_volume;
        let volume = (volume * BANK_VOLUME[slot] * option / 0x480).clamp(0, 0x7F);
        let pan = (pan + 0x40).clamp(0, 0x7F);
        let pitch = ((pitch as f32 * 18.75) as i32).clamp(-1200, 1200);
        self.queue.push(Cmd::Play(Play {
            slot,
            id: id as u8,
            volume: volume as u8,
            pan: pan as u8,
            pitch: pitch as i16,
            handle,
        }));
    }

    /// `Snd_Update`: sends the frame's queue.
    pub fn update(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let mut engine = self.engine.lock().unwrap();
        for cmd in self.queue.drain(..) {
            match cmd {
                Cmd::Play(play) => engine.play(play),
                Cmd::Stop(handle) => engine.stop_handle(handle),
                Cmd::StopBanks(mask) => engine.stop_banks(mask),
            }
        }
    }
}

/// `Snd_BankIndex`: the lowest set bit of the low 8.
fn bank_index(mask: u32) -> usize {
    (0..BANKS).find(|i| mask & (1 << i) != 0).unwrap_or(0)
}

/// A message on the sequencer → synthesizer stream (`FE 01 id 8n ...`, M:da8).
#[derive(Clone, Copy, Debug)]
struct NoteMessage {
    port: usize,
    id: u32,
    set: usize,
    timbre: u8,
    note: u8,
    velocity: u8,
    pan: i8,
    volume: u8,
    pan_offset: u8,
    pitch_offset: i16,
}

/// A playing SE sequence (MODSESQ2 track).
struct Track {
    port: usize,
    id: u32,
    data: Arc<[u8]>,
    cursor: usize,
    /// Absolute time of the next event and the current time, in sequence ticks.
    next: u32,
    now: u32,
    acc: u64,
    tick_len: u64,
    velocity_scale: [i32; 3],
    pan: i32,
    volume: u8,
    pan_offset: u8,
    pitch: i16,
}

/// Sequencer port settings SOUNDS.IRX writes after `sceSESq2Load` (S:1158-1280).
fn port_velocity(port: usize) -> i32 {
    if matches!(port, 2 | 4 | 5) {
        0xFF
    } else {
        0x7F
    }
}
const PORT_PAN: i32 = 0x40;
const PORT_RATE: u64 = 1000;

pub struct Engine {
    banks: [Option<Arc<LoadedBank>>; BANKS],
    /// The game's handles of the latest plays and their sequences' ids.
    handles: Vec<(u32, u32)>,
    tracks: Vec<Track>,
    stream: Vec<NoteMessage>,
    voices: Vec<Voice>,
    serial: u16,
    tick_acc: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            banks: Default::default(),
            handles: Vec::new(),
            tracks: Vec::new(),
            stream: Vec::new(),
            voices: (0..VOICES).map(|_| Voice::default()).collect(),
            serial: 0,
            tick_acc: 0,
        }
    }
}

impl Engine {
    /// SOUNDS.IRX QUEUE PLAY → `sceSESq2Play(ctx, port, 0, id, vol, pan, pitch)` (S:184c, M:2d4c): set 0,
    /// sequence `id`; the track starts on the next tick.
    fn play(&mut self, play: Play) {
        let Some(bank) = self.banks[play.slot].clone() else {
            return;
        };
        let Some(set) = bank.sesq.sets.first().and_then(Option::as_ref) else {
            return;
        };
        let Some(seq) = set
            .sequences
            .get(usize::from(play.id))
            .and_then(Option::as_ref)
        else {
            return;
        };
        // M:854: the sequence rate counts twice, then the set, Sesq and port rates.
        let rate = |a: u64, b: u64| a * b / 1000;
        let speed = rate(
            rate(
                rate(
                    rate(
                        rate(u64::from(seq.rate), u64::from(seq.rate)),
                        u64::from(set.rate),
                    ),
                    1000,
                ),
                u64::from(bank.sesq.rate),
            ),
            PORT_RATE,
        );
        if speed == 0 {
            return;
        }
        // M:c5c: signed byte sum, clamped.
        let pan =
            (0x40 + PORT_PAN + i32::from(bank.sesq.pan) + i32::from(set.pan) + i32::from(seq.pan)
                - 256)
                .clamp(-127, 127);
        self.serial = self.serial.wrapping_add(1);
        let port = play.slot;
        let mut track = Track {
            port,
            id: 0xF800_0000 | (port as u32) << 16 | u32::from(self.serial),
            data: seq.data.clone().into(),
            cursor: 0,
            next: 0,
            now: 0,
            acc: 0,
            tick_len: 128_000_000 / speed,
            velocity_scale: [
                port_velocity(port),
                i32::from(bank.sesq.volume),
                i32::from(set.volume) * i32::from(seq.volume),
            ],
            pan,
            volume: play.volume,
            pan_offset: play.pan,
            pitch: play.pitch,
        };
        match read_vlq(&track.data, &mut track.cursor) {
            Some(delta) => track.next = delta,
            None => return,
        }
        if play.handle != 0 {
            if self.handles.len() >= 64 {
                self.handles.remove(0);
            }
            self.handles.push((play.handle, track.id));
        }
        self.tracks.push(track);
    }

    /// A stop by handle: the sound's sequence ends and its voices are released (a looping sample keeps sounding
    /// until then). How SOUNDS.IRX does it is not documented; this is the plain reading of the command.
    fn stop_handle(&mut self, handle: u32) {
        let Some(i) = self.handles.iter().position(|&(h, _)| h == handle) else {
            return;
        };
        let (_, id) = self.handles.remove(i);
        self.tracks.retain(|t| t.id != id);
        self.stream.retain(|m| m.id != id);
        for voice in &mut self.voices {
            if voice.owner.is_some_and(|o| o.id == id) {
                voice.spu.key_off();
            }
        }
    }

    /// The sounds of the banks in `mask` (a bit per slot) end: their sequences and voices.
    fn stop_banks(&mut self, mask: u32) {
        let in_mask = |id: u32| mask >> ((id >> 16) & 0xFF) & 1 != 0;
        self.tracks.retain(|t| mask >> t.port & 1 == 0);
        self.stream.retain(|m| mask >> m.port & 1 == 0);
        for voice in &mut self.voices {
            if voice.owner.is_some_and(|o| in_mask(o.id)) {
                voice.spu.key_off();
            }
        }
    }

    /// One SOUNDS.IRX wakeup: the synthesizer reads what the sequencer wrote last time, then the sequencer runs.
    fn tick(&mut self) {
        let messages = std::mem::take(&mut self.stream);
        self.free_finished_voices();
        for message in messages {
            if message.velocity == 0 {
                self.note_off(&message);
            } else {
                self.note_on(&message);
            }
        }
        let mut tracks = std::mem::take(&mut self.tracks);
        tracks.retain_mut(|track| {
            track.acc += TICK_USEC * 128;
            let mut alive = run_track(track, &mut self.stream);
            while alive && track.acc >= track.tick_len {
                track.acc -= track.tick_len;
                track.now += 1;
                alive = run_track(track, &mut self.stream);
            }
            alive
        });
        self.tracks = tracks;
    }

    /// H:544c: a voice is given back once its envelope is silent.
    fn free_finished_voices(&mut self) {
        for voice in &mut self.voices {
            if voice.owner.is_some() && !voice.spu.is_on() {
                voice.owner = None;
            }
        }
    }

    /// H:8714 (SE timbre note-on) + H:5a14 / H:5c30 (voice setup) + KON.
    fn note_on(&mut self, message: &NoteMessage) {
        let Some(bank) = self.banks[message.port].clone() else {
            return;
        };
        let Some(timbre) = bank
            .timbre_sets
            .get(message.set)
            .and_then(Option::as_ref)
            .and_then(|set| set.get(usize::from(message.timbre)))
            .and_then(Option::as_ref)
        else {
            return;
        };
        if message.note < timbre.note_min || message.note > timbre.note_max {
            return;
        }
        let tone = timbre.tones[usize::from(message.note - timbre.note_min)];
        // A tone past the last sample would be a noise voice; the game's banks have none.
        let Some(vag) = bank.vags.get(usize::from(tone.vag)).copied() else {
            warn!(
                "SE tone {} wants a noise voice, which is not emulated",
                tone.vag
            );
            return;
        };
        if vag.offset == u32::MAX {
            return;
        }
        if tone.lfo_mode & 0x77 != 0
            || tone.exclusive_group != 0
            || tone.semitone != 0
            || tone.fine != 0
        {
            warn!("SE tone uses LFO, exclusive group or transpose, which are not emulated");
        }
        let priority = tone.priority;
        let index = match self.voices.iter().position(|v| v.owner.is_none()) {
            Some(index) => index,
            // H:8d18: steal the lowest priority, then the quietest.
            None => {
                let (index, _) = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.priority <= priority)
                    .min_by_key(|(_, v)| (v.priority, v.spu.envelope_level))
                    .unwrap_or((0, &self.voices[0]));
                index
            }
        };
        let curve = velocity_curve(tone.velocity_curve, message.velocity);
        // H:4ca4: port volume × Play volume / 127, times timbre volume × curve, >> 8; then × pan / 128.
        let base = i32::from(tone.volume) * curve;
        let level = (PORT_VOLUME * i32::from(message.volume) / 127) * base >> 8;
        let (pan_l, pan_r, phase) = pan_levels(
            tone.pan,
            tone.pan_mode & 1 != 0,
            message.pan,
            message.pan_offset,
        );
        let side = |pan: i32| {
            let value = (level * pan >> 7).min(0x3FFF);
            // Direct mode holds volume / 2; the phase flag 0 writes the negated value.
            if phase { value * 2 } else { -value * 2 }
        };
        let pitch = voice_pitch(vag.sample_rate, i32::from(message.pitch_offset));
        let voice = &mut self.voices[index];
        voice.owner = Some(Owner {
            id: message.id,
            set: message.set,
            timbre: message.timbre,
            note: message.note,
        });
        voice.priority = priority;
        voice.bank = Some(bank.clone());
        voice.spu.volume = [
            if tone.mix & 1 != 0 { side(pan_l) } else { 0 },
            if tone.mix & 2 != 0 { side(pan_r) } else { 0 },
        ];
        voice.spu.pitch = pitch;
        voice.spu.adsr1 = tone.adsr1;
        voice.spu.adsr2 = tone.adsr2;
        voice.spu.key_on(vag.offset as usize);
    }

    /// H:787c: KOFF for the voices of that id, set, timbre and note.
    fn note_off(&mut self, message: &NoteMessage) {
        for voice in &mut self.voices {
            if voice.owner.is_some_and(|o| {
                (o.id == message.id || message.id == 0xFF)
                    && o.set == message.set
                    && o.timbre == message.timbre
                    && o.note == message.note
            }) {
                voice.spu.key_off();
            }
        }
    }

    /// Mixes `out.len() / 2` stereo frames at 48 kHz.
    pub fn render(&mut self, out: &mut [f32]) {
        let mono = crate::sound::options().mono;
        for frame in out.chunks_exact_mut(2) {
            self.tick_acc += 1_000_000;
            if self.tick_acc >= TICK_USEC * u64::from(SAMPLE_RATE) {
                self.tick_acc -= TICK_USEC * u64::from(SAMPLE_RATE);
                self.tick();
            }
            let (mut left, mut right) = (0i32, 0i32);
            for voice in &mut self.voices {
                if let Some(bank) = &voice.bank {
                    let (l, r) = voice.spu.sample(&bank.body);
                    left += l;
                    right += r;
                }
            }
            // Snd_SetStereo(0): the driver mixes both sides into each.
            if mono {
                let m = (left + right) / 2;
                (left, right) = (m, m);
            }
            frame[0] = left.clamp(-0x8000, 0x7FFF) as f32 / 32768.0;
            frame[1] = right.clamp(-0x8000, 0x7FFF) as f32 / 32768.0;
        }
    }
}

/// Interprets events up to the current time (M:cb4). Returns false when the sequence is over.
fn run_track(track: &mut Track, stream: &mut Vec<NoteMessage>) -> bool {
    while track.next <= track.now {
        let Some(&status) = track.data.get(track.cursor) else {
            return false;
        };
        track.cursor += 1;
        match status {
            0xFF => return false,
            0x90..=0xAF => {
                let Some(operands) = track.data.get(track.cursor..track.cursor + 3) else {
                    return false;
                };
                let (timbre, note, v) = (operands[0], operands[1], operands[2]);
                track.cursor += 3;
                // M:bc8: velocity × port, Sesq, set and sequence volumes, each pair over 127².
                let [port, sesq, set_seq] = track.velocity_scale;
                let a = i32::from(v) * port * sesq / 16129;
                let velocity = (a * set_seq / 16129).min(127) as u8;
                stream.push(NoteMessage {
                    port: track.port,
                    id: track.id,
                    set: usize::from(status & 0xF),
                    timbre,
                    note,
                    velocity,
                    pan: track.pan as i8,
                    volume: track.volume,
                    pan_offset: track.pan_offset,
                    pitch_offset: track.pitch,
                });
            }
            _ => {
                warn!("SE sequence event {status:#04X} is not emulated; the sequence stops");
                return false;
            }
        }
        match read_vlq(&track.data, &mut track.cursor) {
            Some(delta) => track.next += delta,
            None => return false,
        }
    }
    true
}

fn read_vlq(data: &[u8], cursor: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    loop {
        let byte = *data.get(*cursor)?;
        *cursor += 1;
        value = value << 7 | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
}

/// H:98a4 with the function table at H:c288 (entries 6..9 repeat 0..3; custom tables are not set up by the game).
fn velocity_curve(curve: u8, velocity: u8) -> i32 {
    let v = i32::from(velocity.max(1));
    let square = |v: i32| (v * v / 127).max(1);
    match (curve & 0xF) % 6 {
        0 => v,
        1 => 128 - v,
        2 => square(v),
        3 => 128 - square(v),
        4 => 128 - square(128 - v),
        _ => square(128 - v),
    }
}

/// H:5158: the left / right pan factors (0..128) and the phase flag.
fn pan_levels(tone_pan: i8, surround: bool, sequence_pan: i8, pan_offset: u8) -> (i32, i32, bool) {
    let p0 = (i32::from(sequence_pan) - 64).clamp(-191, 63);
    let offset = i32::from(pan_offset as i8);
    let mut phase = true;
    let p = if surround {
        let fold = |p: &mut i32, phase: &mut bool| {
            if *p < 0 {
                *phase = !*phase;
                *p = -*p;
            } else if *p >= 128 {
                *phase = !*phase;
                *p = 256 - *p;
            }
        };
        let mut p = i32::from(tone_pan);
        if p < 0 {
            p = -p;
            phase = false;
        }
        p += p0;
        fold(&mut p, &mut phase);
        p = p - 64 + offset;
        fold(&mut p, &mut phase);
        p - 64
    } else {
        p0 + i32::from(tone_pan) - 64 + offset.max(0) - 64
    };
    let (mut l, mut r) = (128, 128);
    if p > 0 {
        l = if p >= 63 { 0 } else { (63 - p) * 128 / 63 };
    } else if p < 0 {
        r = if p < -62 { 0 } else { (p + 63) * 128 / 63 };
    }
    (l, r, phase)
}

/// H:4a48: the note only selects the tone; pitch comes from the sample rate and the pitch offset.
fn voice_pitch(sample_rate: u16, offset: i32) -> u16 {
    // sceSdNote2Pitch(key, 0, key, offset): 0x1000 for no offset. The game never sends one (pitch 0); others use
    // the equal-tempered value, as the libsd table is not part of the files read here.
    let base = if offset == 0 {
        0x1000u64
    } else {
        (4096.0 * 2f64.powf(f64::from(offset) / 1536.0)) as u64
    };
    let rate = ((u64::from(sample_rate) << 16) as u32 as u64 * 0x0576_19F1) >> 42;
    ((base * rate) >> 16).min(0x3FFF) as u16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    id: u32,
    set: usize,
    timbre: u8,
    note: u8,
}

#[derive(Default)]
struct Voice {
    owner: Option<Owner>,
    priority: u8,
    bank: Option<Arc<LoadedBank>>,
    spu: SpuVoice,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AdsrPhase {
    #[default]
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// One SPU2 voice (psx-spx "SPU ADPCM Samples", "Pitch", "Volume and ADSR Generator").
#[derive(Default)]
struct SpuVoice {
    adsr1: u16,
    adsr2: u16,
    pitch: u16,
    /// Effective left / right volume (-0x8000..0x7FFE).
    volume: [i32; 2],
    address: usize,
    loop_address: usize,
    counter: u32,
    /// The last three samples of the previous block, then this block's 28.
    samples: [i16; 31],
    history: [i32; 2],
    flags: u8,
    decoded: bool,
    phase: AdsrPhase,
    envelope_level: i16,
    envelope_counter: u32,
}

impl SpuVoice {
    fn is_on(&self) -> bool {
        self.phase != AdsrPhase::Off
    }

    fn key_on(&mut self, start: usize) {
        self.address = start;
        // LSAX is never written by modhsyn; a sample without a loop-start block keeps whatever was there. Taking
        // the start address is this port's choice for that unobservable value.
        self.loop_address = start;
        self.counter = 0;
        self.samples = [0; 31];
        self.history = [0; 2];
        self.decoded = false;
        self.envelope_level = 0;
        self.envelope_counter = 0;
        self.phase = AdsrPhase::Attack;
    }

    fn key_off(&mut self) {
        if matches!(self.phase, AdsrPhase::Off | AdsrPhase::Release) {
            return;
        }
        self.phase = AdsrPhase::Release;
        self.envelope_counter = 0;
    }

    fn force_off(&mut self) {
        self.phase = AdsrPhase::Off;
        self.envelope_level = 0;
    }

    /// Decodes the 16-byte block at `address`.
    fn decode_block(&mut self, ram: &[u8]) {
        const POS: [i32; 16] = [0, 60, 115, 98, 122, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        const NEG: [i32; 16] = [0, 0, -52, -55, -60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let block = ram.get(self.address..self.address + 16).unwrap_or(&[0; 16]);
        self.samples.copy_within(28..31, 0);
        let shift = match block[0] & 0xF {
            s @ 0..=12 => s,
            _ => 9,
        };
        let filter = usize::from(block[0] >> 4);
        self.flags = block[1];
        let [mut old, mut older] = self.history;
        for i in 0..28 {
            let nibble = (block[2 + i / 2] >> (4 * (i & 1))) & 0xF;
            let t = i32::from(((u16::from(nibble) << 12) as i16) >> shift);
            let s =
                (t + ((old * POS[filter] + older * NEG[filter] + 32) >> 6)).clamp(-0x8000, 0x7FFF);
            self.samples[3 + i] = s as i16;
            older = old;
            old = s;
        }
        self.history = [old, older];
        if self.flags & 4 != 0 {
            self.loop_address = self.address;
        }
        self.decoded = true;
    }

    /// One 48 kHz output sample.
    fn sample(&mut self, ram: &[u8]) -> (i32, i32) {
        if !self.is_on() {
            return (0, 0);
        }
        if !self.decoded {
            self.decode_block(ram);
        }
        let i = ((self.counter >> 4) & 0xFF) as usize;
        let s = (self.counter >> 12) as usize;
        let w = &self.samples[s..s + 4];
        let interpolated = (i32::from(GAUSS[0xFF - i]) * i32::from(w[0]) >> 15)
            + (i32::from(GAUSS[0x1FF - i]) * i32::from(w[1]) >> 15)
            + (i32::from(GAUSS[0x100 + i]) * i32::from(w[2]) >> 15)
            + (i32::from(GAUSS[i]) * i32::from(w[3]) >> 15);
        let out = interpolated * i32::from(self.envelope_level) >> 15;
        self.tick_adsr();
        self.counter += u32::from(self.pitch.min(0x3FFF));
        if self.counter >> 12 >= 28 {
            self.counter -= 28 << 12;
            self.decoded = false;
            self.address += 16;
            if self.flags & 1 != 0 {
                self.address = self.loop_address;
                if self.flags & 2 == 0 {
                    self.force_off();
                }
            }
        }
        (out * self.volume[0] >> 15, out * self.volume[1] >> 15)
    }

    fn tick_adsr(&mut self) {
        let (adsr1, adsr2) = (u32::from(self.adsr1), u32::from(self.adsr2));
        let level = i32::from(self.envelope_level);
        match self.phase {
            AdsrPhase::Off => {}
            AdsrPhase::Attack => {
                self.envelope_step(
                    (adsr1 >> 10) & 0x1F,
                    (adsr1 >> 8) & 3,
                    adsr1 & 0x8000 != 0,
                    false,
                    0x7F,
                );
                if self.envelope_level >= 0x7FFF {
                    self.phase = AdsrPhase::Decay;
                    self.envelope_counter = 0;
                }
            }
            AdsrPhase::Decay => {
                let target = (((adsr1 & 0xF) + 1) * 0x800).min(0x7FFF) as i32;
                if level > target {
                    self.envelope_step((adsr1 >> 4) & 0xF, 0, true, true, 0x1F << 2);
                }
                if i32::from(self.envelope_level) <= target {
                    self.phase = AdsrPhase::Sustain;
                    self.envelope_counter = 0;
                }
            }
            AdsrPhase::Sustain => {
                self.envelope_step(
                    (adsr2 >> 8) & 0x1F,
                    (adsr2 >> 6) & 3,
                    adsr2 & 0x8000 != 0,
                    adsr2 & 0x4000 != 0,
                    0x7F,
                );
            }
            AdsrPhase::Release => {
                self.envelope_step(adsr2 & 0x1F, 0, adsr2 & 0x20 != 0, true, 0x1F << 2);
                if self.envelope_level == 0 {
                    self.phase = AdsrPhase::Off;
                }
            }
        }
    }

    /// psx-spx "Envelope Operation depending on Shift/Step/Mode/Direction"; `never` is the rate whose all-ones value
    /// never steps.
    fn envelope_step(
        &mut self,
        shift: u32,
        step_value: u32,
        exponential: bool,
        decreasing: bool,
        never: u32,
    ) {
        let rate = shift << 2 | step_value;
        if rate & never == never {
            return;
        }
        let mut step = 7 - step_value as i32;
        if decreasing {
            step = !step;
        }
        step <<= 11u32.saturating_sub(shift);
        let mut increment = (0x8000u32 >> shift.saturating_sub(11)).max(1);
        let level = i32::from(self.envelope_level);
        if exponential && !decreasing && level > 0x6000 {
            if shift < 10 {
                step >>= 2;
            } else if shift >= 11 {
                increment = (increment >> 2).max(1);
            } else {
                step >>= 1;
                increment = (increment >> 1).max(1);
            }
        } else if exponential && decreasing {
            step = step * level >> 15;
        }
        self.envelope_counter += increment;
        if self.envelope_counter & 0x8000 == 0 {
            return;
        }
        self.envelope_counter = 0;
        let next = level + step;
        self.envelope_level = if decreasing {
            next.max(0)
        } else {
            next.clamp(-0x8000, 0x7FFF)
        } as i16;
    }
}

/// The SPU's 4-point Gaussian interpolation table (psx-spx).
#[rustfmt::skip]
const GAUSS: [i16; 512] = [
    -0x001,-0x001,-0x001,-0x001,-0x001,-0x001,-0x001,-0x001, -0x001,-0x001,-0x001,-0x001,-0x001,-0x001,-0x001,-0x001,
    0x0000,0x0000,0x0000,0x0000,0x0000,0x0000,0x0000,0x0001, 0x0001,0x0001,0x0001,0x0002,0x0002,0x0002,0x0003,0x0003,
    0x0003,0x0004,0x0004,0x0005,0x0005,0x0006,0x0007,0x0007, 0x0008,0x0009,0x0009,0x000A,0x000B,0x000C,0x000D,0x000E,
    0x000F,0x0010,0x0011,0x0012,0x0013,0x0015,0x0016,0x0018, 0x0019,0x001B,0x001C,0x001E,0x0020,0x0021,0x0023,0x0025,
    0x0027,0x0029,0x002C,0x002E,0x0030,0x0033,0x0035,0x0038, 0x003A,0x003D,0x0040,0x0043,0x0046,0x0049,0x004D,0x0050,
    0x0054,0x0057,0x005B,0x005F,0x0063,0x0067,0x006B,0x006F, 0x0074,0x0078,0x007D,0x0082,0x0087,0x008C,0x0091,0x0096,
    0x009C,0x00A1,0x00A7,0x00AD,0x00B3,0x00BA,0x00C0,0x00C7, 0x00CD,0x00D4,0x00DB,0x00E3,0x00EA,0x00F2,0x00FA,0x0101,
    0x010A,0x0112,0x011B,0x0123,0x012C,0x0135,0x013F,0x0148, 0x0152,0x015C,0x0166,0x0171,0x017B,0x0186,0x0191,0x019C,
    0x01A8,0x01B4,0x01C0,0x01CC,0x01D9,0x01E5,0x01F2,0x0200, 0x020D,0x021B,0x0229,0x0237,0x0246,0x0255,0x0264,0x0273,
    0x0283,0x0293,0x02A3,0x02B4,0x02C4,0x02D6,0x02E7,0x02F9, 0x030B,0x031D,0x0330,0x0343,0x0356,0x036A,0x037E,0x0392,
    0x03A7,0x03BC,0x03D1,0x03E7,0x03FC,0x0413,0x042A,0x0441, 0x0458,0x0470,0x0488,0x04A0,0x04B9,0x04D2,0x04EC,0x0506,
    0x0520,0x053B,0x0556,0x0572,0x058E,0x05AA,0x05C7,0x05E4, 0x0601,0x061F,0x063E,0x065C,0x067C,0x069B,0x06BB,0x06DC,
    0x06FD,0x071E,0x0740,0x0762,0x0784,0x07A7,0x07CB,0x07EF, 0x0813,0x0838,0x085D,0x0883,0x08A9,0x08D0,0x08F7,0x091E,
    0x0946,0x096F,0x0998,0x09C1,0x09EB,0x0A16,0x0A40,0x0A6C, 0x0A98,0x0AC4,0x0AF1,0x0B1E,0x0B4C,0x0B7A,0x0BA9,0x0BD8,
    0x0C07,0x0C38,0x0C68,0x0C99,0x0CCB,0x0CFD,0x0D30,0x0D63, 0x0D97,0x0DCB,0x0E00,0x0E35,0x0E6B,0x0EA1,0x0ED7,0x0F0F,
    0x0F46,0x0F7F,0x0FB7,0x0FF1,0x102A,0x1065,0x109F,0x10DB, 0x1116,0x1153,0x118F,0x11CD,0x120B,0x1249,0x1288,0x12C7,
    0x1307,0x1347,0x1388,0x13C9,0x140B,0x144D,0x1490,0x14D4, 0x1517,0x155C,0x15A0,0x15E6,0x162C,0x1672,0x16B9,0x1700,
    0x1747,0x1790,0x17D8,0x1821,0x186B,0x18B5,0x1900,0x194B, 0x1996,0x19E2,0x1A2E,0x1A7B,0x1AC8,0x1B16,0x1B64,0x1BB3,
    0x1C02,0x1C51,0x1CA1,0x1CF1,0x1D42,0x1D93,0x1DE5,0x1E37, 0x1E89,0x1EDC,0x1F2F,0x1F82,0x1FD6,0x202A,0x207F,0x20D4,
    0x2129,0x217F,0x21D5,0x222C,0x2282,0x22DA,0x2331,0x2389, 0x23E1,0x2439,0x2492,0x24EB,0x2545,0x259E,0x25F8,0x2653,
    0x26AD,0x2708,0x2763,0x27BE,0x281A,0x2876,0x28D2,0x292E, 0x298B,0x29E7,0x2A44,0x2AA1,0x2AFF,0x2B5C,0x2BBA,0x2C18,
    0x2C76,0x2CD4,0x2D33,0x2D91,0x2DF0,0x2E4F,0x2EAE,0x2F0D, 0x2F6C,0x2FCC,0x302B,0x308B,0x30EA,0x314A,0x31AA,0x3209,
    0x3269,0x32C9,0x3329,0x3389,0x33E9,0x3449,0x34A9,0x3509, 0x3569,0x35C9,0x3629,0x3689,0x36E8,0x3748,0x37A8,0x3807,
    0x3867,0x38C6,0x3926,0x3985,0x39E4,0x3A43,0x3AA2,0x3B00, 0x3B5F,0x3BBD,0x3C1B,0x3C79,0x3CD7,0x3D35,0x3D92,0x3DEF,
    0x3E4C,0x3EA9,0x3F05,0x3F62,0x3FBD,0x4019,0x4074,0x40D0, 0x412A,0x4185,0x41DF,0x4239,0x4292,0x42EB,0x4344,0x439C,
    0x43F4,0x444C,0x44A3,0x44FA,0x4550,0x45A6,0x45FC,0x4651, 0x46A6,0x46FA,0x474E,0x47A1,0x47F4,0x4846,0x4898,0x48E9,
    0x493A,0x498A,0x49D9,0x4A29,0x4A77,0x4AC5,0x4B13,0x4B5F, 0x4BAC,0x4BF7,0x4C42,0x4C8D,0x4CD7,0x4D20,0x4D68,0x4DB0,
    0x4DF7,0x4E3E,0x4E84,0x4EC9,0x4F0E,0x4F52,0x4F95,0x4FD7, 0x5019,0x505A,0x509A,0x50DA,0x5118,0x5156,0x5194,0x51D0,
    0x520C,0x5247,0x5281,0x52BA,0x52F3,0x532A,0x5361,0x5397, 0x53CC,0x5401,0x5434,0x5467,0x5499,0x54CA,0x54FA,0x5529,
    0x5558,0x5585,0x55B2,0x55DE,0x5609,0x5632,0x565B,0x5684, 0x56AB,0x56D1,0x56F6,0x571B,0x573E,0x5761,0x5782,0x57A3,
    0x57C3,0x57E2,0x57FF,0x581C,0x5838,0x5853,0x586D,0x5886, 0x589E,0x58B5,0x58CB,0x58E0,0x58F4,0x5907,0x5919,0x592A,
    0x593A,0x5949,0x5958,0x5965,0x5971,0x597C,0x5986,0x598F, 0x5997,0x599E,0x59A4,0x59A9,0x59AD,0x59B0,0x59B2,0x59B3,
];

/// The engine as a Bevy audio asset; its decoder renders the SPU output as it plays.
#[derive(Asset, TypePath, Clone)]
pub struct SeOutput(pub Arc<Mutex<Engine>>);

pub struct SeSource {
    engine: Arc<Mutex<Engine>>,
    buffer: Vec<f32>,
    position: usize,
}

impl Decodable for SeOutput {
    type Decoder = SeSource;

    fn decoder(&self) -> SeSource {
        SeSource {
            engine: self.0.clone(),
            buffer: vec![0.0; 512],
            position: 512,
        }
    }
}

impl Iterator for SeSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.position == self.buffer.len() {
            self.engine.lock().unwrap().render(&mut self.buffer);
            self.position = 0;
        }
        self.position += 1;
        Some(self.buffer[self.position - 1])
    }
}

impl Source for SeSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> bevy::audio::ChannelCount {
        bevy::audio::ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> bevy::audio::SampleRate {
        bevy::audio::SampleRate::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{GAUSS, SpuVoice, pan_levels, velocity_curve, voice_pitch};

    /// Renders the title's sound effects from the ISO in `rom/`, when it is there.
    #[test]
    fn real_title_effects_sound_and_end() {
        use super::{Engine, LoadedBank, SAMPLE_RATE, SoundEffects};
        use bt3_data::Iso9660;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use std::sync::{Arc, Mutex};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let (common, menu) = (read(0x14B), read(0x14C));
        for (mask, id) in [(1, 0), (1, 1), (2, 0x37), (2, 0x38)] {
            let engine = Arc::new(Mutex::new(Engine::default()));
            let mut effects = SoundEffects::new(engine.clone());
            effects.load_bank(1, LoadedBank::parse(&common).unwrap());
            effects.load_bank(2, LoadedBank::parse(&menu).unwrap());
            effects.play_se(mask, id);
            effects.update();
            let mut out = vec![0f32; SAMPLE_RATE as usize * 2 * 4];
            engine.lock().unwrap().render(&mut out);
            let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
            let first = out.iter().position(|s| *s != 0.0).map(|i| i / 2);
            let last = out.iter().rposition(|s| *s != 0.0).map(|i| i / 2);
            println!(
                "SE ({mask}, {id:#X}): first {:?} ms, last {:?} ms, peak {:.3} ({:.1} dBFS)",
                first.map(|f| f as f32 * 1000.0 / SAMPLE_RATE as f32),
                last.map(|f| f as f32 * 1000.0 / SAMPLE_RATE as f32),
                peak,
                20.0 * peak.log10()
            );
            assert!(peak > 0.01, "silent");
            assert!(last.unwrap() < SAMPLE_RATE as usize * 4 - 1, "never ended");
            if let Some(dir) = std::env::var_os("BT3_SE_WAV") {
                let pcm: Vec<i16> = out.iter().map(|s| (s * 32767.0) as i16).collect();
                let wav = bt3_data::adx::to_wav(SAMPLE_RATE, 2, &pcm);
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("se_{mask}_{id:02x}.wav")),
                    wav,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn gaussian_table_weights_sum_to_almost_0x8000() {
        for i in 0..256 {
            let sum = i32::from(GAUSS[i])
                + i32::from(GAUSS[0xFF - i])
                + i32::from(GAUSS[0x100 + i])
                + i32::from(GAUSS[0x1FF - i]);
            assert!((0x7F7F..=0x7F81).contains(&sum), "phase {i}: {sum:#X}");
        }
    }

    #[test]
    fn a_16_khz_sample_plays_at_pitch_0x555() {
        assert_eq!(voice_pitch(16000, 0), 0x555);
        assert_eq!(voice_pitch(48000, 0), 0x1000);
    }

    #[test]
    fn curves_and_centre_pan() {
        assert_eq!(velocity_curve(0, 62), 62);
        assert_eq!(velocity_curve(8, 62), 30);
        assert_eq!(pan_levels(64, true, 64, 64), (128, 128, true));
        assert_eq!(pan_levels(64, true, 64, 64 + 63).0, 0);
    }

    #[test]
    fn a_one_shot_block_plays_then_mutes() {
        // Filter 0, shift 0: nibble 7 = 0x7000. Flags 1 = end + mute.
        let mut ram = vec![0x00, 0x01];
        ram.extend([0x77; 14]);
        let mut voice = SpuVoice {
            adsr1: 0x80FF,
            adsr2: 0x5FC0,
            pitch: 0x1000,
            volume: [0x7FFE, 0x7FFE],
            ..SpuVoice::default()
        };
        voice.key_on(0);
        let out: Vec<i32> = (0..40).map(|_| voice.sample(&ram).0).collect();
        assert!(out[10] > 0x5000, "{out:?}");
        assert!(!voice.is_on());
        assert_eq!(out[39], 0);
    }
}
