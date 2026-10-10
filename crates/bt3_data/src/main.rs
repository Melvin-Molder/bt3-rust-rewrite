use std::path::PathBuf;

use bt3_data::Iso9660;
use bt3_data::adx;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::cam_anim::CamAnim;
use bt3_data::gs::{decode_sprite_texture, psmt8_row_from_upload};
use bt3_data::model::{CharaModel, TEXTURE_SECTION, tex0_cbp, tex0_tbp};
use bt3_data::pack_section;
use bt3_data::stage::StageResource;
use bt3_data::stage_model::StageModel;
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

/// A `Snd_LoadBankFile` bank: five words, then the sample body, the HD header and the SE sequences.
fn probe_se_bank(image: &mut Iso9660, file_id: u32) -> Result<(), Box<dyn std::error::Error>> {
    let location = partition_entry_for_id(file_id);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    let bank = archive.read_entry(image, location.index)?;
    let word = |o: usize| u32::from_le_bytes(bank[o..o + 4].try_into().unwrap()) as usize;
    let header: Vec<usize> = (0..5).map(|i| word(i * 4)).collect();
    println!(
        "\nSE bank 0x{file_id:X}: {} bytes, header {header:X?}",
        bank.len()
    );
    let parsed = bt3_data::snd::SeBank::parse(&bank)?;
    let mut features = std::collections::BTreeSet::new();
    for set in parsed.timbre_sets.iter().flatten() {
        for timbre in set.iter().flatten() {
            for tone in &timbre.tones {
                features.insert(format!(
                    "curve {:02X} semi {} fine {} group {} pan_mode {} mix {:02X} lfo {:02X} adsr {:04X}/{:04X}",
                    tone.velocity_curve,
                    tone.semitone,
                    tone.fine,
                    tone.exclusive_group,
                    tone.pan_mode,
                    tone.mix,
                    tone.lfo_mode,
                    tone.adsr1,
                    tone.adsr2
                ));
            }
        }
    }
    let rates: std::collections::BTreeSet<_> = parsed.vags.iter().map(|v| v.sample_rate).collect();
    let mut opcodes = std::collections::BTreeSet::new();
    for set in parsed.sesq.sets.iter().flatten() {
        for seq in set.sequences.iter().flatten() {
            opcodes.insert(format!("{:02X?}", &seq.data[..seq.data.len().min(12)]));
        }
    }
    println!("  {} vags, rates {rates:?}; tone kinds:", parsed.vags.len());
    for feature in &features {
        println!("    {feature}");
    }
    println!("  sequence heads: {opcodes:?}");
    for (name, start, end) in [
        ("body", header[1], header[2]),
        ("hd", header[2], header[3]),
        ("seq", header[3], header[4]),
    ] {
        println!("  {name} 0x{start:X}..0x{end:X} ({} bytes)", end - start);
        if name == "body" {
            println!("    {:02X?}", &bank[start..start + 32]);
            continue;
        }
        let dump_all = std::env::var_os("BT3_SE_DUMP").is_some();
        let mut ofs = start;
        while ofs + 16 <= end.min(if dump_all { end } else { start + 0x100 }) {
            let row = &bank[ofs..ofs + 16];
            let text: String = row
                .iter()
                .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
                .collect();
            println!("    +{:04X} {row:02X?} {text}", ofs - start);
            ofs += 16;
        }
    }
    Ok(())
}

