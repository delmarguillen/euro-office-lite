// The CPU an executable is built for, read from its header without running
// it, so a run can tell a native x2t from an emulated one.

use std::io::Read;
use std::path::Path;

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

// Names as std::env::consts::ARCH spells them, so they compare with the host.
fn elf_machine(machine: u16) -> &'static str {
    match machine {
        0x3E => "x86_64",
        0xB7 => "aarch64",
        0x03 => "x86",
        _ => "unknown",
    }
}

fn pe_machine(machine: u16) -> &'static str {
    match machine {
        0x8664 => "x86_64",
        0xAA64 => "aarch64",
        0x014C => "x86",
        _ => "unknown",
    }
}

fn macho_cpu(cpu: u32) -> &'static str {
    match cpu {
        0x0100_0007 => "x86_64",
        0x0100_000C => "aarch64",
        0x0000_0007 => "x86",
        _ => "unknown",
    }
}

fn elf(bytes: &[u8]) -> Option<String> {
    let machine = match bytes.get(5)? {
        1 => u16_le(bytes, 18)?,
        2 => u16_be(bytes, 18)?,
        _ => return None,
    };
    Some(elf_machine(machine).to_string())
}

fn pe(bytes: &[u8]) -> Option<String> {
    let at = u32_le(bytes, 0x3C)? as usize;
    if bytes.get(at..at + 4)? != b"PE\0\0" {
        return None;
    }
    Some(pe_machine(u16_le(bytes, at + 4)?).to_string())
}

// A universal binary lists one entry per slice: 20 bytes each, 32 in the
// 64-bit variant. Java class files share the magic, with a large count.
fn universal(bytes: &[u8], entry: usize) -> Option<String> {
    let count = u32_be(bytes, 4)? as usize;
    if count == 0 || count > 16 {
        return None;
    }
    let cpus = (0..count)
        .map(|i| u32_be(bytes, 8 + i * entry).map(macho_cpu))
        .collect::<Option<Vec<_>>>()?;
    Some(format!("universal:{}", cpus.join(",")))
}

pub fn exe_arch(bytes: &[u8]) -> String {
    let found = match bytes.get(..4) {
        Some([0x7F, b'E', b'L', b'F']) => elf(bytes),
        Some([b'M', b'Z', ..]) => pe(bytes),
        Some([0xCF, 0xFA, 0xED, 0xFE]) | Some([0xCE, 0xFA, 0xED, 0xFE]) => u32_le(bytes, 4).map(|c| macho_cpu(c).to_string()),
        Some([0xCA, 0xFE, 0xBA, 0xBE]) => universal(bytes, 20),
        Some([0xCA, 0xFE, 0xBA, 0xBF]) => universal(bytes, 32),
        _ => None,
    };
    found.unwrap_or_else(|| "unknown".to_string())
}

// The headers sit at the start: the first 64 KiB are plenty.
pub fn file_arch(path: &Path) -> String {
    let mut bytes = Vec::new();
    match std::fs::File::open(path).and_then(|file| file.take(64 * 1024).read_to_end(&mut bytes)) {
        Ok(_) => exe_arch(&bytes),
        Err(_) => "unknown".to_string(),
    }
}

// An x2t built for another CPU than the host's runs under emulation. A
// universal binary with a slice for the host runs natively. Results written
// before the architecture was recorded have none.
pub fn emulated(x2t: &str, host: &str) -> bool {
    if x2t.is_empty() || host.is_empty() || x2t == "unknown" || x2t == host {
        return false;
    }
    match x2t.strip_prefix("universal:") {
        Some(list) => !list.split(',').any(|cpu| cpu == host),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf_header(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0x7F, b'E', b'L', b'F', 2, 1, 1, 0];
        bytes.resize(18, 0);
        bytes.extend_from_slice(&machine.to_le_bytes());
        bytes
    }

    fn pe_header(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x80];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
        bytes[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    fn macho_header(cpu: u32) -> Vec<u8> {
        let mut bytes = vec![0xCF, 0xFA, 0xED, 0xFE];
        bytes.extend_from_slice(&cpu.to_le_bytes());
        bytes
    }

    fn fat_header(cpus: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE];
        bytes.extend_from_slice(&(cpus.len() as u32).to_be_bytes());
        for cpu in cpus {
            bytes.extend_from_slice(&cpu.to_be_bytes());
            bytes.extend_from_slice(&[0u8; 16]);
        }
        bytes
    }

    #[test]
    fn each_executable_format_names_its_cpu() {
        assert_eq!(exe_arch(&elf_header(0x3E)), "x86_64");
        assert_eq!(exe_arch(&elf_header(0xB7)), "aarch64");
        assert_eq!(exe_arch(&pe_header(0x8664)), "x86_64");
        assert_eq!(exe_arch(&pe_header(0xAA64)), "aarch64");
        assert_eq!(exe_arch(&macho_header(0x0100_0007)), "x86_64");
        assert_eq!(exe_arch(&macho_header(0x0100_000C)), "aarch64");
        assert_eq!(exe_arch(&fat_header(&[0x0100_0007, 0x0100_000C])), "universal:x86_64,aarch64");
    }

    #[test]
    fn short_or_unknown_files_are_unknown() {
        assert_eq!(exe_arch(b""), "unknown");
        assert_eq!(exe_arch(b"\x7FEL"), "unknown");
        assert_eq!(exe_arch(&elf_header(0x3E)[..19]), "unknown");
        assert_eq!(exe_arch(b"#!/bin/sh\necho x2t\n"), "unknown");
        assert_eq!(exe_arch(&elf_header(0x28)), "unknown");
        assert_eq!(exe_arch(&pe_header(0x8664)[..0x44]), "unknown");
        let mut no_pe = pe_header(0x8664);
        no_pe[0x40] = b'X';
        assert_eq!(exe_arch(&no_pe), "unknown");
        // A Java class file: same magic, a version where the count would be.
        assert_eq!(exe_arch(&[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 0x34]), "unknown");
    }

    #[test]
    fn a_foreign_x2t_is_emulated_unless_a_slice_matches() {
        assert!(emulated("x86_64", "aarch64"));
        assert!(!emulated("x86_64", "x86_64"));
        assert!(!emulated("unknown", "aarch64"));
        assert!(!emulated("", "aarch64"));
        assert!(!emulated("universal:x86_64,aarch64", "aarch64"));
        assert!(emulated("universal:x86_64", "aarch64"));
    }

    // A real header on whichever OS runs the tests.
    #[test]
    fn this_test_binary_is_built_for_the_host() {
        assert_eq!(file_arch(&std::env::current_exe().unwrap()), std::env::consts::ARCH);
    }

    #[test]
    fn a_file_is_read_from_disk() {
        let path = std::env::temp_dir().join(format!("corpus-arch-{}", std::process::id()));
        std::fs::write(&path, pe_header(0xAA64)).unwrap();
        assert_eq!(file_arch(&path), "aarch64");
        let _ = std::fs::remove_file(&path);
        assert_eq!(file_arch(&path), "unknown");
    }
}
