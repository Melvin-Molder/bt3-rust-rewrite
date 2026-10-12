//! The survival courses of the Ultimate Battle menus (`SurvSel`, src/menu/survival_select.c): sections 12..14 of
//! the course select's unpacked resource (menu archive 3, section 2). Section 12 holds the three courses
//! (`SurvCourse`, 0xE0 bytes: the rule and fifty indices into section 13), section 13 the opponents (`SurvOpp`,
//! 0x2C bytes: character, costume, CPU level and items) and section 14 the characters a random opponent is drawn
//! from.

use std::io;

use crate::pack_section;

/// `SURV_COURSE_NUM`.
pub const COURSES: usize = 3;
/// `SURV_OPP_NUM`: the opponents of a course.
pub const OPPONENTS: usize = 50;
/// `SURV_RANDOM_NUM`: the entries of the random character list.
pub const RANDOM_CHARAS: usize = 0x66;
/// `SURV_RANDOM`: a value drawn at random.
pub const RANDOM: i32 = 0x3E6;
/// `SURV_NONE`: an empty slot.
pub const NONE: i32 = 0x3E7;

const COURSE_SIZE: usize = 0xE0;
const OPP_SIZE: usize = 0x2C;

/// `SurvCourse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Course {
    pub announcer: i32,
    pub stage_change: i32,
    pub time_limit: i32,
    pub stage: i32,
    pub bgm: i32,
    pub change_allowed: i32,
    /// Indices into the opponents.
    pub opp: [i32; OPPONENTS],
}

/// `SurvOpp`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opponent {
    pub chara: i32,
    pub costume: i32,
    pub cpu_level: i32,
    /// The 0-based id of the AI item (slot 7), or [`NONE`].
    pub item7: i32,
    /// 0-based item ids, [`NONE`] for none.
    pub item: [i32; 7],
}

/// The course select's tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurvivalData {
    pub courses: Vec<Course>,
    pub opponents: Vec<Opponent>,
    pub random_charas: Vec<i32>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn words(bytes: &[u8], count: usize) -> io::Result<Vec<i32>> {
    let bytes = bytes
        .get(..count * 4)
        .ok_or_else(|| invalid("survival table is truncated"))?;
    Ok(bytes
        .chunks_exact(4)
        .map(|w| i32::from_le_bytes(w.try_into().unwrap()))
        .collect())
}

impl SurvivalData {
    /// Reads the tables from the course select's unpacked resource. The opponent section runs up to the next
    /// section, and only the opponents the courses name are read.
    pub fn parse(res: &[u8]) -> io::Result<Self> {
        let courses = (0..COURSES)
            .map(|i| {
                let w = words(
                    pack_section(res, 12)?
                        .get(i * COURSE_SIZE..)
                        .unwrap_or_default(),
                    56,
                )?;
                Ok(Course {
                    announcer: w[0],
                    stage_change: w[1],
                    time_limit: w[2],
                    stage: w[3],
                    bgm: w[4],
                    change_allowed: w[5],
                    opp: w[6..].try_into().unwrap(),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let count = courses
            .iter()
            .flat_map(|c| c.opp)
            .map(|i| usize::try_from(i).map_err(|_| invalid("negative survival opponent")))
            .try_fold(0, |n, i| i.map(|i| n.max(i + 1)))?;
        let section = pack_section(res, 13)?;
        let opponents = (0..count)
            .map(|i| {
                let w = words(section.get(i * OPP_SIZE..).unwrap_or_default(), 11)?;
                Ok(Opponent {
                    chara: w[0],
                    costume: w[1],
                    cpu_level: w[2],
                    item7: w[3],
                    item: w[4..].try_into().unwrap(),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let random_charas = words(pack_section(res, 14)?, RANDOM_CHARAS)?;
        Ok(Self {
            courses,
            opponents,
            random_charas,
        })
    }
}

impl Opponent {
    /// `SurvSel_SetupBattle`'s item set: the ids made 1-based and packed into slots 0.., the AI item in slot 7.
    pub fn items(&self) -> [u16; 8] {
        let mut items = [0; 8];
        let mut n = 0;
        for &id in &self.item {
            if id != NONE {
                items[n] = (id + 1) as u16;
                n += 1;
            }
        }
        if self.item7 != NONE {
            items[7] = (self.item7 + 1) as u16;
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(out: &mut Vec<u8>, words: &[i32]) {
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }

    #[test]
    fn parses_courses_opponents_and_the_random_list() {
        let header = 15 * 4;
        let mut body = Vec::new();
        for c in 0..COURSES as i32 {
            put(&mut body, &[c, 1, 2, RANDOM, 0x18, 0]);
            put(&mut body, &[c; OPPONENTS]);
        }
        let opps = header + body.len();
        put(
            &mut body,
            &[5, 1, 13, NONE, 7, NONE, 2, NONE, NONE, NONE, NONE],
        );
        put(
            &mut body,
            &[RANDOM, 0, 29, 40, NONE, NONE, NONE, NONE, NONE, NONE, NONE],
        );
        put(
            &mut body,
            &[3, 0, 0, NONE, NONE, NONE, NONE, NONE, NONE, NONE, NONE],
        );
        let random = header + body.len();
        put(&mut body, &(0..RANDOM_CHARAS as i32).collect::<Vec<_>>());
        let mut res = vec![0; header];
        res[12 * 4..13 * 4].copy_from_slice(&(header as u32).to_le_bytes());
        res[13 * 4..14 * 4].copy_from_slice(&(opps as u32).to_le_bytes());
        res[14 * 4..15 * 4].copy_from_slice(&(random as u32).to_le_bytes());
        res.extend(body);
        let data = SurvivalData::parse(&res).unwrap();
        assert_eq!(data.courses[2].opp, [2; OPPONENTS]);
        assert_eq!(data.courses[0].stage, RANDOM);
        assert_eq!(data.opponents.len(), 3);
        assert_eq!(data.opponents[0].items(), [8, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(data.opponents[2].chara, 3);
        assert_eq!(data.random_charas[0x65], 0x65);
    }
}