/// `Font_InitEx`: the two fonts are at byte offsets boot[1] and boot[2] of common file 5.
fn probe_fonts(image: &mut Iso9660) -> Result<(), Box<dyn std::error::Error>> {
    let location = partition_entry_for_id(5);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    let boot = archive.read_entry(image, location.index)?;
    let word = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let header: Vec<usize> = (0..10).map(|i| word(&boot, i * 4)).collect();
    println!("\nBoot file 5: {} bytes, header {header:X?}", boot.len());
    for slot in 1..=2 {
        let font = &boot[header[slot]..];
        let tex = word(font, 0xC);
        let glyphs = word(font, 0x14);
        let count = word(font, 0x18);
        println!(
            "  font {slot}: head {:02X?} tex +{tex:X} glyphs +{glyphs:X} count {count} unk1C {:X} line height {}",
            &font[..0x24],
            word(font, 0x1C),
            word(font, 0x20)
        );
        let t = &font[tex..tex + 0x40];
        let tex0 = u64::from_le_bytes(t[0x30..0x38].try_into().unwrap());
        println!(
            "    tex: pixels +{:X} clut +{:X} sizes {:X}/{:X} blt {:08X}/{:08X} tex0 {tex0:016X} (psm {:X} tw {} th {} tbw {})",
            word(t, 0),
            word(t, 4),
            word(t, 8),
            word(t, 12),
            word(t, 24),
            word(t, 28),
            (tex0 >> 20) & 0x3F,
            1 << ((tex0 >> 26) & 0xF),
            1 << ((tex0 >> 30) & 0xF),
            (tex0 >> 14) & 0x3F
        );
        let mut cluts = std::collections::BTreeSet::new();
        let mut sizes = std::collections::BTreeSet::new();
        for g in 0..count {
            let e = glyphs + g * 6;
            let (u, v) = (
                u16::from_le_bytes([font[e + 2], font[e + 3]]),
                u16::from_le_bytes([font[e + 4], font[e + 5]]),
            );
            cluts.insert(((v >> 15) << 1) | (u >> 15));
            sizes.insert(((u >> 10) & 0x1F, (v >> 10) & 0x1F));
        }
        let sample: Vec<String> = (0..count.min(6))
            .map(|g| {
                let e = glyphs + g * 6;
                format!(
                    "{:04X}:{:04X}/{:04X}",
                    u16::from_le_bytes([font[e], font[e + 1]]),
                    u16::from_le_bytes([font[e + 2], font[e + 3]]),
                    u16::from_le_bytes([font[e + 4], font[e + 5]])
                )
            })
            .collect();
        println!(
            "    glyph cluts {cluts:?}, {} sizes (first {:?}), first glyphs {sample:?}",
            sizes.len(),
            sizes.iter().next()
        );
    }
    Ok(())
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

    // Dialog_Init(MPACK_AT(res, 12)): parts 1-4 textures, 5 the window movie, 6 the message table.
    let dialog = pack_section(&res, 12)?;
    for part in 1..=4 {
        let textures = TextureResource::parse(pack_section(dialog, part)?)?;
        let summary: Vec<String> = textures
            .entries
            .iter()
            .map(|t| {
                format!(
                    "pix {:X} clut {:X} blt {:08X}/{:08X} tex0 {:016X} {}",
                    t.pixel_size,
                    t.clut_size,
                    t.pixel_blt,
                    t.clut_blt,
                    t.texture_register,
                    match decode_sprite_texture(t) {
                        Ok(rgba) => format!("{}x{}", rgba.width, rgba.height),
                        Err(error) => error.to_string(),
                    }
                )
            })
            .collect();
        println!("  dialog part {part}: {summary:?}");
    }
    let window = Fod::parse(pack_section(dialog, 5)?)?;
    let images: Vec<String> = (0..window.record_count(3)?)
        .map(|i| {
            let rec = window.record(3, i).unwrap();
            let data = window.bytes();
            format!(
                "{}x{}",
                fod::u16_at(data, rec + 8).unwrap(),
                fod::u16_at(data, rec + 10).unwrap()
            )
        })
        .collect();
    println!("  dialog movie images {images:?}");
    let table = pack_section(dialog, 6)?;
    let count = u32::from_le_bytes(table[0..4].try_into()?) as usize;
    println!("  dialog messages: {count}");
    for i in 0..count.min(0x40) {
        let ofs = (u32::from_le_bytes(table[4 + i * 4..8 + i * 4].try_into()?) as usize) & !3;
        let text: String = table[ofs..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .map(|c| char::from_u32(u32::from(c)).unwrap_or('?'))
            .collect();
        println!("    {i:02X}: {:?}", text);
    }
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

fn describe_textures(data: &[u8]) -> String {
    match TextureResource::parse(data) {
        Ok(res) => res
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
                        "/clut-only".to_owned()
                    } else {
                        match decode_sprite_texture(t) {
                            Ok(_) => String::new(),
                            Err(error) => format!("/{error}"),
                        }
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" "),
        Err(error) => format!("not textures: {error}"),
    }
}

fn describe_fod(data: &[u8]) -> String {
    use bt3_data::fod::{Fod, FrameTag};

    let Ok(movie) = Fod::parse(data) else {
        return "not a FOD".to_owned();
    };
    let bytes = movie.bytes();
    let images: Vec<String> = (0..movie.record_count(3).unwrap_or(0))
        .map(|i| {
            let rec = movie.record(3, i).unwrap();
            format!(
                "{}x{}",
                bt3_data::fod::u16_at(bytes, rec + 8).unwrap(),
                bt3_data::fod::u16_at(bytes, rec + 10).unwrap()
            )
        })
        .collect();
    let mut labels = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut timelines = vec![movie.record(7, 0).unwrap()];
    for index in 0..movie.record_count(6).unwrap_or(0) {
        timelines.push(movie.record(6, index).unwrap());
    }
    for rec in timelines {
        let mut block = rec + 8;
        while let Ok(Some(frame)) = bt3_data::fod::parse_frame(bytes, block) {
            if !frame.label.is_empty() {
                labels.insert(String::from_utf8_lossy(&frame.label).into_owned());
            }
            for tag in &frame.tags {
                if let FrameTag::PlaceNew { name, .. } = tag
                    && !name.is_empty()
                {
                    names.insert(String::from_utf8_lossy(name).into_owned());
                }
            }
            match bt3_data::fod::skip_named(bytes, block, 1) {
                Ok(Some(next)) => block = next,
                _ => break,
            }
        }
    }
    format!("images {images:?}\n      labels {labels:?}\n      clips {names:?}")
}

fn utf16_line(table: &[u8], i: usize) -> String {
    bt3_data::font::message(table, i)
        .map(|t| String::from_utf16_lossy(&t))
        .unwrap_or_else(|e| e.to_string())
}

/// Sections of an unpacked menu screen pack and how to read them.
struct MenuSections<'a> {
    fods: &'a [usize],
    packs: &'a [usize],
    lips: &'a [usize],
    texts: &'a [usize],
}

