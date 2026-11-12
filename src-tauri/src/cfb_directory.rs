// Reads the entries at the root of a CFB's directory the way x2t reads them
// (POLE, core/Common/3dParty/pole/pole.cpp at v10.0.0.169), to tell an OOXML
// document with a password from any other CFB, a Word, Excel or PowerPoint
// 97-2003 document from any other CFB, and a CFB whose directory can be read
// whole from one whose directory cannot.
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

// x2t takes a CFB for an OOXML document with a password when "EncryptionInfo"
// is at its root (isMS_OFFICECRYPTOFormatFile, OfficeFileFormatChecker2.cpp:651),
// and decrypts the "EncryptedPackage" stream it finds next to it
// (OfficeCryptReader/source/ECMACryptFile.cpp:980). It names both from the
// root, and POLE compares names exactly, in their case (DirEntry::compare).
const ENCRYPTION_INFO: &str = "EncryptionInfo";
const ENCRYPTED_PACKAGE: &str = "EncryptedPackage";

// The streams x2t picks a binary format from, in the order
// COfficeFileFormatChecker::isOfficeFile tries them
// (OfficeFileFormatChecker2.cpp:723-747): isDocFormatFile (:408-443),
// isXlsFormatFile (:485-520) and isPptFormatFile (:544-556). Each one takes
// the file for its format when its stream gives at least one byte.
const WORD_DOCUMENT: &str = "WordDocument";
const WORKBOOK_STREAMS: [&str; 5] = ["Workbook", "Book", "WORKBOOK", "BOOK", "book"];
const POWERPOINT_DOCUMENT: &str = "PowerPoint Document";
// isDocFormatFile does not take a file whose "DataSpaces" storage has entries
// (:421-423).
const DATA_SPACES: &str = "DataSpaces";
// isOleObjectFile runs before them (:717-722). With an "Ole" stream and a
// "CompObj" stream of COMP_OBJ_HEADER bytes or more, it decides the file from
// the names in CompObj (:326-384) and x2t tries nothing else; with an "Ole"
// stream and a "LinkInfo" one, it converts nothing (:385-398).
const OLE: &str = "Ole";
const COMP_OBJ: &str = "CompObj";
const COMP_OBJ_HEADER: u32 = 28;
const LINK_INFO: &str = "LinkInfo";
// With a "Package" stream, x2t converts the package that stream holds
// (:356-359).
const PACKAGE: &str = "Package";
// POLE reads a stream smaller than this from the mini FAT (StreamIO::StreamIO,
// pole.cpp:2010-2013); the header gives no other value (Cfb::open).
const MINI_STREAM_CUTOFF: u32 = 4096;

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
    Readable { holds_encrypted_package: bool, binary_format: Option<BinaryFormat> },
    // The file starts with the CFB signature, but its header is shorter than
    // 512 bytes or is one POLE refuses, its DIFAT is not the length POLE reads,
    // one of those sectors or a FAT sector is not whole in the file, or the
    // directory or the DIFAT is past its limit.
    Unreadable,
}

// The format x2t converts a CFB as when a stream at its root decides it. When
// it picks one, x2t returns before it tries the file as a ZIP
// (OfficeFileFormatChecker2.cpp:715-790), and converts it as that format.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BinaryFormat {
    Doc,
    Xls,
    Ppt,
}

// Whether the file is a CFB, whether its root holds both entries x2t decrypts
// an OOXML package from, and the binary format x2t takes it for.
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
    let Ok(root) = read_root(reader, &header) else {
        return CfbDirectory::Unreadable;
    };
    CfbDirectory::Readable {
        holds_encrypted_package: root.has(ENCRYPTION_INFO) && root.has(ENCRYPTED_PACKAGE),
        binary_format: binary_format(&root),
    }
}

// The entries at the root, as x2t looks them up by name.
struct Root {
    entries: Vec<RootEntry>,
    comp_obj: CompObj,
}

struct RootEntry {
    name: String,
    is_stream: bool,
    // A stream POLE reads at least one byte of.
    gives_data: bool,
}

// The names isOleObjectFile reads from the "CompObj" stream (:337-354).
enum CompObj {
    // No CompObj stream of COMP_OBJ_HEADER bytes or more.
    Short,
    // The user type and the program, as bytes.
    Names { user_type: Vec<u8>, program: Vec<u8> },
    // A CompObj this reader does not read as x2t does.
    NotRead,
}

