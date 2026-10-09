//! Elden Ring FMG (text table) reading and the text plan for the mashup.
//!
//! Elden Ring keeps each loaded FMG (version 2, little-endian, 64-bit string
//! offsets) as raw bytes. A string is found by id through the group table and
//! read at `fmg_base + string_offset`. The mod changes text by pointing an
//! id's string offset at its own UTF-16 string.

use crate::generated::{ABILITIES, FINISHERS, FORMS};

pub const HEADER: usize = 0x28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Name,
    Info,
    Caption,
}

#[derive(Debug, Clone, Copy)]
struct Group {
    first_index: u32,
    first_id: i32,
    last_id: i32,
}

/// A validated view of one FMG's bytes.
pub struct Fmg<'a> {
    pub bytes: &'a [u8],
    groups: Vec<Group>,
    pub string_count: u32,
    offsets_at: usize,
}

fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}

fn i64_at(b: &[u8], p: usize) -> i64 {
    i64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}

/// Quick check for the first four bytes of an FMG v2 header.
pub fn looks_like_header(b: &[u8]) -> bool {
    b.len() >= 4 && b[0] == 0 && b[1] == 0 && b[2] == 2 && b[3] == 0
}

impl<'a> Fmg<'a> {
    /// Validate an FMG starting at `bytes[0]`; `bytes` may run past its end.
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER || !looks_like_header(bytes) {
            return None;
        }
        let size = u32_at(bytes, 4) as usize;
        if size < HEADER || size > bytes.len() || size > 64 << 20 {
            return None;
        }
        let b = &bytes[..size];
        if b[8..12] != [1, 0, 0, 0] || u32_at(b, 0x14) != 0xFF || i64_at(b, 0x20) != 0 {
            return None;
        }
        let group_count = u32_at(b, 0x0C) as usize;
        let string_count = u32_at(b, 0x10);
        let offsets_at = i64_at(b, 0x18);
        if group_count == 0 || string_count == 0 || offsets_at != (HEADER + group_count * 16) as i64 {
            return None;
        }
        let offsets_at = offsets_at as usize;
        if offsets_at + string_count as usize * 8 > size {
            return None;
        }
        let mut groups = Vec::with_capacity(group_count);
        let mut next_index = 0u32;
        let mut prev_last = i32::MIN;
        for g in 0..group_count {
            let p = HEADER + g * 16;
            let first_index = u32_at(b, p);
            let first_id = u32_at(b, p + 4) as i32;
            let last_id = u32_at(b, p + 8) as i32;
            if u32_at(b, p + 12) != 0 || first_index != next_index || last_id < first_id || (g > 0 && first_id <= prev_last) {
                return None;
            }
            next_index = first_index.checked_add((last_id - first_id) as u32 + 1)?;
            prev_last = last_id;
            groups.push(Group { first_index, first_id, last_id });
        }
        if next_index != string_count {
            return None;
        }
        // Every non-null original offset must land inside the file, after the offset table.
        let table_end = (offsets_at + string_count as usize * 8) as i64;
        for i in 0..string_count as usize {
            let o = i64_at(b, offsets_at + i * 8);
            if o != 0 && (o < table_end || o >= size as i64) {
                return None;
            }
        }
        Some(Fmg { bytes: b, groups, string_count, offsets_at })
    }

    pub fn index_of(&self, id: i32) -> Option<usize> {
        let g = self.groups.binary_search_by(|g| {
            if id < g.first_id {
                std::cmp::Ordering::Greater
            } else if id > g.last_id {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        g.ok().map(|i| (self.groups[i].first_index as i64 + (id - self.groups[i].first_id) as i64) as usize)
    }

    pub fn contains(&self, id: i32) -> bool {
        self.index_of(id).is_some()
    }

    pub fn max_id(&self) -> i32 {
        self.groups.last().map(|g| g.last_id).unwrap_or(0)
    }

    /// Byte position (from the FMG start) of the 8-byte string offset for index `i`.
    pub fn offset_slot(&self, i: usize) -> usize {
        self.offsets_at + i * 8
    }

    pub fn string_at_index(&self, i: usize) -> Option<String> {
        let o = i64_at(self.bytes, self.offset_slot(i));
        if o <= 0 || o as usize >= self.bytes.len() {
            return None;
        }
        let mut s = Vec::new();
        let mut p = o as usize;
        while p + 1 < self.bytes.len() {
            let c = u16::from_le_bytes([self.bytes[p], self.bytes[p + 1]]);
            if c == 0 {
                return Some(String::from_utf16_lossy(&s));
            }
            s.push(c);
            p += 2;
        }
        None
    }

    pub fn get(&self, id: i32) -> Option<String> {
        self.string_at_index(self.index_of(id)?)
    }

    /// Guess whether this table holds names, one-line info, or multi-line captions.
    pub fn kind(&self) -> Kind {
        let (mut n, mut total, mut newline) = (0usize, 0usize, 0usize);
        for i in 0..self.string_count as usize {
            if let Some(s) = self.string_at_index(i) {
                if s.is_empty() || s == "[ERROR]" {
                    continue;
                }
                n += 1;
                total += s.chars().count();
                if s.contains('\n') {
                    newline += 1;
                }
                if n >= 400 {
                    break;
                }
            }
        }
        if n == 0 {
            return Kind::Name;
        }
        if newline * 10 >= n {
            Kind::Caption
        } else if total / n <= 28 {
            Kind::Name
        } else {
            Kind::Info
        }
    }
}

pub const MISSING_CR: &str = "\n\n[CONTROL Resonant was not found on this PC. Install it (Steam or Epic) to hear the Aberrant's own sounds, or set ELDEN_RESONANT_CR_PATH to its folder.]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Weapons,
    Goods,
    Arts,
}

/// Which of the mashup's tables an FMG is, if any.
pub fn table_of(f: &Fmg) -> Option<Table> {
    let weapons = FORMS.iter().all(|x| f.contains(x.weapon_id as i32));
    let goods = ABILITIES.iter().all(|x| f.contains(x.goods_id as i32));
    let arts = FINISHERS.iter().all(|x| f.contains(x.sword_arts_id));
    if weapons && !goods {
        Some(Table::Weapons)
    } else if goods && !weapons && f.contains(2020) && f.max_id() >= 8000 {
        Some(Table::Goods)
    } else if arts && !weapons && !goods {
        Some(Table::Arts)
    } else {
        None
    }
}

/// The (id, text) pairs to write into an FMG of the given table and kind.
pub fn plan(table: Table, kind: Kind, cr_found: bool) -> Vec<(i32, String)> {
    let missing = if cr_found { "" } else { MISSING_CR };
    match table {
        Table::Weapons => FORMS
            .iter()
            .map(|f| {
                let t = match kind {
                    Kind::Name => f.name.to_string(),
                    Kind::Info => f.info.to_string(),
                    Kind::Caption => format!("{}{}", f.caption, missing),
                };
                (f.weapon_id as i32, t)
            })
            .collect(),
        Table::Goods => ABILITIES
            .iter()
            .map(|a| {
                let t = match kind {
                    Kind::Name => a.name.to_string(),
                    Kind::Info => a.info.to_string(),
                    Kind::Caption => format!("{}{}", a.caption, missing),
                };
                (a.goods_id as i32, t)
            })
            .collect(),
        Table::Arts => FINISHERS
            .iter()
            .map(|s| {
                let t = match kind {
                    Kind::Name => s.name.to_string(),
                    Kind::Info => s.description.to_string(),
                    Kind::Caption => format!("{}\nA combo finisher of CONTROL Resonant's Aberrant.", s.description),
                };
                (s.sword_arts_id, t)
            })
            .collect(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Build an FMG v2 from (id, text) pairs sorted by id.
    pub fn build(entries: &[(i32, &str)]) -> Vec<u8> {
        let mut groups: Vec<(u32, i32, i32)> = Vec::new();
        for (i, (id, _)) in entries.iter().enumerate() {
            match groups.last_mut() {
                Some(g) if g.2 + 1 == *id => g.2 = *id,
                _ => groups.push((i as u32, *id, *id)),
            }
        }
        let offsets_at = HEADER + groups.len() * 16;
        let strings_at = offsets_at + entries.len() * 8;
        let mut strings = Vec::new();
        let mut offs = Vec::new();
        for (_, t) in entries {
            offs.push((strings_at + strings.len()) as i64);
            for c in t.encode_utf16() {
                strings.extend_from_slice(&c.to_le_bytes());
            }
            strings.extend_from_slice(&[0, 0]);
        }
        let size = strings_at + strings.len();
        let mut b = vec![0u8, 0, 2, 0];
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&(groups.len() as u32).to_le_bytes());
        b.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        b.extend_from_slice(&0xFFu32.to_le_bytes());
        b.extend_from_slice(&(offsets_at as i64).to_le_bytes());
        b.extend_from_slice(&0i64.to_le_bytes());
        for (fi, first, last) in &groups {
            b.extend_from_slice(&fi.to_le_bytes());
            b.extend_from_slice(&first.to_le_bytes());
            b.extend_from_slice(&last.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
        }
        for o in offs {
            b.extend_from_slice(&o.to_le_bytes());
        }
        b.extend_from_slice(&strings);
        b
    }

    fn weapon_names() -> Vec<u8> {
        let mut e: Vec<(i32, &str)> = vec![(1000000, "Dagger"), (1000100, "Heavy Dagger")];
        for f in FORMS {
            e.push((f.weapon_id as i32, "Some Weapon"));
        }
        e.sort();
        build(&e)
    }

    #[test]
    fn parses_and_finds() {
        let b = weapon_names();
        let mut padded = b.clone();
        padded.extend_from_slice(&[0xAA; 64]);
        let f = Fmg::parse(&padded).unwrap();
        assert_eq!(f.get(1000000).as_deref(), Some("Dagger"));
        assert_eq!(f.get(1000100).as_deref(), Some("Heavy Dagger"));
        assert!(f.get(1000001).is_none());
        assert_eq!(f.kind(), Kind::Name);
        assert_eq!(table_of(&f), Some(Table::Weapons));
    }

    #[test]
    fn rejects_corrupt() {
        let mut b = weapon_names();
        b[0x14] = 0;
        assert!(Fmg::parse(&b).is_none());
        let mut b = weapon_names();
        b[HEADER + 4] ^= 0xFF; // first group's first id no longer ordered/consistent
        b[HEADER] = 5;
        assert!(Fmg::parse(&b).is_none());
        assert!(Fmg::parse(&[0, 0, 2, 0, 1]).is_none());
    }

    #[test]
    fn classifies_captions_and_info() {
        let cap = build(&[(1, "Line one.\nLine two."), (2, "More\ntext"), (3, "x\ny")]);
        assert_eq!(Fmg::parse(&cap).unwrap().kind(), Kind::Caption);
        let info = build(&[(1, "A fairly long single line of description text here."), (2, "Another long single line of descriptive item text.")]);
        assert_eq!(Fmg::parse(&info).unwrap().kind(), Kind::Info);
    }

    #[test]
    fn plan_covers_every_row() {
        assert_eq!(plan(Table::Weapons, Kind::Name, true).len(), FORMS.len());
        assert_eq!(plan(Table::Goods, Kind::Caption, true).len(), ABILITIES.len());
        assert_eq!(plan(Table::Arts, Kind::Name, true).len(), FINISHERS.len());
        assert!(plan(Table::Weapons, Kind::Caption, false)[0].1.contains("CONTROL Resonant was not found"));
        assert!(!plan(Table::Weapons, Kind::Name, false)[0].1.contains("not found"));
    }

    /// `FIXTURE_DIR=... cargo test write_fmg_fixtures -- --ignored` writes FMGs for the Wine self-test.
    #[test]
    #[ignore]
    fn write_fmg_fixtures() {
        let dir = std::path::PathBuf::from(std::env::var("FIXTURE_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("weapon_names.fmg"), weapon_names()).unwrap();
        let mut caps: Vec<(i32, &str)> = FORMS.iter().map(|f| (f.weapon_id as i32, "Old caption\nsecond line")).collect();
        caps.sort();
        std::fs::write(dir.join("weapon_captions.fmg"), build(&caps)).unwrap();
        let mut goods: Vec<(i32, &str)> = ABILITIES.iter().map(|a| (a.goods_id as i32, "Pot")).collect();
        goods.push((2020, "Rainbow Stone"));
        goods.push((8948, "Bell Bearing"));
        goods.sort();
        std::fs::write(dir.join("goods_names.fmg"), build(&goods)).unwrap();
        std::fs::write(dir.join("classes.fmg"), build(&[(1, "Vagabond"), (2, "Wretch")])).unwrap();
    }
}
