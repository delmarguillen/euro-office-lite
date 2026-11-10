// Reads the names at the root of a CFB's directory the way x2t reads them
// (POLE, core/Common/3dParty/pole/pole.cpp at v10.0.0.169), to tell an OOXML
// document with a password from any other CFB, and a CFB whose directory can
// be read whole from one whose directory cannot.
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

// x2t takes a CFB for an OOXML document with a password when "EncryptionInfo"
// is at its root (isMS_OFFICECRYPTOFormatFile, OfficeFileFormatChecker2.cpp:651),
// and decrypts the "EncryptedPackage" stream it finds next to it
// (OfficeCryptReader/source/ECMACryptFile.cpp:980). It names both from the
// root, and POLE compares names exactly, in their case (DirEntry::compare).
const ENCRYPTION_INFO: &str = "EncryptionInfo";
const ENCRYPTED_PACKAGE: &str = "EncryptedPackage";

// A CFB whose directory is longer than this cannot be read whole.
const MAX_DIRECTORY_ENTRIES: usize = 1 << 16;
// DIFAT sectors followed at most to find a FAT sector: enough for a FAT of
// 109 + 1024 * 127 sectors at 512 bytes, a file of about 8 GiB.
const MAX_DIFAT_SECTORS: usize = 1024;

const SIGNATURE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const HEADER_LEN: usize = 512;
const HEADER_FAT_SECTORS: u64 = 109;
const ENTRY_LEN: u64 = 128;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
// Sector numbers from here up are not sectors of the file (MAXREGSECT + 1).
const FIRST_SPECIAL_SECTOR: u32 = 0xFFFF_FFFA;

#[derive(Debug, PartialEq)]
pub enum CfbDirectory {
    // The file does not start with the CFB signature.
    NotCfb,
    // Every sector of the header, the FAT, the DIFAT and the directory read to
    // find the names at the root is in the file.
    Readable { holds_encrypted_package: bool },
    // The file starts with the CFB signature, but its header is shorter than
    // 512 bytes or has a sector size POLE refuses, one of those sectors is not
    // whole in the file, or the directory or the DIFAT is past its limit.
    Unreadable,
}

// Whether the file is a CFB, and whether its root holds both entries x2t
// decrypts an OOXML package from.
pub fn read(path: &std::path::Path) -> CfbDirectory {
    let Ok(file) = std::fs::File::open(path) else {
        return CfbDirectory::Unreadable;
    };
    read_from(std::io::BufReader::new(file))
}

fn read_from<R: Read + Seek>(mut reader: R) -> CfbDirectory {
    let mut header = Vec::with_capacity(HEADER_LEN);
    if reader.by_ref().take(HEADER_LEN as u64).read_to_end(&mut header).is_err() {
        return CfbDirectory::Unreadable;
    }
    if !header.starts_with(SIGNATURE) {
        return CfbDirectory::NotCfb;
    }
    let Ok(names) = root_names(reader, &header) else {
        return CfbDirectory::Unreadable;
    };
    let has = |wanted: &str| names.iter().any(|name| name == wanted);
    CfbDirectory::Readable { holds_encrypted_package: has(ENCRYPTION_INFO) && has(ENCRYPTED_PACKAGE) }
}

// A sector, or the header, that cannot be read whole, or a limit passed.
struct Unreadable;

fn root_names<R: Read + Seek>(reader: R, header: &[u8]) -> Result<Vec<String>, Unreadable> {
    let mut cfb = Cfb::open(reader, header)?;
    let chain = cfb.directory_chain()?;
    cfb.names_under_root(&chain)
}

struct Cfb<R> {
    reader: R,
    file_len: u64,
    sector_shift: u32,
    // The number of FAT sectors the header declares: POLE reads no FAT entry
    // past them.
    fat_sectors: u64,
    directory_start: u32,
    header_difat: Vec<u32>,
    first_difat_sector: u32,
    // The DIFAT sectors found so far, in chain order.
    difat_chain: Vec<u32>,
}