impl Root {
    fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a RootEntry> + 'a {
        self.entries.iter().filter(move |entry| entry.name == name)
    }

    fn has(&self, name: &str) -> bool {
        self.named(name).next().is_some()
    }

    // POLE finds one of the entries with a name; when there are several, all
    // of them have to give data.
    fn gives_data(&self, name: &str) -> bool {
        self.has(name) && self.named(name).all(|entry| entry.gives_data)
    }

    fn has_only_streams_named(&self, name: &str) -> bool {
        self.has(name) && self.named(name).all(|entry| entry.is_stream)
    }

    fn takes_for_a_doc(&self) -> bool {
        self.gives_data(WORD_DOCUMENT) && !self.has(DATA_SPACES)
    }

    fn takes_for_an_xls(&self) -> bool {
        WORKBOOK_STREAMS.iter().any(|name| self.gives_data(name))
    }

    fn takes_for_a_ppt(&self) -> bool {
        self.gives_data(POWERPOINT_DOCUMENT)
    }
}

fn binary_format(root: &Root) -> Option<BinaryFormat> {
    if !root.has(OLE) {
        return format_from_streams(root);
    }
    if !root.has_only_streams_named(OLE) {
        return None;
    }
    embedded_object_format(root)
}

fn format_from_streams(root: &Root) -> Option<BinaryFormat> {
    if root.takes_for_a_doc() {
        return Some(BinaryFormat::Doc);
    }
    if root.takes_for_an_xls() {
        return Some(BinaryFormat::Xls);
    }
    if root.takes_for_a_ppt() {
        return Some(BinaryFormat::Ppt);
    }
    None
}

// isOleObjectFile with an "Ole" stream: the first name in CompObj's user type
// or program among "Excel", "Word" and "PowerPoint" says which stream it
// looks for (:360-381).
fn embedded_object_format(root: &Root) -> Option<BinaryFormat> {
    let (user_type, program) = match &root.comp_obj {
        CompObj::Short if root.has(LINK_INFO) => return None,
        CompObj::Short => return format_from_streams(root),
        CompObj::NotRead => return None,
        CompObj::Names { user_type, program } => (user_type, program),
    };
    if root.has(PACKAGE) {
        return None;
    }
    let names = |word: &[u8]| contains(user_type, word) || contains(program, word);
    if names(b"Excel") {
        return root.takes_for_an_xls().then_some(BinaryFormat::Xls);
    }
    if names(b"Word") {
        return root.takes_for_a_doc().then_some(BinaryFormat::Doc);
    }
    if names(b"PowerPoint") {
        return root.takes_for_a_ppt().then_some(BinaryFormat::Ppt);
    }
    None
}

fn contains(bytes: &[u8], word: &[u8]) -> bool {
    bytes.windows(word.len()).any(|window| window == word)
}

// A sector, or the header, that cannot be read whole, or a limit passed.
struct Unreadable;

fn read_root<R: Read + Seek>(reader: R, header: &[u8]) -> Result<Root, Unreadable> {
    let mut cfb = Cfb::open(reader, header)?;
    cfb.check_fat_sectors_whole()?;
    let chain = cfb.directory_chain()?;
    let entries = cfb.entries_under_root(&chain)?;
    let mini_fat_entries = cfb.mini_fat_entries()?;
    let comp_obj = cfb.comp_obj(&chain, &entries, mini_fat_entries);
    let entries = entries
        .into_iter()
        .map(|entry| RootEntry {
            is_stream: entry.is_stream,
            gives_data: entry.gives_data(&cfb, mini_fat_entries),
            name: entry.name,
        })
        .collect();
    Ok(Root { entries, comp_obj })
}

// As ReadStringFromOle (OfficeFileFormatChecker2.cpp:57-101) reads the strings
// of CompObj after its header, while more than 4 bytes are left: a length,
// that many bytes, then 2 more skipped. With a length past what is left, the
// string is empty and the reading goes on after the first zero byte that
// follows the length; None when there is none, as x2t then reads past what
// the stream gave.
fn comp_obj_names(stream: &[u8]) -> Option<CompObj> {
    let mut position = COMP_OBJ_HEADER as usize;
    let mut strings = Vec::new();
    for _ in 0..3 {
        let left = stream.len().saturating_sub(position);
        if left <= 4 {
            strings.push(Vec::new());
            continue;
        }
        let length = u32_at(stream, position) as usize;
        position += 4;
        if length > left {
            let zero = stream[position..].iter().position(|&byte| byte == 0)?;
            strings.push(Vec::new());
            position += zero + 1;
            continue;
        }
        let end = (position + length).min(stream.len());
        strings.push(stream[position..end].to_vec());
        position = end + 2;
    }
    let program = strings.pop()?;
    let _clipboard_format = strings.pop()?;
    let user_type = strings.pop()?;
    Some(CompObj::Names { user_type, program })
}

struct Cfb<R> {
    reader: R,
    file_len: u64,
    sector_shift: u32,
    mini_sector_shift: u32,
    // The number of FAT sectors the header declares: POLE reads no FAT entry
    // past them.
    fat_sectors: u64,
    // The number of DIFAT sectors the header declares: POLE reads exactly
    // that many.
    difat_sectors: u64,
    directory_start: u32,
    mini_fat_start: u32,
    header_difat: Vec<u32>,
    first_difat_sector: u32,
    // The DIFAT sectors found so far, in chain order.
    difat_chain: Vec<u32>,
}