/// A menu screen's `*_Init(section)`: section `section` of archive `file_id`, BPE-packed.
fn probe_menu(
    image: &mut Iso9660,
    name: &str,
    file_id: u32,
    section: usize,
    kinds: MenuSections<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let location = partition_entry_for_id(file_id);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    let pack = archive.read_entry(image, location.index)?;
    let res = bpe::decode(pack_section(&pack, section)?)?;
    let count = u32::from_le_bytes(res[0..4].try_into()?) as usize;
    println!(
        "\n{name} (file 0x{file_id:X} section {section}): {} bytes, {count} sections",
        res.len()
    );
    for section in 1..=count {
        let data = pack_section(&res, section)?;
        if kinds.fods.contains(&section) {
            println!("  {section}: FOD {}", describe_fod(data));
        } else if kinds.packs.contains(&section) {
            let parts = u32::from_le_bytes(data[0..4].try_into()?) as usize;
            for part in 1..=parts {
                let part_data = pack_section(data, part)?;
                let fod = describe_fod(part_data);
                if fod != "not a FOD" {
                    println!("  {section}.{part}: FOD {fod}");
                } else {
                    println!("  {section}.{part}: {}", describe_textures(part_data));
                }
            }
        } else if kinds.lips.contains(&section) {
            let words: Vec<u32> = data[..0x40.min(data.len())]
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            println!("  {section}: {} bytes, head {words:X?}", data.len());
        } else if kinds.texts.contains(&section) {
            let n = bt3_data::font::message_count(data)?;
            println!("  {section}: text, {n} lines");
            for i in 0..n {
                println!("    {i:02X}: {:?}", utf16_line(data, i));
            }
        } else {
            println!("  {section}: {}", describe_textures(data));
        }
    }
    Ok(())
}

