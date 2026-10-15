//! Binary multi-volume container splitter, recombiner, and inspector (`DOOG` format).
//!
//! Features:
//! - Preserves original filename and total uncompressed size.
//! - Embeds volume indices and total volume counts to prevent missing or out-of-order parts.
//! - Two-level CRC-32 validation: per-volume payload CRC + full-file overall CRC.
//! - Constant O(1) memory footprint via streaming 64KB buffers.

use std::cmp;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::common::{get_part_path, STREAM_BUFFER_SIZE};
use crate::crc32::Crc32;
use crate::header::{DoogHeader, HeaderError};

#[derive(Debug)]
pub enum ContainerError {
    Io(io::Error),
    Header(HeaderError),
    MissingParts { expected: u32, found: usize },
    VolumeIndexMismatch { expected: u32, actual: u32, path: PathBuf },
    VolumeCountMismatch { expected: u32, actual: u32, path: PathBuf },
    PayloadSizeMismatch { expected: u64, actual: u64, path: PathBuf },
    VolumeCrcMismatch { volume: u32, expected: u32, actual: u32 },
    OverallCrcMismatch { expected: u32, actual: u32 },
    OriginalFileMismatch { expected: String, actual: String, path: PathBuf },
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::Io(e) => write!(f, "I/O error: {}", e),
            ContainerError::Header(e) => write!(f, "Header error: {}", e),
            ContainerError::MissingParts { expected, found } => write!(
                f,
                "Missing volume parts! Expected {} volumes, but found {}",
                expected, found
            ),
            ContainerError::VolumeIndexMismatch { expected, actual, path } => write!(
                f,
                "Volume index mismatch in '{}': expected part {}, but header says {}",
                path.display(),
                expected,
                actual
            ),
            ContainerError::VolumeCountMismatch { expected, actual, path } => write!(
                f,
                "Total volume mismatch in '{}': expected {}, but header says {}",
                path.display(),
                expected,
                actual
            ),
            ContainerError::PayloadSizeMismatch { expected, actual, path } => write!(
                f,
                "Payload size mismatch in '{}': expected {} bytes, but read {}",
                path.display(),
                expected,
                actual
            ),
            ContainerError::VolumeCrcMismatch { volume, expected, actual } => write!(
                f,
                "CRC-32 checksum failed for volume {}! Expected 0x{:08X}, got 0x{:08X} (corrupted data)",
                volume, expected, actual
            ),
            ContainerError::OverallCrcMismatch { expected, actual } => write!(
                f,
                "Overall file CRC-32 checksum failed! Expected 0x{:08X}, got 0x{:08X} (reconstructed file is corrupted)",
                expected, actual
            ),
            ContainerError::OriginalFileMismatch { expected, actual, path } => write!(
                f,
                "Original filename mismatch in '{}': expected '{}', got '{}'",
                path.display(),
                expected,
                actual
            ),
        }
    }
}

impl std::error::Error for ContainerError {}

impl From<io::Error> for ContainerError {
    fn from(e: io::Error) -> Self {
        ContainerError::Io(e)
    }
}

impl From<HeaderError> for ContainerError {
    fn from(e: HeaderError) -> Self {
        ContainerError::Header(e)
    }
}

/// Metadata discovered during Pass 1 for a single chunk.
#[derive(Debug, Clone)]
struct ChunkMeta {
    size: u64,
    crc32: u32,
}