struct Entry {
    name: String,
    valid: bool,
    is_stream: bool,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u32,
}

impl Entry {
    // StreamIO::read gives as many bytes as asked, up to the size, as soon as
    // the stream's first sector has an entry in the table it is read from
    // (pole.cpp:2123-2184): what is in the sectors does not count.
    // AllocTable::follow gives no sector for a start past the table
    // (pole.cpp:533-540).
    fn gives_data<R: Read + Seek>(&self, cfb: &Cfb<R>, mini_fat_entries: u64) -> bool {
        if !self.is_stream || self.size == 0 {
            return false;
        }
        if self.size < MINI_STREAM_CUTOFF {
            return (self.start as u64) < mini_fat_entries;
        }
        cfb.in_fat(self.start)
    }
}

impl<R: Read + Seek> Cfb<R> {
    fn open(mut reader: R, header: &[u8]) -> Result<Cfb<R>, Unreadable> {
        if header.len() < HEADER_LEN {
            return Err(Unreadable);
        }
        if !pole_accepts_header(header) {
            return Err(Unreadable);
        }
        let sector_shift = u16_at(header, 0x1E) as u32;
        let file_len = reader.seek(SeekFrom::End(0)).map_err(|_| Unreadable)?;
        let header_difat = (0..HEADER_FAT_SECTORS as usize)
            .map(|i| u32_at(header, 0x4C + i * 4))
            .collect();
        let cfb = Cfb {
            reader,
            file_len,
            sector_shift,
            mini_sector_shift: u16_at(header, 0x20) as u32,
            fat_sectors: u32_at(header, 0x2C) as u64,
            difat_sectors: u32_at(header, 0x48) as u64,
            directory_start: u32_at(header, 0x30),
            mini_fat_start: u32_at(header, 0x3C),
            header_difat,
            first_difat_sector: u32_at(header, 0x44),
            difat_chain: Vec::new(),
        };
        if !cfb.difat_length_as_pole_reads_it() {
            return Err(Unreadable);
        }
        Ok(cfb)
    }

    // POLE reads the FAT sectors past the first 109 from exactly the number
    // of DIFAT sectors the header declares, and none when there are 109 or
    // fewer (StorageIO::getbbatBlocks, pole.cpp:1903-1931). With fewer, it
    // takes the next link as a FAT sector and leaves the rest at sector 0;
    // with more, it reads the last FAT sector as one more DIFAT sector. Only
    // with as many as the FAT sectors need does it follow the chain this
    // reader follows.
    fn difat_length_as_pole_reads_it(&self) -> bool {
        if self.fat_sectors <= HEADER_FAT_SECTORS {
            return true;
        }
        let per_difat_sector = self.u32s_per_sector() - 1;
        let needed = (self.fat_sectors - HEADER_FAT_SECTORS).div_ceil(per_difat_sector);
        self.difat_sectors == needed
    }

    // POLE loads every FAT sector into one table, one after the other
    // (StorageIO::loadBigBlocks, pole.cpp:1682-1706): one not whole in the
    // file moves the ones after it to other entries of that table.
    fn check_fat_sectors_whole(&mut self) -> Result<(), Unreadable> {
        let in_header = self.fat_sectors.min(HEADER_FAT_SECTORS) as usize;
        for index in 0..in_header {
            self.sector_offset(self.header_difat[index])?;
        }
        let per_difat_sector = self.u32s_per_sector() - 1;
        let mut remaining = self.fat_sectors.saturating_sub(HEADER_FAT_SECTORS);
        let mut position = 0;
        while remaining > 0 {
            let count = remaining.min(per_difat_sector);
            let difat_sector = self.difat_sector(position)?;
            let offset = self.sector_offset(difat_sector)?;
            self.reader.seek(SeekFrom::Start(offset)).map_err(|_| Unreadable)?;
            for _ in 0..count {
                let fat_sector = self.read_next_u32()?;
                self.sector_offset(fat_sector)?;
            }
            remaining -= count;
            position += 1;
        }
        Ok(())
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
        self.reader.seek(SeekFrom::Start(offset)).map_err(|_| Unreadable)?;
        self.read_next_u32()
    }