struct Entry {
    name: String,
    valid: bool,
    left: u32,
    right: u32,
    child: u32,
}

impl<R: Read + Seek> Cfb<R> {
    fn open(mut reader: R, header: &[u8]) -> Result<Cfb<R>, Unreadable> {
        if header.len() < HEADER_LEN {
            return Err(Unreadable);
        }
        // POLE refuses a sector size outside these (Header::valid).
        let sector_shift = u16_at(header, 0x1E) as u32;
        if !(7..=30).contains(&sector_shift) {
            return Err(Unreadable);
        }
        let file_len = reader.seek(SeekFrom::End(0)).map_err(|_| Unreadable)?;
        let header_difat = (0..HEADER_FAT_SECTORS as usize)
            .map(|i| u32_at(header, 0x4C + i * 4))
            .collect();
        Ok(Cfb {
            reader,
            file_len,
            sector_shift,
            fat_sectors: u32_at(header, 0x2C) as u64,
            directory_start: u32_at(header, 0x30),
            header_difat,
            first_difat_sector: u32_at(header, 0x44),
            difat_chain: Vec::new(),
        })
    }

    // Where a sector starts, when the file holds all of it.
    fn sector_offset(&self, sector: u32) -> Result<u64, Unreadable> {
        let sector_len = 1u64 << self.sector_shift;
        let offset = (sector as u64 + 1) << self.sector_shift;
        if offset + sector_len > self.file_len {
            return Err(Unreadable);
        }
        Ok(offset)
    }

    fn u32s_per_sector(&self) -> u64 {
        1 << (self.sector_shift - 2)
    }

    fn entries_per_sector(&self) -> usize {
        1 << (self.sector_shift - 7)
    }

    fn read_bytes(&mut self, offset: u64, bytes: &mut [u8]) -> Result<(), Unreadable> {
        self.reader.seek(SeekFrom::Start(offset)).map_err(|_| Unreadable)?;
        self.reader.read_exact(bytes).map_err(|_| Unreadable)
    }