fn probe_main_menu(image: &mut Iso9660) -> Result<(), Box<dyn std::error::Error>> {
    probe_menu(
        image,
        "Main menu",
        0x1C1 + 1,
        2,
        MenuSections {
            fods: &[20],
            packs: &[21, 22],
            lips: &[27],
            texts: &[28],
        },
    )?;
    // DuelMenu_Run(1): section 1 of archive 5 (file baseFile).
    probe_menu(
        image,
        "Duel menu",
        0x1C1,
        1,
        MenuSections {
            fods: &[19],
            packs: &[22, 23, 25],
            lips: &[26],
            texts: &[21, 27],
        },
    )?;
    // CharSel_Run(2): section 2 of archive 5.
    probe_chara_models(image)?;
    for (what, file_id) in [
        ("portrait", 0x2F9u32),
        ("portrait", 0x2F9 + 0x10),
        ("stage", 0x39D),
        ("stage", 0x39D + 5),
    ] {
        let location = partition_entry_for_id(file_id);
        let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
        let data = archive.read_entry(image, location.index)?;
        let unpacked = bpe::decode(&data)?;
        println!(
            "\n{what} file 0x{file_id:X}: {} packed, {} unpacked: {}",
            data.len(),
            unpacked.len(),
            describe_textures(&unpacked)
        );
    }
    let location = partition_entry_for_id(4);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    let table = archive.read_entry(image, location.index)?;
    let words: Vec<u32> = table[..16]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let chara = (words[1] & !3) as usize;
    let costumes: Vec<u16> = (0..0xA1)
        .map(|i| u16::from_le_bytes([table[chara + i * 0x3C + 0xA], table[chara + i * 0x3C + 0xB]]))
        .collect();
    println!(
        "\ncommon file 4: {} bytes, head {words:X?}, costumes {costumes:?}",
        table.len()
    );
    probe_menu(
        image,
        "Character select",
        0x1C1,
        2,
        MenuSections {
            fods: &[15, 24, 25, 32, 33],
            packs: &[51, 53, 54],
            lips: &[39, 42, 56],
            texts: &[43, 44],
        },
    )
}

/// Character model files (`0x590 + chara * 10 + costume`): the parts and batches `CharaModel` reads, and whether
/// every TEX0 of the parts finds its texture entry (`tbpOfs` / `cbpOfs` after `MdlTex_RebaseChain`) and every
/// shading TEX0 its CLUT in the `clutFirst` range.
fn probe_chara_models(image: &mut Iso9660) -> Result<(), Box<dyn std::error::Error>> {
    let archive = AfsArchive::open(image, "DATA/PZS3US1.AFS")?;
    for chara in [0u32, 1, 10, 50, 100, 160] {
        let file_id = 0x590 + chara * 10;
        let data = archive.read_entry(image, partition_entry_for_id(file_id).index)?;
        let model = match CharaModel::from_file(&data) {
            Ok(model) => model,
            Err(error) => {
                println!("\nmodel file 0x{file_id:X}: {error}");
                continue;
            }
        };
        let textures = TextureResource::parse(pack_section(&data, TEXTURE_SECTION)?)?;
        let h = &model.header;
        let batches: Vec<_> = model.parts.iter().flat_map(|p| &p.batches).collect();
        let vertices: usize = batches.iter().map(|b| b.vertices.len()).sum();
        let mut missing = 0;
        for batch in &batches {
            let (tbp, cbp) = (
                tex0_tbp(batch.tex0[0]) - model.min_tbp,
                tex0_cbp(batch.tex0[0]) - model.min_cbp,
            );
            let base = (h.tex_first..h.tex_first + h.tex_count).any(|i| {
                let e = &textures.entries[usize::from(i)];
                e.tbp_offset as u32 == tbp && e.cbp_offset as u32 == cbp
            });
            let shade = (h.clut_first..h.clut_first + h.clut_count).any(|i| {
                textures.entries[usize::from(i)].tbp_offset as u32 == tex0_cbp(batch.tex0[1])
            });
            missing += usize::from(!base) + usize::from(!shade);
        }
        println!(
            "\nmodel file 0x{file_id:X}: size {:?}, tex {}+{}, clut {}+{}, face {}, seams +{:X}; {} parts ({} enabled), {} batches, {vertices} vertices, {missing} unmatched TEX0",
            h.size,
            h.tex_first,
            h.tex_count,
            h.clut_first,
            h.clut_count,
            h.face_tex,
            h.seam_offset,
            model.parts.len(),
            model.parts.iter().filter(|p| p.enabled).count(),
            batches.len(),
        );
        if chara == 0 {
            for p in &model.parts {
                println!(
                    "    node {:3} pop {} en {} origin {:?} b {:?} rest {:?}",
                    p.node, p.pop, p.enabled, p.offset_a, p.offset_b, p.rest
                );
            }
        }
    }
    // BtlObjLight_Init: section 1 of common file 2 is the light's texture file; texture 0 is the ramp that
    // BtlObjLight_FindRes0 uploads to block 0x3D40.
    let common = archive.read_entry(image, partition_entry_for_id(2).index)?;
    let light = TextureResource::parse(pack_section(&common, 1)?)?;
    let ramp = psmt8_row_from_upload(&light.entries[0], 4, 0, 256)?;
    println!("  light ramp indices: {ramp:?}");
    // BtlChar_AllocAll: header word 3 of common file 2 is the motion flag table (one word per motion id).
    let word = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let flags_at = word(&common, 12) as usize & !3;
    println!(
        "  motion flags 0 {:08X}, 1 {:08X}, 0x26 {:08X}",
        word(&common, flags_at),
        word(&common, flags_at + 4),
        word(&common, flags_at + 0x26 * 4)
    );
    // BtlObj_BindTables: anims[i] = section i + 1 of file 0x598 + chara * 10, BPE-packed (BtlObjAnim_Load).
    let motions = archive.read_entry(image, partition_entry_for_id(0x598).index)?;
    println!(
        "  motion file 0x598: {} bytes, {} sections",
        motions.len(),
        word(&motions, 0)
    );
    for id in [0usize, 0x26] {
        let anim = bpe::decode(pack_section(&motions, id + 1)?)?;
        let length = u16::from_le_bytes([anim[2], anim[3]]);
        let tracks: Vec<(usize, u16, u16)> = (0..71)
            .filter_map(|node| {
                let ofs =
                    usize::from(u16::from_le_bytes([anim[6 + node * 2], anim[7 + node * 2]])) * 4;
                (ofs != 0).then(|| {
                    (
                        node,
                        u16::from_le_bytes([anim[ofs], anim[ofs + 1]]),
                        u16::from_le_bytes([anim[ofs + 2], anim[ofs + 3]]),
                    )
                })
            })
            .collect();
        println!(
            "  motion {id:#X}: {} bytes, events {}, length {length}, tracks (node, flags, keys) {tracks:?}",
            anim.len(),
            anim[1]
        );
    }
    Ok(())
}