/// Splits an input file into self-contained `DOOG` volume files.
pub fn split_container(
    input_path: &Path,
    chunk_size: u64,
    output_dir: Option<&Path>,
) -> Result<(Vec<PathBuf>, u64), ContainerError> {
    if chunk_size == 0 {
        return Err(ContainerError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Chunk size must be greater than 0",
        )));
    }

    let filename = input_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed.bin".to_string());

    let mut in_file = File::open(input_path)?;
    let total_size = in_file.metadata()?.len();

    // -------------------------------------------------------------
    // PASS 1: Calculate overall CRC and per-chunk CRC & size in one streaming sweep.
    // -------------------------------------------------------------
    let mut overall_hasher = Crc32::new();
    let mut chunks: Vec<ChunkMeta> = Vec::new();

    let mut buf = [0u8; STREAM_BUFFER_SIZE];
    let mut current_chunk_hasher = Crc32::new();
    let mut current_chunk_bytes = 0u64;

    if total_size == 0 {
        // Special case: 0 byte empty file
        chunks.push(ChunkMeta {
            size: 0,
            crc32: 0,
        });
    } else {
        let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, &in_file);

        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }

            overall_hasher.update(&buf[..n]);

            let mut offset = 0;
            while offset < n {
                let needed_for_chunk = chunk_size - current_chunk_bytes;
                let available = (n - offset) as u64;
                let take = cmp::min(needed_for_chunk, available) as usize;

                current_chunk_hasher.update(&buf[offset..offset + take]);
                current_chunk_bytes += take as u64;
                offset += take;

                if current_chunk_bytes == chunk_size {
                    chunks.push(ChunkMeta {
                        size: current_chunk_bytes,
                        crc32: current_chunk_hasher.finalize(),
                    });
                    current_chunk_hasher = Crc32::new();
                    current_chunk_bytes = 0;
                }
            }
        }

        if current_chunk_bytes > 0 {
            chunks.push(ChunkMeta {
                size: current_chunk_bytes,
                crc32: current_chunk_hasher.finalize(),
            });
        }
    }

    let overall_crc = overall_hasher.finalize();
    let total_volumes = chunks.len() as u32;

    // -------------------------------------------------------------
    // PASS 2: Write volume headers and stream chunk payloads.
    // -------------------------------------------------------------
    in_file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, in_file);

    let mut created_parts = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let volume_index = (i + 1) as u32;
        let part_path = get_part_path(input_path, volume_index as usize, output_dir);

        let out_file = File::create(&part_path)?;
        let mut writer = BufWriter::with_capacity(STREAM_BUFFER_SIZE, out_file);

        // Build and write the binary header
        let header = DoogHeader::new(
            volume_index,
            total_volumes,
            chunk.size,
            total_size,
            chunk.crc32,
            overall_crc,
            filename.clone(),
        );
        header.write_to(&mut writer)?;

        // Stream payload bytes
        let mut written_for_chunk = 0u64;
        while written_for_chunk < chunk.size {
            let to_read = cmp::min(
                buf.len() as u64,
                chunk.size - written_for_chunk,
            ) as usize;

            let n = reader.read(&mut buf[..to_read])?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            written_for_chunk += n as u64;
        }

        writer.flush()?;
        created_parts.push(part_path);
    }

    Ok((created_parts, total_size))
}