    fn read_next_u32(&mut self) -> Result<u32, Unreadable> {
        let mut bytes = [0u8; 4];
        self.reader.read_exact(&mut bytes).map_err(|_| Unreadable)?;
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

    // The entries POLE loads into the mini FAT: as many as its chain of sectors
    // holds (StorageIO::load, pole.cpp:1419-1426), followed as
    // directory_chain follows the directory.
    fn mini_fat_entries(&mut self) -> Result<u64, Unreadable> {
        let mut sectors = 0;
        let mut seen = HashSet::new();
        let mut sector = self.mini_fat_start;
        while self.in_fat(sector) && seen.insert(sector) {
            sectors += 1;
            sector = self.next_sector(sector)?;
        }
        Ok(sectors * self.u32s_per_sector())
    }

    // The CompObj stream at the root, as POLE reads it.
    fn comp_obj(&mut self, chain: &[u32], entries: &[Entry], mini_fat_entries: u64) -> CompObj {
        let named: Vec<&Entry> = entries.iter().filter(|entry| entry.name == COMP_OBJ).collect();
        let [entry] = named.as_slice() else {
            return if named.is_empty() { CompObj::Short } else { CompObj::NotRead };
        };
        if !entry.is_stream || entry.size < COMP_OBJ_HEADER {
            return CompObj::Short;
        }
        if entry.size >= MINI_STREAM_CUTOFF {
            return CompObj::NotRead;
        }
        let Ok(stream) = self.mini_stream_bytes(chain, entry, mini_fat_entries) else {
            return CompObj::NotRead;
        };
        comp_obj_names(&stream).unwrap_or(CompObj::NotRead)
    }

    // A stream smaller than MINI_STREAM_CUTOFF, from the mini stream: the
    // chain of sectors that starts at the first sector of the root entry
    // (StorageIO::load, pole.cpp:1441-1445; StorageIO::loadSmallBlocks,
    // pole.cpp:1771-1807). Every sector it needs has to be in the file.
    fn mini_stream_bytes(&mut self, chain: &[u32], entry: &Entry, mini_fat_entries: u64) -> Result<Vec<u8>, Unreadable> {
        let container_start = self.entry(chain, 0)?.start;
        let mini_len = 1u64 << self.mini_sector_shift;
        let sector_len = 1u64 << self.sector_shift;
        let mut bytes = Vec::new();
        let mut seen = HashSet::new();
        let mut mini_sector = entry.start;
        while bytes.len() < entry.size as usize {
            if mini_sector as u64 >= mini_fat_entries || !seen.insert(mini_sector) {
                return Err(Unreadable);
            }
            let position = mini_sector as u64 * mini_len;
            let sector = self.nth_sector(container_start, position / sector_len)?;
            let offset = self.sector_offset(sector)? + position % sector_len;
            let mut mini = vec![0u8; mini_len as usize];
            self.read_bytes(offset, &mut mini)?;
            bytes.extend_from_slice(&mini);
            mini_sector = self.next_mini_sector(mini_sector)?;
        }
        bytes.truncate(entry.size as usize);
        Ok(bytes)
    }

    fn next_mini_sector(&mut self, mini_sector: u32) -> Result<u32, Unreadable> {
        let per_sector = self.u32s_per_sector();
        let sector = self.nth_sector(self.mini_fat_start, mini_sector as u64 / per_sector)?;
        let offset = self.sector_offset(sector)? + (mini_sector as u64 % per_sector) * 4;
        self.read_u32(offset)
    }

    // The nth sector of the chain that starts at `start`, as
    // AllocTable::follow gives the chain.
    fn nth_sector(&mut self, start: u32, n: u64) -> Result<u32, Unreadable> {
        let mut seen = HashSet::new();
        let mut sector = start;
        for _ in 0..n {
            if !self.in_fat(sector) || !seen.insert(sector) {
                return Err(Unreadable);
            }
            sector = self.next_sector(sector)?;
        }
        if !self.in_fat(sector) || seen.contains(&sector) {
            return Err(Unreadable);
        }
        Ok(sector)
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

    // The root's children: every entry reachable through the siblings of the
    // root's child, which holds every one POLE's search from the root can
    // reach (dirtree_find_sibling). An invalid entry ends a branch, as it ends
    // POLE's search.
    fn entries_under_root(&mut self, chain: &[u32]) -> Result<Vec<Entry>, Unreadable> {
        let count = chain.len() * self.entries_per_sector();
        let in_directory = |index: u32| (index as usize) < count;
        let mut entries = Vec::new();
        if chain.is_empty() {
            return Ok(entries);
        }
        let root = self.entry(chain, 0)?;
        if !root.valid || !in_directory(root.child) {
            return Ok(entries);
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
            for sibling in [entry.left, entry.right] {
                if sibling > 0 && in_directory(sibling) {
                    pending.push(sibling);
                }
            }
            entries.push(entry);
        }
        Ok(entries)
    }
}

// Header::valid (pole.cpp:377-388), with the threshold Header::load reads
// (pole.cpp:401-402): POLE reads no directory from a header it refuses.
fn pole_accepts_header(header: &[u8]) -> bool {
    let sector_shift = u16_at(header, 0x1E);
    let mini_sector_shift = u16_at(header, 0x20);
    let fat_sectors = u32_at(header, 0x2C);
    let difat_sectors = u32_at(header, 0x48);
    let threshold = match u32_at(header, 0x38) {
        0 => 4096,
        threshold => threshold,
    };
    if threshold != 4096 || fat_sectors == 0 {
        return false;
    }
    if fat_sectors < HEADER_FAT_SECTORS as u32 && difat_sectors != 0 {
        return false;
    }
    mini_sector_shift <= sector_shift && (7..=30).contains(&sector_shift)
}

// As DirTree::load reads an entry (pole.cpp:963-1022): at most 32 UTF-16
// units of name, without the terminating one, and without a first unit below
// 32; only a stream is not a directory, and the size is 32 bits.
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
        is_stream: object_type == 2,
        left: u32_at(bytes, 0x44),
        right: u32_at(bytes, 0x48),
        child: u32_at(bytes, 0x4C),
        start: u32_at(bytes, 0x74),
        size: u32_at(bytes, 0x78),
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
    // next, so the DIFAT chain never ends. Every FAT sector is sector 0.
    pub fn directory_behind_difat_sector(position: u32) -> Vec<u8> {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        let fat_index = 109 + position * 127;
        bytes[0x2C..0x30].copy_from_slice(&(fat_index + 1).to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&(fat_index * 128).to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x48..0x4C].copy_from_slice(&(position + 1).to_le_bytes());
        bytes[0x4C..0x200].fill(0);
        let difat = 1024..1536;
        bytes[difat.clone()].fill(0);
        bytes[difat.end - 4..difat.end].copy_from_slice(&1u32.to_le_bytes());
        bytes
    }

    // The offset of the link at the end of the first DIFAT sector of
    // directory_found_through_the_difat.
    pub const FIRST_DIFAT_LINK: usize = 2 * 512 + 127 * 4;

    // 512-byte sectors and 237 FAT sectors, 128 more than the header names, so
    // the DIFAT takes two sectors: sectors 1 and 2. Sector 0 is FAT sectors 0
    // to 108, and sector 3 FAT sectors 109 to 236. The directory is in sector
    // 109 * 128, the first one whose FAT entry is in FAT sector 109.
    pub fn directory_found_through_the_difat() -> Vec<u8> {
        const FAT_SECTOR: u32 = 0xFFFF_FFFD;
        const DIFAT_SECTOR: u32 = 0xFFFF_FFFC;
        const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
        let directory_start: u32 = 109 * 128;
        let streams = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        let mut bytes = streams[..512].to_vec();
        bytes[0x2C..0x30].copy_from_slice(&237u32.to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&directory_start.to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x48..0x4C].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x4C..0x200].fill(0);
        let mut sectors = [[NONE; 128]; 4];
        sectors[0][..4].copy_from_slice(&[FAT_SECTOR, DIFAT_SECTOR, DIFAT_SECTOR, FAT_SECTOR]);
        sectors[1][..127].fill(3);
        sectors[1][127] = 2;
        sectors[2][0] = 3;
        sectors[2][127] = END_OF_CHAIN;
        sectors[3][0] = END_OF_CHAIN;
        bytes.extend(sectors.iter().flatten().flat_map(|entry| entry.to_le_bytes()));
        bytes.resize((directory_start as usize + 1) * 512, 0);
        bytes.extend_from_slice(&streams[1024..1536]);
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

    // A root whose children are streams with these names and contents, each in
    // its own run of sectors after the directory. Each content is 4096 bytes
    // or more, so it is read through the FAT.
    pub fn with_root_stream_contents(streams: &[(&str, &[u8])]) -> Vec<u8> {
        let names: Vec<&str> = streams.iter().map(|(name, _)| *name).collect();
        let mut bytes = with_root_streams(&names);
        let mut sector = (bytes.len() / 512 - 1) as u32;
        for (i, (_, content)) in streams.iter().enumerate() {
            assert!(content.len() >= 4096);
            let sectors = content.len().div_ceil(512) as u32;
            let entry = 1024 + (i + 1) * 128;
            bytes[entry + 0x74..entry + 0x78].copy_from_slice(&sector.to_le_bytes());
            bytes[entry + 0x78..entry + 0x7C].copy_from_slice(&(content.len() as u32).to_le_bytes());
            let last = sector + sectors - 1;
            for run in sector..=last {
                let next = if run == last { 0xFFFF_FFFE } else { run + 1 };
                let fat_entry = 512 + run as usize * 4;
                bytes[fat_entry..fat_entry + 4].copy_from_slice(&next.to_le_bytes());
            }
            bytes.extend_from_slice(content);
            bytes.resize(bytes.len().div_ceil(512) * 512, 0);
            sector = last + 1;
        }
        assert!(sector <= 128, "one FAT sector");
        bytes
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
        directory_behind_difat_sector, directory_found_through_the_difat, directory_of_sectors,
        directory_past_the_end, header_only, with_entries, with_root_stream_contents, with_root_streams,
        FIRST_DIFAT_LINK, NONE, STORAGE, STREAM,
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
        matches!(read_bytes(bytes), CfbDirectory::Readable { holds_encrypted_package: true, .. })
    }

    #[test]
    fn a_real_document_with_a_password_holds_an_encrypted_package() {
        assert!(holds(ENCRYPTED_DOCX));
    }

    #[test]
    fn a_real_word_97_doc_does_not() {
        assert!(!holds(WORD97_DOC));
    }

    fn binary_format_of(bytes: &[u8]) -> Option<BinaryFormat> {
        match read_bytes(bytes) {
            CfbDirectory::Readable { binary_format, .. } => binary_format,
            other => panic!("{other:?}"),
        }
    }

    const STREAM_DATA: &[u8] = &[1; 4096];

    // The offset of the first child's entry in the CFBs test_cfb builds.
    const FIRST_CHILD: usize = 1024 + 128;

    fn set_start_and_size(bytes: &mut [u8], entry: usize, start: u32, size: u32) {
        bytes[entry + 0x74..entry + 0x78].copy_from_slice(&start.to_le_bytes());
        bytes[entry + 0x78..entry + 0x7C].copy_from_slice(&size.to_le_bytes());
    }

    #[test]
    fn a_real_word_97_doc_is_converted_as_a_doc() {
        assert_eq!(binary_format_of(WORD97_DOC), Some(BinaryFormat::Doc));
    }

    #[test]
    fn a_real_document_with_a_password_is_not_converted_as_a_binary_format() {
        assert_eq!(binary_format_of(ENCRYPTED_DOCX), None);
    }

    #[test]
    fn each_97_2003_stream_with_data_decides_its_format() {
        let cases = [
            ("WordDocument", BinaryFormat::Doc),
            ("Workbook", BinaryFormat::Xls),
            ("Book", BinaryFormat::Xls),
            ("WORKBOOK", BinaryFormat::Xls),
            ("BOOK", BinaryFormat::Xls),
            ("book", BinaryFormat::Xls),
            ("PowerPoint Document", BinaryFormat::Ppt),
        ];
        for (name, format) in cases {
            let cfb = with_root_stream_contents(&[("Other", STREAM_DATA), (name, STREAM_DATA)]);
            assert_eq!(binary_format_of(&cfb), Some(format), "{name}");
        }
    }

    #[test]
    fn the_word_stream_is_tried_before_the_excel_one_and_that_before_the_powerpoint_one() {
        let all = with_root_stream_contents(&[
            ("PowerPoint Document", STREAM_DATA),
            ("Workbook", STREAM_DATA),
            ("WordDocument", STREAM_DATA),
        ]);
        assert_eq!(binary_format_of(&all), Some(BinaryFormat::Doc));
        let two = with_root_stream_contents(&[("PowerPoint Document", STREAM_DATA), ("Book", STREAM_DATA)]);
        assert_eq!(binary_format_of(&two), Some(BinaryFormat::Xls));
    }

    #[test]
    fn an_empty_stream_or_a_storage_with_that_name_decides_nothing() {
        assert_eq!(binary_format_of(&with_root_streams(&["WordDocument", "Workbook", "PowerPoint Document"])), None);
        let storage = with_entries(&[("Root Entry", 5, NONE, NONE, 1), ("Workbook", STORAGE, NONE, NONE, NONE)]);
        assert_eq!(binary_format_of(&storage), None);
    }

    #[test]
    fn a_stream_that_starts_past_the_fat_decides_nothing() {
        let mut bytes = with_root_stream_contents(&[("Workbook", STREAM_DATA)]);
        set_start_and_size(&mut bytes, FIRST_CHILD, 128, 4096);
        assert_eq!(binary_format_of(&bytes), None);
    }

    #[test]
    fn stream_names_are_compared_in_their_case() {
        for name in ["worddocument", "Powerpoint Document", "WorkBook"] {
            assert_eq!(binary_format_of(&with_root_stream_contents(&[(name, STREAM_DATA)])), None, "{name}");
        }
    }

    #[test]
    fn a_stream_inside_a_storage_decides_nothing() {
        let mut nested = with_entries(&[
            ("Root Entry", 5, NONE, NONE, 1),
            ("ObjectPool", STORAGE, NONE, NONE, 2),
            ("WordDocument", STREAM, NONE, NONE, NONE),
        ]);
        set_start_and_size(&mut nested, FIRST_CHILD + 128, 0, 4096);
        nested.resize(nested.len() + 4096, 1);
        assert_eq!(binary_format_of(&nested), None);
    }

    // Puts the content, under 4096 bytes, in the mini stream as the stream of
    // the root's child number `child`: a mini FAT sector and a mini stream
    // sector after what is there.
    fn with_small_stream(mut bytes: Vec<u8>, child: usize, content: &[u8]) -> Vec<u8> {
        let mini_sectors = content.len().div_ceil(64);
        assert!(mini_sectors <= 8, "one mini stream sector");
        let mini_fat = (bytes.len() / 512 - 1) as u32;
        let container = mini_fat + 1;
        for sector in [mini_fat, container] {
            let fat_entry = 512 + sector as usize * 4;
            bytes[fat_entry..fat_entry + 4].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        }
        bytes[0x3C..0x40].copy_from_slice(&mini_fat.to_le_bytes());
        bytes[1024 + 0x74..1024 + 0x78].copy_from_slice(&container.to_le_bytes());
        set_start_and_size(&mut bytes, 1024 + child * 128, 0, content.len() as u32);
        let mut mini_fat_sector = vec![NONE; 128];
        for mini in 0..mini_sectors {
            mini_fat_sector[mini] = if mini + 1 == mini_sectors { END_OF_CHAIN } else { mini as u32 + 1 };
        }
        bytes.extend(mini_fat_sector.iter().flat_map(|entry| entry.to_le_bytes()));
        let mut mini_stream = content.to_vec();
        mini_stream.resize(512, 0);
        bytes.extend(mini_stream);
        bytes
    }

    // A CompObj as ReadStringFromOle reads it: its header, then each string
    // after its length and followed by 2 bytes.
    fn comp_obj(user_type: &str, program: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 28];
        for string in [user_type, "", program] {
            bytes.extend((string.len() as u32).to_le_bytes());
            bytes.extend(string.as_bytes());
            bytes.extend([0, 0]);
        }
        bytes
    }

