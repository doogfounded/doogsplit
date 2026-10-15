//! Custom binary container header for `doogsplit` volumes.
//!
//! # Binary Layout (All multi-byte integers are Little-Endian):
//!
//! ```text
//! Offset  Size  Field          Description
//! -------------------------------------------------------------
//! 0x00    4     magic          Fixed ASCII string: "DOOG"
//! 0x04    2     version        Format version (currently 1)
//! 0x06    2     flags          Bit 0: IS_LAST_VOLUME
//! 0x08    4     volume_index   1-based index (1 ..= total_volumes)
//! 0x0C    4     total_volumes  Total number of volumes in the split set
//! 0x10    8     payload_size   Bytes of raw payload in this volume
//! 0x18    8     original_size  Total size in bytes of the original file
//! 0x20    4     volume_crc32   CRC-32 checksum of this volume's payload
//! 0x24    4     overall_crc32  CRC-32 checksum of the complete original file
//! 0x28    2     filename_len   Length in bytes of original filename (N)
//! 0x2A    N     filename       UTF-8 encoded original filename
//! 0x2A+N  4     header_crc32   CRC-32 checksum of all preceding header bytes
//! ```

use std::fmt;
use std::io::{self, Read, Write};

use crate::crc32::Crc32;

pub const MAGIC: &[u8; 4] = b"DOOG";
pub const FORMAT_VERSION: u16 = 1;
pub const FIXED_PREFIX_SIZE: usize = 42;

pub const FLAG_LAST_VOLUME: u16 = 1 << 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoogHeader {
    pub version: u16,
    pub flags: u16,
    pub volume_index: u32,
    pub total_volumes: u32,
    pub payload_size: u64,
    pub original_size: u64,
    pub volume_crc32: u32,
    pub overall_crc32: u32,
    pub filename: String,
}

#[derive(Debug)]
pub enum HeaderError {
    Io(io::Error),
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u16),
    InvalidUtf8(std::string::FromUtf8Error),
    HeaderCrcMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::Io(e) => write!(f, "I/O error reading header: {}", e),
            HeaderError::InvalidMagic(m) => write!(
                f,
                "Invalid magic bytes: {:?} (expected {:?})",
                m, MAGIC
            ),
            HeaderError::UnsupportedVersion(v) => {
                write!(f, "Unsupported header version: {} (supported: {})", v, FORMAT_VERSION)
            }
            HeaderError::InvalidUtf8(e) => write!(f, "Corrupted filename in header: {}", e),
            HeaderError::HeaderCrcMismatch { expected, actual } => write!(
                f,
                "Header checksum mismatch! Expected 0x{:08X}, got 0x{:08X} (header is corrupted)",
                expected, actual
            ),
        }
    }
}

impl std::error::Error for HeaderError {}

impl From<io::Error> for HeaderError {
    fn from(e: io::Error) -> Self {
        HeaderError::Io(e)
    }
}

impl DoogHeader {
    pub fn new(
        volume_index: u32,
        total_volumes: u32,
        payload_size: u64,
        original_size: u64,
        volume_crc32: u32,
        overall_crc32: u32,
        filename: String,
    ) -> Self {
        let mut flags = 0u16;
        if volume_index == total_volumes {
            flags |= FLAG_LAST_VOLUME;
        }

        Self {
            version: FORMAT_VERSION,
            flags,
            volume_index,
            total_volumes,
            payload_size,
            original_size,
            volume_crc32,
            overall_crc32,
            filename,
        }
    }

    pub fn is_last_volume(&self) -> bool {
        (self.flags & FLAG_LAST_VOLUME) != 0
    }

    /// Serializes the header into raw bytes including the header CRC-32 checksum.
    pub fn serialize(&self) -> Vec<u8> {
        let filename_bytes = self.filename.as_bytes();
        let filename_len = filename_bytes.len() as u16;

        let total_size = FIXED_PREFIX_SIZE + (filename_len as usize) + 4;
        let mut bytes = Vec::with_capacity(total_size);

        // Fixed prefix
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.flags.to_le_bytes());
        bytes.extend_from_slice(&self.volume_index.to_le_bytes());
        bytes.extend_from_slice(&self.total_volumes.to_le_bytes());
        bytes.extend_from_slice(&self.payload_size.to_le_bytes());
        bytes.extend_from_slice(&self.original_size.to_le_bytes());
        bytes.extend_from_slice(&self.volume_crc32.to_le_bytes());
        bytes.extend_from_slice(&self.overall_crc32.to_le_bytes());
        bytes.extend_from_slice(&filename_len.to_le_bytes());

        // Variable length filename
        bytes.extend_from_slice(filename_bytes);