/// Recombines `DOOG` volume files, verifying headers and CRCs at every stage.
pub fn join_container(
    parts: &[PathBuf],
    output_override: Option<&Path>,
) -> Result<(PathBuf, u64), ContainerError> {
    if parts.is_empty() {
        return Err(ContainerError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No part files provided",
        )));
    }

    // Inspect the first part to discover expected metadata
    let first_file = File::open(&parts[0])?;
    let mut first_reader = BufReader::new(first_file);
    let first_header = DoogHeader::deserialize(&mut first_reader)?;

    if parts.len() != first_header.total_volumes as usize {
        return Err(ContainerError::MissingParts {
            expected: first_header.total_volumes,
            found: parts.len(),
        });
    }

    let final_output_path = match output_override {
        Some(path) => path.to_path_buf(),
        None => {
            // Restore to the original filename in the same directory as part 1
            if let Some(parent) = parts[0].parent() {
                parent.join(&first_header.filename)
            } else {
                PathBuf::from(&first_header.filename)
            }
        }
    };

    let out_file = File::create(&final_output_path)?;
    let mut writer = BufWriter::with_capacity(STREAM_BUFFER_SIZE, out_file);
    let mut overall_hasher = Crc32::new();
    let mut buf = [0u8; STREAM_BUFFER_SIZE];
    let mut total_restored = 0u64;

    for (i, part_path) in parts.iter().enumerate() {
        let expected_index = (i + 1) as u32;
        let part_file = File::open(part_path)?;
        let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, part_file);

        let header = DoogHeader::deserialize(&mut reader)?;

        // Integrity & sequence validations
        if header.volume_index != expected_index {
            return Err(ContainerError::VolumeIndexMismatch {
                expected: expected_index,
                actual: header.volume_index,
                path: part_path.clone(),
            });
        }

        if header.total_volumes != first_header.total_volumes {
            return Err(ContainerError::VolumeCountMismatch {
                expected: first_header.total_volumes,
                actual: header.total_volumes,
                path: part_path.clone(),
            });
        }

        if header.overall_crc32 != first_header.overall_crc32 {
            return Err(ContainerError::OverallCrcMismatch {
                expected: first_header.overall_crc32,
                actual: header.overall_crc32,
            });
        }

        if header.filename != first_header.filename {
            return Err(ContainerError::OriginalFileMismatch {
                expected: first_header.filename.clone(),
                actual: header.filename,
                path: part_path.clone(),
            });
        }

        // Stream and verify volume payload
        let mut vol_hasher = Crc32::new();
        let mut read_for_part = 0u64;

        while read_for_part < header.payload_size {
            let to_read = cmp::min(
                buf.len() as u64,
                header.payload_size - read_for_part,
            ) as usize;

            let n = reader.read(&mut buf[..to_read])?;
            if n == 0 {
                return Err(ContainerError::PayloadSizeMismatch {
                    expected: header.payload_size,
                    actual: read_for_part,
                    path: part_path.clone(),
                });
            }

            vol_hasher.update(&buf[..n]);
            overall_hasher.update(&buf[..n]);
            writer.write_all(&buf[..n])?;
            read_for_part += n as u64;
        }

        let actual_vol_crc = vol_hasher.finalize();
        if actual_vol_crc != header.volume_crc32 {
            return Err(ContainerError::VolumeCrcMismatch {
                volume: expected_index,
                expected: header.volume_crc32,
                actual: actual_vol_crc,
            });
        }

        total_restored += read_for_part;
    }

    writer.flush()?;

    let actual_overall_crc = overall_hasher.finalize();
    if actual_overall_crc != first_header.overall_crc32 {
        return Err(ContainerError::OverallCrcMismatch {
            expected: first_header.overall_crc32,
            actual: actual_overall_crc,
        });
    }

    Ok((final_output_path, total_restored))
}

/// Inspection report for a single volume file.
#[derive(Debug, Clone)]
pub struct VolumeReport {
    pub path: PathBuf,
    pub header: Result<DoogHeader, String>,
    pub actual_payload_size: u64,
    pub actual_volume_crc32: Option<u32>,
    pub is_valid: bool,
    pub error_message: Option<String>,
}