    // WordDocument and Workbook with data, an Ole stream, and the CompObj,
    // the root's fourth child.
    fn with_ole_and_comp_obj(extra: &[(&str, &[u8])], comp_obj: &[u8]) -> Vec<u8> {
        let mut streams: Vec<(&str, &[u8])> =
            vec![("WordDocument", STREAM_DATA), ("Workbook", STREAM_DATA), ("\u{1}Ole", STREAM_DATA), ("\u{1}CompObj", STREAM_DATA)];
        streams.extend_from_slice(extra);
        with_small_stream(with_root_stream_contents(&streams), 4, comp_obj)
    }

    #[test]
    fn with_an_ole_stream_the_names_in_comp_obj_pick_the_stream() {
        let cases = [
            ("Microsoft Word 97-2003 Document", "Word.Document.8", Some(BinaryFormat::Doc)),
            ("Microsoft Excel 97-2003 Worksheet", "Excel.Sheet.8", Some(BinaryFormat::Xls)),
            ("Microsoft PowerPoint Presentation", "PowerPoint.Show.8", None),
            ("Some Drawing", "Draw.1", None),
        ];
        for (user_type, program, format) in cases {
            let cfb = with_ole_and_comp_obj(&[], &comp_obj(user_type, program));
            assert_eq!(binary_format_of(&cfb), format, "{user_type}");
        }
    }