/// The stage's drawable model (`StageModel`): meshes per draw group, how many `StgModel_Cull` can reach after
/// `BtlStage_Reset`, the vertex extent, and the stage texture file (offsets from the stage object).
fn probe_stage_model(file: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let object = StageModel::object(file)?;
    let model = StageModel::parse(object)?;
    for (g, materials) in model.groups.iter().enumerate() {
        let meshes: Vec<_> = materials.iter().flat_map(|m| &m.meshes).collect();
        let drawn = meshes
            .iter()
            .filter(|m| model.drawable[usize::from(m.vis)])
            .count();
        let strips: usize = meshes.iter().map(|m| m.strips.len()).sum();
        let vertices: Vec<_> = meshes
            .iter()
            .flat_map(|m| m.strips.iter().flatten())
            .collect();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.position[k]);
                hi[k] = hi[k].max(v.position[k]);
            }
        }
        let sample = vertices.first().map(|v| (v.position, v.color, v.stq));
        println!(
            "  group {g}: {} materials, {} meshes ({drawn} drawable), {strips} strips, {} vertices, extent {lo:?}..{hi:?}, first {sample:?}, animated {}",
            materials.len(),
            meshes.len(),
            vertices.len(),
            materials.iter().filter(|m| m.anim.is_some()).count()
        );
    }
    let textures = TextureResource::parse_at(object, model.texture_header)?;
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
                decode_sprite_texture(t)
                    .err()
                    .map_or(String::new(), |e| format!("/{e}"))
            )
        })
        .collect();
    println!(
        "  stage textures ({}): {}",
        formats.len(),
        formats.join(" ")
    );
    Ok(())
}