    fn read_u32(&mut self, offset: u64) -> Result<u32, Unreadable> {
        let mut bytes = [0u8; 4];
        self.read_bytes(offset, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    // Whether the FAT has an entry for this sector.
    fn in_fat(&self, sector: u32) -> bool {
        sector < FIRST_SPECIAL_SECTOR && (sector as u64) < self.fat_sectors * self.u32s_per_sector()
    }

    // The sectors of the directory, in order, as AllocTable::follow reads them
    // (pole.cpp:533-560): up to the end of the chain, a sector the FAT has no
    // entry for, or a sector already in it.
    fn directory_chain(&mut self) -> Result<Vec<u32>, Unreadable> {
        let max_sectors = MAX_DIRECTORY_ENTRIES / self.entries_per_sector();
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut sector = self.directory_start;
        while self.in_fat(sector) && seen.insert(sector) {
            if chain.len() == max_sectors {
                return Err(Unreadable);
            }
            chain.push(sector);
            sector = self.next_sector(sector)?;
        }
        // POLE reads every sector of the chain (StorageIO::loadBigBlocks).
        for &sector in &chain {
            self.sector_offset(sector)?;
        }
        Ok(chain)
    }

    fn next_sector(&mut self, sector: u32) -> Result<u32, Unreadable> {
        let per_sector = self.u32s_per_sector();
        let fat_sector = self.fat_sector(sector as u64 / per_sector)?;
        let offset = self.sector_offset(fat_sector)? + (sector as u64 % per_sector) * 4;
        self.read_u32(offset)
    }

    // The sector that holds FAT sector `index`: one of the first 109 is named
    // in the header, the others in the DIFAT chain.
    fn fat_sector(&mut self, index: u64) -> Result<u32, Unreadable> {
        if index < HEADER_FAT_SECTORS {
            return Ok(self.header_difat[index as usize]);
        }
        let per_difat_sector = self.u32s_per_sector() - 1;
        let index = index - HEADER_FAT_SECTORS;
        let difat_sector = self.difat_sector((index / per_difat_sector) as usize)?;
        let offset = self.sector_offset(difat_sector)? + (index % per_difat_sector) * 4;
        self.read_u32(offset)
    }

    // The `position`th sector of the DIFAT chain; each one ends with the
    // number of the next.
    fn difat_sector(&mut self, position: usize) -> Result<u32, Unreadable> {
        if position >= MAX_DIFAT_SECTORS {
            return Err(Unreadable);
        }
        while self.difat_chain.len() <= position {
            let next = match self.difat_chain.last() {
                None => self.first_difat_sector,
                Some(&last) => {
                    let offset = self.sector_offset(last)? + (self.u32s_per_sector() - 1) * 4;
                    self.read_u32(offset)?
                }
            };
            self.difat_chain.push(next);
        }
        Ok(self.difat_chain[position])
    }

    fn entry(&mut self, chain: &[u32], index: usize) -> Result<Entry, Unreadable> {
        let per_sector = self.entries_per_sector();
        let sector = chain[index / per_sector];
        let offset = self.sector_offset(sector)? + (index % per_sector) as u64 * ENTRY_LEN;
        let mut bytes = [0u8; ENTRY_LEN as usize];
        self.read_bytes(offset, &mut bytes)?;
        Ok(parse_entry(&bytes))
    }

    // The names of the root's children: every entry reachable through the
    // siblings of the root's child, which holds every one POLE's search from
    // the root can reach (dirtree_find_sibling). An invalid entry ends a
    // branch, as it ends POLE's search.
    fn names_under_root(&mut self, chain: &[u32]) -> Result<Vec<String>, Unreadable> {
        let count = chain.len() * self.entries_per_sector();
        let in_directory = |index: u32| (index as usize) < count;
        let mut names = Vec::new();
        if chain.is_empty() {
            return Ok(names);
        }
        let root = self.entry(chain, 0)?;
        if !root.valid || !in_directory(root.child) {
            return Ok(names);
        }
        let mut seen = HashSet::new();
        let mut pending = vec![root.child];
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            let entry = self.entry(chain, index as usize)?;
            if !entry.valid {
                continue;
            }
            names.push(entry.name);
            for sibling in [entry.left, entry.right] {
                if sibling > 0 && in_directory(sibling) {
                    pending.push(sibling);
                }
            }
        }
        Ok(names)
    }
}

// As DirTree::load reads an entry: at most 32 UTF-16 units of name, without
// the terminating one, and without a first unit below 32.
fn parse_entry(bytes: &[u8]) -> Entry {
    let name_len = (u16_at(bytes, 0x40) as usize).min(64);
    let units_len = if name_len > 1 { name_len / 2 - 1 } else { 0 };
    let mut units: Vec<u16> = (0..units_len).map(|i| u16_at(bytes, i * 2)).collect();
    if units.first().is_some_and(|&first| first < 32) {
        units.remove(0);
    }
    // 1 storage, 2 stream, 5 root.
    let object_type = bytes[0x42];
    Entry {
        name: String::from_utf16_lossy(&units),
        valid: matches!(object_type, 1 | 2 | 5) && name_len >= 1,
        left: u32_at(bytes, 0x44),
        right: u32_at(bytes, 0x48),
        child: u32_at(bytes, 0x4C),
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(word)
}

#[cfg(test)]
pub mod test_cfb {
    // A CFB of 512-byte sectors: the header, one FAT sector, then the
    // directory. Each entry is (name, object type, left, right, child).
    pub type RawEntry<'a> = (&'a str, u8, u32, u32, u32);

    pub const NONE: u32 = 0xFFFF_FFFF;
    pub const STORAGE: u8 = 1;
    pub const STREAM: u8 = 2;
    const ROOT: u8 = 5;

    pub fn with_entries(entries: &[RawEntry]) -> Vec<u8> {
        let directory_sectors = entries.len().div_ceil(4) as u32;
        let mut bytes = vec![0u8; 512];
        bytes[..8].copy_from_slice(super::SIGNATURE);
        bytes[0x18..0x1A].copy_from_slice(&0x3Eu16.to_le_bytes());
        bytes[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        bytes[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        bytes[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        bytes[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        bytes[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x38..0x3C].copy_from_slice(&4096u32.to_le_bytes());
        bytes[0x3C..0x40].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        for i in 0..109 {
            let fat_sector = if i == 0 { 0 } else { NONE };
            bytes[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&fat_sector.to_le_bytes());
        }

        let mut fat = vec![NONE; 128];
        fat[0] = 0xFFFF_FFFD;
        for sector in 1..=directory_sectors {
            fat[sector as usize] = if sector == directory_sectors { 0xFFFF_FFFE } else { sector + 1 };
        }
        bytes.extend(fat.iter().flat_map(|entry| entry.to_le_bytes()));

        let mut directory = vec![0u8; directory_sectors as usize * 512];
        for (i, entry) in entries.iter().enumerate() {
            directory[i * 128..(i + 1) * 128].copy_from_slice(&raw_entry(entry));
        }
        bytes.extend(directory);
        bytes
    }

    pub fn header_only() -> Vec<u8> {
        with_root_streams(&["EncryptionInfo", "EncryptedPackage"])[..512].to_vec()
    }

    // A directory that starts at sector 50, past the end of the file.
    pub fn directory_past_the_end() -> Vec<u8> {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        bytes[0x30..0x34].copy_from_slice(&50u32.to_le_bytes());
        bytes
    }

    // A directory in a sector whose FAT sector is named in DIFAT sector
    // `position`. Sector 1 is the only DIFAT sector, and names itself as the
    // next, so the DIFAT chain never ends.
    pub fn directory_behind_difat_sector(position: u32) -> Vec<u8> {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        let fat_index = 109 + position * 127;
        bytes[0x2C..0x30].copy_from_slice(&(fat_index + 1).to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&(fat_index * 128).to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x48..0x4C].copy_from_slice(&1u32.to_le_bytes());
        let difat = 1024..1536;
        bytes[difat.clone()].fill(0);
        bytes[difat.end - 4..difat.end].copy_from_slice(&1u32.to_le_bytes());
        bytes
    }

    // 4096-byte sectors: the header sector, three FAT sectors, and a directory
    // chain of `sectors` sectors from sector 3, all of them in the file, whose
    // first entry is not a valid root.
    pub fn directory_of_sectors(sectors: u32) -> Vec<u8> {
        let mut bytes = with_root_streams(&[])[..512].to_vec();
        bytes[0x1E..0x20].copy_from_slice(&12u16.to_le_bytes());
        bytes[0x2C..0x30].copy_from_slice(&3u32.to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&3u32.to_le_bytes());
        bytes[0x50..0x54].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x54..0x58].copy_from_slice(&2u32.to_le_bytes());
        bytes.resize(4096, 0);
        let mut fat = vec![NONE; 3 * 1024];
        fat[..3].fill(0xFFFF_FFFD);
        let last = 2 + sectors;
        for sector in 3..=last {
            fat[sector as usize] = if sector == last { 0xFFFF_FFFE } else { sector + 1 };
        }
        bytes.extend(fat.iter().flat_map(|entry| entry.to_le_bytes()));
        bytes.resize(bytes.len() + sectors as usize * 4096, 0);
        bytes
    }

    // A root whose children are streams with these names, each the right
    // sibling of the one before.
    pub fn with_root_streams(names: &[&str]) -> Vec<u8> {
        let first_child = if names.is_empty() { NONE } else { 1 };
        let mut entries: Vec<RawEntry> = vec![("Root Entry", ROOT, NONE, NONE, first_child)];
        for (i, name) in names.iter().enumerate() {
            let right = if i + 1 < names.len() { i as u32 + 2 } else { NONE };
            entries.push((name, STREAM, NONE, right, NONE));
        }
        with_entries(&entries)
    }

    fn raw_entry(&(name, object_type, left, right, child): &RawEntry) -> [u8; 128] {
        let mut entry = [0u8; 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (i, unit) in units.iter().enumerate() {
            entry[i * 2..i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        entry[0x40..0x42].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        entry[0x42] = object_type;
        entry[0x43] = 1;
        entry[0x44..0x48].copy_from_slice(&left.to_le_bytes());
        entry[0x48..0x4C].copy_from_slice(&right.to_le_bytes());
        entry[0x4C..0x50].copy_from_slice(&child.to_le_bytes());
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::test_cfb::{
        directory_behind_difat_sector, directory_of_sectors, directory_past_the_end, header_only,
        with_entries, with_root_streams, NONE, STORAGE, STREAM,
    };
    use super::*;

    const ENCRYPTED_DOCX: &[u8] = include_bytes!("../../tests/fixtures/encrypted-test.docx");
    const WORD97_DOC: &[u8] = include_bytes!("../../tests/fixtures/word97.doc");

    fn read_bytes(bytes: &[u8]) -> CfbDirectory {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "eo-cfb-directory-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        let directory = read(&path);
        let _ = std::fs::remove_file(&path);
        directory
    }

    fn holds(bytes: &[u8]) -> bool {
        read_bytes(bytes) == CfbDirectory::Readable { holds_encrypted_package: true }
    }

    #[test]
    fn a_real_document_with_a_password_holds_an_encrypted_package() {
        assert!(holds(ENCRYPTED_DOCX));
    }

    #[test]
    fn a_real_word_97_doc_does_not() {
        assert!(!holds(WORD97_DOC));
    }

    #[test]
    fn both_streams_at_the_root_are_found_in_any_order() {
        assert!(holds(&with_root_streams(&["EncryptionInfo", "EncryptedPackage"])));
        assert!(holds(&with_root_streams(&["\u{6}DataSpaces", "EncryptedPackage", "EncryptionInfo"])));
    }

    #[test]
    fn one_of_the_two_streams_alone_is_not_enough() {
        assert!(!holds(&with_root_streams(&["EncryptionInfo"])));
        assert!(!holds(&with_root_streams(&["EncryptedPackage"])));
        assert!(!holds(&with_root_streams(&["WordDocument", "1Table"])));
    }

    #[test]
    fn names_are_compared_in_their_case() {
        assert!(!holds(&with_root_streams(&["encryptioninfo", "EncryptedPackage"])));
        assert!(!holds(&with_root_streams(&["EncryptionInfo", "ENCRYPTEDPACKAGE"])));
    }

    #[test]
    fn a_first_character_below_32_is_not_part_of_the_name() {
        assert!(holds(&with_root_streams(&["\u{5}EncryptionInfo", "EncryptedPackage"])));
    }

    #[test]
    fn the_streams_inside_a_storage_are_not_at_the_root() {
        let nested = with_entries(&[
            ("Root Entry", 5, NONE, NONE, 1),
            ("ObjectPool", STORAGE, NONE, NONE, 2),
            ("EncryptionInfo", STREAM, NONE, 3, NONE),
            ("EncryptedPackage", STREAM, NONE, NONE, NONE),
        ]);
        assert!(!holds(&nested));
    }

    #[test]
    fn an_invalid_entry_ends_its_branch() {
        let behind_invalid = with_entries(&[
            ("Root Entry", 5, NONE, NONE, 1),
            ("EncryptionInfo", STREAM, NONE, 2, NONE),
            ("Unused", 0, NONE, 3, NONE),
            ("EncryptedPackage", STREAM, NONE, NONE, NONE),
        ]);
        assert!(!holds(&behind_invalid));
    }

    #[test]
    fn a_file_without_the_signature_is_not_a_cfb() {
        assert_eq!(read_bytes(b""), CfbDirectory::NotCfb);
        assert_eq!(read_bytes(b"PK\x03\x04"), CfbDirectory::NotCfb);
        assert_eq!(read_bytes(&SIGNATURE[..7]), CfbDirectory::NotCfb);
    }

    #[test]
    fn a_cfb_whose_index_cannot_be_read_whole_is_unreadable() {
        let mut bad_shift = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        bad_shift[0x1E] = 40;
        let unreadable = [
            SIGNATURE.to_vec(),
            header_only(),
            bad_shift,
            directory_past_the_end(),
            directory_behind_difat_sector(MAX_DIFAT_SECTORS as u32),
            directory_of_sectors((MAX_DIRECTORY_ENTRIES / 32) as u32 + 1),
        ];
        for (i, bytes) in unreadable.iter().enumerate() {
            assert_eq!(read_bytes(bytes), CfbDirectory::Unreadable, "case {i}");
        }
    }

    #[test]
    fn a_cfb_whose_last_directory_sector_is_cut_short_is_unreadable() {
        let whole = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        assert_eq!(read_bytes(&whole[..whole.len() - 1]), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_cfb_whose_fat_sector_is_past_the_end_is_unreadable() {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        bytes[0x4C..0x50].copy_from_slice(&40u32.to_le_bytes());
        assert_eq!(read_bytes(&bytes), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_directory_chain_that_ends_normally_is_readable() {
        let mut no_directory = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        no_directory[0x30..0x34].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        assert_eq!(read_bytes(&no_directory), CfbDirectory::Readable { holds_encrypted_package: false });
        let mut outside_the_fat = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        outside_the_fat[0x30..0x34].copy_from_slice(&128u32.to_le_bytes());
        assert_eq!(read_bytes(&outside_the_fat), CfbDirectory::Readable { holds_encrypted_package: false });
    }

    #[test]
    fn every_cfb_under_tests_is_read_whole() {
        let mut pending = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests")];
        let mut cfbs = 0;
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let directory = read(&path);
                if directory == CfbDirectory::NotCfb {
                    continue;
                }
                assert!(matches!(directory, CfbDirectory::Readable { .. }), "{}", path.display());
                cfbs += 1;
            }
        }
        assert!(cfbs >= 2, "{cfbs}");
    }

    #[test]
    fn a_loop_in_the_directory_chain_or_the_siblings_ends() {
        let mut looped = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        // The FAT entry of the only directory sector points back to it.
        looped[512 + 4..512 + 8].copy_from_slice(&1u32.to_le_bytes());
        assert!(holds(&looped));
        let siblings_loop = with_entries(&[
            ("Root Entry", 5, NONE, NONE, 1),
            ("EncryptionInfo", STREAM, NONE, 2, NONE),
            ("EncryptedPackage", STREAM, 1, 1, NONE),
        ]);
        assert!(holds(&siblings_loop));
    }

    #[test]
    fn a_directory_past_the_limit_is_unreadable() {
        let at_limit = (MAX_DIRECTORY_ENTRIES / 32) as u32;
        let readable = CfbDirectory::Readable { holds_encrypted_package: false };
        assert_eq!(read_bytes(&directory_of_sectors(at_limit)), readable);
        assert_eq!(read_bytes(&directory_of_sectors(at_limit + 1)), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_difat_chain_past_the_limit_is_unreadable() {
        let bytes = directory_behind_difat_sector(MAX_DIFAT_SECTORS as u32);
        let mut cfb = Cfb::open(std::io::Cursor::new(bytes.clone()), &bytes[..512]).ok().unwrap();
        assert!(cfb.difat_sector(MAX_DIFAT_SECTORS - 1).is_ok());
        assert!(cfb.difat_sector(MAX_DIFAT_SECTORS).is_err());
    }
}