    #[test]
    fn with_an_ole_stream_a_package_stream_leaves_the_format_undecided() {
        let cfb = with_ole_and_comp_obj(&[("Package", STREAM_DATA)], &comp_obj("Microsoft Word 97-2003 Document", "Word.Document.8"));
        assert_eq!(binary_format_of(&cfb), None);
    }

    #[test]
    fn with_an_ole_stream_and_a_short_comp_obj_the_streams_decide_unless_there_is_a_link() {
        let short = with_ole_and_comp_obj(&[], &[0u8; 27]);
        assert_eq!(binary_format_of(&short), Some(BinaryFormat::Doc));
        let linked = with_ole_and_comp_obj(&[("LinkInfo", STREAM_DATA)], &[0u8; 27]);
        assert_eq!(binary_format_of(&linked), None);
    }

    #[test]
    fn a_comp_obj_whose_mini_stream_is_past_the_end_leaves_the_format_undecided() {
        let mut cfb = with_ole_and_comp_obj(&[], &comp_obj("Microsoft Word 97-2003 Document", "Word.Document.8"));
        cfb[1024 + 0x74..1024 + 0x78].copy_from_slice(&127u32.to_le_bytes());
        assert_eq!(binary_format_of(&cfb), None);
    }

    #[test]
    fn a_comp_obj_length_past_its_end_is_skipped_to_the_next_zero_byte() {
        let mut bytes = vec![0u8; 28];
        bytes.extend(1000u32.to_le_bytes());
        bytes.extend(b"ab\0");
        bytes.extend(15u32.to_le_bytes());
        bytes.extend(b"Word.Document.8\0\0");
        bytes.extend(0u32.to_le_bytes());
        let Some(CompObj::Names { user_type, program }) = comp_obj_names(&bytes) else {
            panic!("not read");
        };
        assert!(user_type.is_empty());
        assert!(program.is_empty(), "the clipboard format is read first");
        let mut no_zero = vec![0u8; 28];
        no_zero.extend(1000u32.to_le_bytes());
        no_zero.extend(b"abc");
        assert!(comp_obj_names(&no_zero).is_none());
    }