        // Calculate and append CRC-32 of the header itself
        let header_crc = Crc32::compute(&bytes);
        bytes.extend_from_slice(&header_crc.to_le_bytes());

        bytes
    }

    /// Deserializes and validates a header from a streaming reader.
    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Self, HeaderError> {
        let mut prefix = [0u8; FIXED_PREFIX_SIZE];
        reader.read_exact(&mut prefix)?;

        // Verify Magic
        if &prefix[0..4] != MAGIC {
            return Err(HeaderError::InvalidMagic([
                prefix[0], prefix[1], prefix[2], prefix[3],
            ]));
        }

        let version = u16::from_le_bytes([prefix[4], prefix[5]]);
        if version != FORMAT_VERSION {
            return Err(HeaderError::UnsupportedVersion(version));
        }

        let flags = u16::from_le_bytes([prefix[6], prefix[7]]);
        let volume_index = u32::from_le_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]);
        let total_volumes = u32::from_le_bytes([prefix[12], prefix[13], prefix[14], prefix[15]]);

        let payload_size = u64::from_le_bytes([
            prefix[16], prefix[17], prefix[18], prefix[19],
            prefix[20], prefix[21], prefix[22], prefix[23],
        ]);

        let original_size = u64::from_le_bytes([
            prefix[24], prefix[25], prefix[26], prefix[27],
            prefix[28], prefix[29], prefix[30], prefix[31],
        ]);

        let volume_crc32 = u32::from_le_bytes([prefix[32], prefix[33], prefix[34], prefix[35]]);
        let overall_crc32 = u32::from_le_bytes([prefix[36], prefix[37], prefix[38], prefix[39]]);
        let filename_len = u16::from_le_bytes([prefix[40], prefix[41]]) as usize;

        let mut filename_bytes = vec![0u8; filename_len];
        reader.read_exact(&mut filename_bytes)?;

        let mut crc_bytes = [0u8; 4];
        reader.read_exact(&mut crc_bytes)?;
        let expected_header_crc = u32::from_le_bytes(crc_bytes);

        // Verify Header CRC-32 integrity
        let mut hasher = Crc32::new();
        hasher.update(&prefix);
        hasher.update(&filename_bytes);
        let actual_header_crc = hasher.finalize();

        if actual_header_crc != expected_header_crc {
            return Err(HeaderError::HeaderCrcMismatch {
                expected: expected_header_crc,
                actual: actual_header_crc,
            });
        }

        let filename = String::from_utf8(filename_bytes).map_err(HeaderError::InvalidUtf8)?;

        Ok(Self {
            version,
            flags,
            volume_index,
            total_volumes,
            payload_size,
            original_size,
            volume_crc32,
            overall_crc32,
            filename,
        })
    }

    /// Writes this header directly to any `Write` stream.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let serialized = self.serialize();
        writer.write_all(&serialized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_header_roundtrip() {
        let original = DoogHeader::new(
            2,
            5,
            10_485_760,
            45_000_000,
            0x1234_5678,
            0x8765_4321,
            "my_secret_archive.iso".to_string(),
        );

        let serialized = original.serialize();
        let mut cursor = Cursor::new(serialized);
        let decoded = DoogHeader::deserialize(&mut cursor).unwrap();

        assert_eq!(original, decoded);
        assert!(!decoded.is_last_volume());
    }

    #[test]
    fn test_last_volume_flag() {
        let header = DoogHeader::new(
            5,
            5,
            5_000_000,
            45_000_000,
            0xABCD_EF01,
            0x8765_4321,
            "archive.iso".to_string(),
        );

        assert!(header.is_last_volume());
        let serialized = header.serialize();
        let decoded = DoogHeader::deserialize(&mut Cursor::new(serialized)).unwrap();
        assert!(decoded.is_last_volume());
    }

    #[test]
    fn test_corrupt_header_crc_detection() {
        let header = DoogHeader::new(
            1,
            1,
            100,
            100,
            0x1111_2222,
            0x1111_2222,
            "test.bin".to_string(),
        );

        let mut bytes = header.serialize();
        // Flip a byte in the payload size field
        bytes[18] ^= 0xFF;

        let mut cursor = Cursor::new(bytes);
        match DoogHeader::deserialize(&mut cursor) {
            Err(HeaderError::HeaderCrcMismatch { .. }) => {}
            other => panic!("Expected HeaderCrcMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn test_invalid_magic() {
        let mut bytes = vec![0u8; FIXED_PREFIX_SIZE + 10];
        bytes[0..4].copy_from_slice(b"NOPE");
        let mut cursor = Cursor::new(bytes);
        match DoogHeader::deserialize(&mut cursor) {
            Err(HeaderError::InvalidMagic(_)) => {}
            other => panic!("Expected InvalidMagic, got: {:?}", other),
        }
    }
}