fn archive_read(image: &mut Iso9660, file_id: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let location = partition_entry_for_id(file_id);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    Ok(archive.read_entry(image, location.index)?)
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
    println!("  object light direction {:?}", stage.light_dir);
    probe_stage_model(&stage_bytes)?;
    // DemoCam_PlayStageAnim(0..2): BtlStage_GetFileMember(idx) = stage file section idx + 3.
    for cut in 0..3 {
        match pack_section(&stage_bytes, cut + 3).map(CamAnim::parse) {
            Ok(Ok(anim)) => println!(
                "  stage cut {cut}: length {}, defaults {:?}, channels {:?}",
                anim.length,
                anim.defaults,
                anim.channels
                    .iter()
                    .map(|c| (
                        c.index,
                        c.keys.len(),
                        c.keys.first().map(|k| (k.time, k.value))
                    ))
                    .collect::<Vec<_>>()
            ),
            other => println!("  stage cut {cut}: {other:?}"),
        }
    }
    {
        // gBattleTimeLimitTbl (0x2C3480, s16[8]) through the ELF's program headers.
        let elf = image.read_file("SLUS_216.78")?;
        let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap());
        let (phoff, phnum) = (
            u32_at(0x1C) as usize,
            u16::from_le_bytes([elf[0x2C], elf[0x2D]]),
        );
        for i in 0..usize::from(phnum) {
            let ph = phoff + i * 32;
            let (offset, vaddr, filesz) = (u32_at(ph + 4), u32_at(ph + 8), u32_at(ph + 16));
            if (vaddr..vaddr + filesz).contains(&0x2C3480) {
                let at = (offset + 0x2C3480 - vaddr) as usize;
                let table: Vec<i16> = (0..8)
                    .map(|k| i16::from_le_bytes([elf[at + k * 2], elf[at + k * 2 + 1]]))
                    .collect();
                println!("\ngBattleTimeLimitTbl: {table:?}");
            }
        }
    }
    {
        // BtlChar_AllocAll: gBtlChars->tbl[4] = table at header word 8 of common file 2 (BtlMemberParam, 16 bytes).
        let common = archive_read(&mut image, 2)?;
        let word = |o: usize| u32::from_le_bytes(common[o..o + 4].try_into().unwrap());
        let table = (word(8 * 4) & !3) as usize;
        let params: Vec<[i32; 4]> = (0..6)
            .map(|c| std::array::from_fn(|i| word(table + c * 16 + i * 4) as i32))
            .collect();
        println!("\nCharacter parameters (health max, ki start, ?, blast stocks): {params:?}");
        // BtlObj_BindTables: mdl->unk38 (object + 0x50, the HUD face) is character file section 1.
        let chara = archive_read(&mut image, 0x590)?;
        let faces = TextureResource::parse(pack_section(&chara, 1)?)?;
        for t in &faces.entries {
            let tex0 = t.texture_register;
            println!(
                "  face {}x{} psm {:#X} decodes {}",
                1 << ((tex0 >> 26) & 0xF),
                1 << ((tex0 >> 30) & 0xF),
                (tex0 >> 20) & 0x3F,
                decode_sprite_texture(t).is_ok()
            );
        }
    }
    {
        // Hud_Init: file 6 + gProgress->unk0[0]; header words 1, 4, 5, 2, 3 are the byte offsets of sheets 0..4.
        let hud = archive_read(&mut image, 6)?;
        let words: Vec<u32> = hud
            .chunks_exact(4)
            .take(8)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            .collect();
        println!("\nHUD file 6: {} bytes, header {words:X?}", hud.len());
        for (sheet, word) in [1usize, 4, 5, 2, 3].into_iter().enumerate() {
            let start = (words[word] & !3) as usize;
            match TextureResource::parse(&hud[start..]) {
                Ok(res) => {
                    let sizes: Vec<String> = res
                        .entries
                        .iter()
                        .map(|t| {
                            let w = 1 << ((t.texture_register >> 26) & 0xF);
                            let h = 1 << ((t.texture_register >> 30) & 0xF);
                            let psm = (t.texture_register >> 20) & 0x3F;
                            let ok = decode_sprite_texture(t).is_ok();
                            format!("{w}x{h}/psm{psm:#X}/{ok}")
                        })
                        .collect();
                    println!("  sheet {sheet} at {start:#X}: {sizes:?}");
                }
                Err(error) => println!("  sheet {sheet} at {start:#X}: {error}"),
            }
        }
    }
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
    for file_id in [0x14B, 0x14C] {
        probe_se_bank(&mut image, file_id)?;
    }
    probe_fonts(&mut image)?;
    probe_main_menu(&mut image)?;

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
