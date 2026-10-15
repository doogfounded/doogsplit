//! A zero-dependency, table-driven IEEE 802.3 CRC-32 implementation.
//!
//! # How CRC-32 Works:
//! Cyclic Redundancy Check treats incoming data as coefficients of a polynomial
//! over GF(2) (binary arithmetic with XOR addition and no carries).
//!
//! The IEEE 802.3 standard uses the reversed polynomial `0xEDB88320`.
//! By processing one byte at a time through a 256-entry lookup table, we avoid
//! 8 bitwise loops per byte, making it fast and efficient for streaming file I/O.

/// Precomputed 256-entry lookup table generated at compile-time using a `const fn`.
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub const CRC32_TABLE: [u32; 256] = make_table();

/// Incremental CRC-32 hasher for streaming data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// Initializes a new CRC-32 hasher with the standard pre-inversion value `0xFFFFFFFF`.
    pub fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    /// Incrementally updates the CRC-32 state with a slice of bytes.
    #[inline]
    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let index = ((self.state ^ (byte as u32)) & 0xFF) as usize;
            self.state = (self.state >> 8) ^ CRC32_TABLE[index];
        }
    }

    /// Finalizes the calculation by applying post-inversion (`XOR 0xFFFFFFFF`)
    /// and returning the 32-bit checksum.
    pub fn finalize(self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }

    /// Convenience helper to compute CRC-32 over a complete buffer in one call.
    pub fn compute(data: &[u8]) -> u32 {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_buffer() {
        assert_eq!(Crc32::compute(b""), 0x0000_0000);
    }

    #[test]
    fn test_standard_test_vector() {
        // Standard check vector for IEEE 802.3 CRC-32:
        // ASCII "123456789" => 0xCBF43926
        assert_eq!(Crc32::compute(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_fox_sentence() {
        // Known test vector for "The quick brown fox jumps over the lazy dog"
        assert_eq!(
            Crc32::compute(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn test_incremental_equals_monolithic() {
        let message = b"Hello world, testing incremental CRC32 hashing for doogsplit!";
        let direct = Crc32::compute(message);

        let mut streaming = Crc32::new();
        streaming.update(&message[..10]);
        streaming.update(&message[10..25]);
        streaming.update(&message[25..]);
        assert_eq!(streaming.finalize(), direct);
    }
}
