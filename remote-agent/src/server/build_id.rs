//! The running binary's build id, which names holder sockets and tells a
//! core which holders are of its own build.
//!
//! It is the ELF `NT_GNU_BUILD_ID` note when the linker wrote one, else an
//! FNV-1a hash of the binary's bytes. It is computed once at startup from
//! `/proc/self/exe`, which still names the running binary after an install
//! replaces the file.

use std::fs;
use std::io;

/// The build id of `/proc/self/exe`, as lower-case hex.
pub fn current() -> io::Result<String> {
    let bytes = fs::read("/proc/self/exe")?;
    Ok(of_binary(&bytes))
}

pub fn of_binary(bytes: &[u8]) -> String {
    match elf_build_id(bytes) {
        Some(id) if !id.is_empty() => hex(id),
        _ => format!("{:016x}", fnv1a(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

const PT_NOTE: u32 = 4;
const NT_GNU_BUILD_ID: u32 = 3;

/// Reads the build-id note from an ELF file's `PT_NOTE` segments.
fn elf_build_id(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..4)? != b"\x7fELF" {
        return None;
    }
    let wide = match bytes.get(4)? {
        1 => false,
        2 => true,
        _ => return None,
    };
    let little = match bytes.get(5)? {
        1 => true,
        2 => false,
        _ => return None,
    };
    let reader = Reader { bytes, little };
    let (phoff, phentsize, phnum) = if wide {
        (
            reader.u64(0x20)? as usize,
            reader.u16(0x36)? as usize,
            reader.u16(0x38)? as usize,
        )
    } else {
        (
            reader.u32(0x1c)? as usize,
            reader.u16(0x2a)? as usize,
            reader.u16(0x2c)? as usize,
        )
    };
    for index in 0..phnum {
        let header = phoff.checked_add(index.checked_mul(phentsize)?)?;
        if reader.u32(header)? != PT_NOTE {
            continue;
        }
        let (offset, size) = if wide {
            (
                reader.u64(header + 0x08)? as usize,
                reader.u64(header + 0x20)? as usize,
            )
        } else {
            (
                reader.u32(header + 0x04)? as usize,
                reader.u32(header + 0x10)? as usize,
            )
        };
        let end = offset.checked_add(size)?;
        if let Some(id) = find_note(&reader, offset, end) {
            return Some(id);
        }
    }
    None
}

fn find_note<'a>(reader: &Reader<'a>, mut at: usize, end: usize) -> Option<&'a [u8]> {
    let align4 = |n: usize| n.checked_add(3).map(|n| n & !3);
    while at.checked_add(12)? <= end {
        let namesz = reader.u32(at)? as usize;
        let descsz = reader.u32(at + 4)? as usize;
        let kind = reader.u32(at + 8)?;
        let name_at = at + 12;
        let desc_at = name_at.checked_add(align4(namesz)?)?;
        let next = desc_at.checked_add(align4(descsz)?)?;
        if desc_at.checked_add(descsz)? > end {
            return None;
        }
        if kind == NT_GNU_BUILD_ID && reader.bytes.get(name_at..name_at + namesz)? == b"GNU\0" {
            return reader.bytes.get(desc_at..desc_at + descsz);
        }
        at = next;
    }
    None
}

struct Reader<'a> {
    bytes: &'a [u8],
    little: bool,
}

impl Reader<'_> {
    fn array<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.bytes.get(at..at.checked_add(N)?)?.try_into().ok()
    }

    fn u16(&self, at: usize) -> Option<u16> {
        let bytes = self.array(at)?;
        Some(if self.little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let bytes = self.array(at)?;
        Some(if self.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn u64(&self, at: usize) -> Option<u64> {
        let bytes = self.array(at)?;
        Some(if self.little {
            u64::from_le_bytes(bytes)
        } else {
            u64::from_be_bytes(bytes)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal little-endian ELF64 file with one `PT_NOTE` segment holding
    /// `notes`.
    fn elf64(notes: &[u8]) -> Vec<u8> {
        let mut file = vec![0u8; 64 + 56];
        file[..4].copy_from_slice(b"\x7fELF");
        file[4] = 2;
        file[5] = 1;
        file[0x20..0x28].copy_from_slice(&64u64.to_le_bytes());
        file[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        file[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
        file[64..68].copy_from_slice(&PT_NOTE.to_le_bytes());
        file[64 + 8..64 + 16].copy_from_slice(&120u64.to_le_bytes());
        file[64 + 0x20..64 + 0x28].copy_from_slice(&(notes.len() as u64).to_le_bytes());
        file.extend_from_slice(notes);
        file
    }

    fn note(name: &[u8], kind: u32, desc: &[u8]) -> Vec<u8> {
        let mut note = Vec::new();
        note.extend_from_slice(&(name.len() as u32).to_le_bytes());
        note.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        note.extend_from_slice(&kind.to_le_bytes());
        note.extend_from_slice(name);
        note.resize(note.len().next_multiple_of(4), 0);
        note.extend_from_slice(desc);
        note.resize(note.len().next_multiple_of(4), 0);
        note
    }

    #[test]
    fn the_gnu_build_id_note_is_used() {
        let mut notes = note(b"GNU\0", 1, &[9; 16]);
        notes.extend(note(b"XYZ\0", NT_GNU_BUILD_ID, &[7; 4]));
        notes.extend(note(b"GNU\0", NT_GNU_BUILD_ID, &[0xab, 0x01, 0xff]));
        assert_eq!(of_binary(&elf64(&notes)), "ab01ff");
    }

    #[test]
    fn without_a_note_the_bytes_are_hashed() {
        let file = elf64(&note(b"GNU\0", 1, &[1, 2]));
        let id = of_binary(&file);
        assert_eq!(id.len(), 16);
        assert_eq!(id, format!("{:016x}", fnv1a(&file)));
        assert_ne!(of_binary(b"not elf"), of_binary(b"not elf!"));
        assert_eq!(of_binary(b""), "cbf29ce484222325");
        // Truncated and corrupt headers never panic.
        for len in 0..file.len() {
            let _ = of_binary(&file[..len]);
        }
    }

    #[test]
    fn this_test_binary_has_a_stable_id() {
        let id = current().unwrap();
        assert!(
            id.len() >= 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "{id}"
        );
        assert_eq!(current().unwrap(), id);
    }
}