/// Inspects and verifies volume files on disk without writing the reconstructed file.
pub fn inspect_container(parts: &[PathBuf]) -> Vec<VolumeReport> {
    let mut reports = Vec::new();
    let mut buf = [0u8; STREAM_BUFFER_SIZE];

    for part_path in parts {
        let file = match File::open(part_path) {
            Ok(f) => f,
            Err(e) => {
                reports.push(VolumeReport {
                    path: part_path.clone(),
                    header: Err(format!("Could not open file: {}", e)),
                    actual_payload_size: 0,
                    actual_volume_crc32: None,
                    is_valid: false,
                    error_message: Some(format!("Could not open file: {}", e)),
                });
                continue;
            }
        };

        let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, file);

        let header = match DoogHeader::deserialize(&mut reader) {
            Ok(h) => h,
            Err(e) => {
                reports.push(VolumeReport {
                    path: part_path.clone(),
                    header: Err(format!("{}", e)),
                    actual_payload_size: 0,
                    actual_volume_crc32: None,
                    is_valid: false,
                    error_message: Some(format!("Header failure: {}", e)),
                });
                continue;
            }
        };

        // Stream and calculate actual payload CRC
        let mut hasher = Crc32::new();
        let mut actual_size = 0u64;
        let mut read_err = None;

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buf[..n]);
                    actual_size += n as u64;
                }
                Err(e) => {
                    read_err = Some(format!("Error reading payload: {}", e));
                    break;
                }
            }
        }

        let actual_crc = hasher.finalize();

        let is_valid = read_err.is_none()
            && actual_size == header.payload_size
            && actual_crc == header.volume_crc32;

        let error_msg = if let Some(e) = read_err {
            Some(e)
        } else if actual_size != header.payload_size {
            Some(format!(
                "Payload size mismatch: expected {} bytes, read {}",
                header.payload_size, actual_size
            ))
        } else if actual_crc != header.volume_crc32 {
            Some(format!(
                "CRC-32 mismatch: expected 0x{:08X}, computed 0x{:08X}",
                header.volume_crc32, actual_crc
            ))
        } else {
            None
        };

        reports.push(VolumeReport {
            path: part_path.clone(),
            header: Ok(header),
            actual_payload_size: actual_size,
            actual_volume_crc32: Some(actual_crc),
            is_valid,
            error_message: error_msg,
        });
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_container_split_join_and_inspect() {
        let temp_dir = std::env::temp_dir().join("doogsplit_test_container");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let source_file = temp_dir.join("original_document.pdf");
        // Create 180,000 bytes of deterministic test data
        let mut test_data = Vec::with_capacity(180_000);
        for i in 0..180_000 {
            test_data.push((i * 7 % 256) as u8);
        }
        fs::write(&source_file, &test_data).unwrap();

        // Split into chunks of 50,000 bytes (should yield 4 volumes: 3 * 50k + 1 * 30k)
        let chunk_size = 50_000;
        let (parts, total_bytes) =
            split_container(&source_file, chunk_size, Some(&temp_dir)).unwrap();

        assert_eq!(parts.len(), 4);
        assert_eq!(total_bytes, 180_000);

        // Inspect the volumes
        let reports = inspect_container(&parts);
        assert_eq!(reports.len(), 4);
        for report in &reports {
            assert!(report.is_valid, "Report error: {:?}", report.error_message);
            let h = report.header.as_ref().unwrap();
            assert_eq!(h.filename, "original_document.pdf");
            assert_eq!(h.total_volumes, 4);
        }

        // Join the volumes back together into a separate output directory
        let restored_dir = temp_dir.join("restored_output");
        fs::create_dir_all(&restored_dir).unwrap();
        let target_output = restored_dir.join("restored_doc.pdf");

        let (restored_path, restored_bytes) =
            join_container(&parts, Some(&target_output)).unwrap();

        assert_eq!(restored_path, target_output);
        assert_eq!(restored_bytes, 180_000);

        let restored_data = fs::read(&restored_path).unwrap();
        assert_eq!(restored_data, test_data);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_corrupted_volume_detected() {
        let temp_dir = std::env::temp_dir().join("doogsplit_test_corruption");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let source_file = temp_dir.join("data.bin");
        fs::write(&source_file, b"Quick brown fox 1234567890 hello doogsplit").unwrap();

        let (parts, _) = split_container(&source_file, 15, Some(&temp_dir)).unwrap();
        assert!(parts.len() > 1);

        // Corrupt the payload of part 2
        let mut part2_bytes = fs::read(&parts[1]).unwrap();
        // The last byte of part 2 is in the payload
        let last_idx = part2_bytes.len() - 1;
        part2_bytes[last_idx] ^= 0xFF;
        fs::write(&parts[1], &part2_bytes).unwrap();

        // Joining should now fail with VolumeCrcMismatch
        let restored_target = temp_dir.join("restored.bin");
        let result = join_container(&parts, Some(&restored_target));
        match result {
            Err(ContainerError::VolumeCrcMismatch { volume, .. }) => {
                assert_eq!(volume, 2);
            }
            other => panic!("Expected VolumeCrcMismatch for volume 2, got: {:?}", other),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