    #[test]
    fn a_data_spaces_entry_rules_out_only_the_doc() {
        let doc = with_root_stream_contents(&[("WordDocument", STREAM_DATA), ("\u{6}DataSpaces", STREAM_DATA)]);
        assert_eq!(binary_format_of(&doc), None);
        let xls = with_root_stream_contents(&[
            ("WordDocument", STREAM_DATA),
            ("\u{6}DataSpaces", STREAM_DATA),
            ("Workbook", STREAM_DATA),
        ]);
        assert_eq!(binary_format_of(&xls), Some(BinaryFormat::Xls));
    }

    #[test]
    fn a_small_stream_gives_data_when_the_mini_fat_holds_its_start() {
        let mut bytes = with_root_streams(&["WordDocument"]);
        set_start_and_size(&mut bytes, FIRST_CHILD, 5, 100);
        assert_eq!(binary_format_of(&bytes), None, "no mini FAT");

        let mini_fat = (bytes.len() / 512 - 1) as u32;
        let fat_entry = 512 + mini_fat as usize * 4;
        bytes[fat_entry..fat_entry + 4].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        bytes[0x3C..0x40].copy_from_slice(&mini_fat.to_le_bytes());
        bytes.resize(bytes.len() + 512, 0);
        assert_eq!(binary_format_of(&bytes), Some(BinaryFormat::Doc));

        set_start_and_size(&mut bytes, FIRST_CHILD, 128, 100);
        assert_eq!(binary_format_of(&bytes), None, "past the mini FAT");
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
        assert_eq!(read_bytes(&no_directory), CfbDirectory::Readable { holds_encrypted_package: false, binary_format: None });
        let mut outside_the_fat = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        outside_the_fat[0x30..0x34].copy_from_slice(&128u32.to_le_bytes());
        assert_eq!(read_bytes(&outside_the_fat), CfbDirectory::Readable { holds_encrypted_package: false, binary_format: None });
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
        let readable = CfbDirectory::Readable { holds_encrypted_package: false, binary_format: None };
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

    fn declaring_difat_sectors(declared: u32) -> Vec<u8> {
        let mut bytes = directory_found_through_the_difat();
        bytes[0x48..0x4C].copy_from_slice(&declared.to_le_bytes());
        bytes
    }

    #[test]
    fn an_index_found_through_the_difat_is_read_as_the_converter_reads_it() {
        assert!(holds(&directory_found_through_the_difat()));
    }

    #[test]
    fn with_109_fat_sectors_the_declared_difat_length_is_not_read() {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        bytes[0x2C..0x30].copy_from_slice(&109u32.to_le_bytes());
        bytes[0x48..0x4C].copy_from_slice(&5u32.to_le_bytes());
        bytes[0x4C..0x200].fill(0);
        assert!(holds(&bytes));
    }

    #[test]
    fn a_header_that_declares_no_difat_sector_for_more_than_109_fat_sectors_is_unreadable() {
        assert_eq!(read_bytes(&declaring_difat_sectors(0)), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_header_that_declares_fewer_difat_sectors_than_the_fat_needs_is_unreadable() {
        // The converter would read the link at the end of the first DIFAT
        // sector as FAT sector 236.
        assert_eq!(read_bytes(&declaring_difat_sectors(1)), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_header_that_declares_more_difat_sectors_than_the_fat_needs_is_unreadable() {
        // The converter would read FAT sector 236 as a third DIFAT sector.
        assert_eq!(read_bytes(&declaring_difat_sectors(3)), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_difat_link_to_a_sector_outside_the_file_is_unreadable() {
        let mut bytes = directory_found_through_the_difat();
        bytes[FIRST_DIFAT_LINK..FIRST_DIFAT_LINK + 4].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        assert_eq!(read_bytes(&bytes), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_fat_sector_the_index_does_not_need_is_still_read_whole() {
        let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        bytes[0x2C..0x30].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x50..0x54].copy_from_slice(&40u32.to_le_bytes());
        assert_eq!(read_bytes(&bytes), CfbDirectory::Unreadable);
    }

    #[test]
    fn a_header_the_converter_refuses_is_unreadable() {
        let refused = [(0x2C, 0u32), (0x48, 1), (0x38, 512)];
        for (offset, value) in refused {
            let mut bytes = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(read_bytes(&bytes), CfbDirectory::Unreadable, "{offset:#x}");
        }
        let mut mini_sectors_too_large = with_root_streams(&["EncryptionInfo", "EncryptedPackage"]);
        mini_sectors_too_large[0x20] = 10;
        assert_eq!(read_bytes(&mini_sectors_too_large), CfbDirectory::Unreadable);
    }
}
