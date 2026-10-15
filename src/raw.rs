//! Raw byte-slice splitting and recombining (GNU `split` / `cat` style).
//!
//! In this mode:
//! - Parts contain ONLY raw slices of the original stream, with zero metadata or headers.
//! - Slices can be recombined by simple sequential concatenation.
//! - Memory footprint is strictly O(1) via streaming fixed-size buffers.

use std::cmp;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::common::{get_part_path, STREAM_BUFFER_SIZE};

/// Splits a file into raw sequential chunks of `chunk_size` bytes.
///
/// Each part will be named `<input_filename>.001`, `<input_filename>.002`, etc.
/// Returns the list of created part file paths and the total bytes processed.
pub fn split_raw(
    input_path: &Path,
    chunk_size: u64,
    output_dir: Option<&Path>,
) -> io::Result<(Vec<PathBuf>, u64)> {
    if chunk_size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Chunk size must be greater than 0 bytes",
        ));
    }

    let input_file = File::open(input_path)?;
    let total_size = input_file.metadata()?.len();

    let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, input_file);
    let mut buffer = [0u8; STREAM_BUFFER_SIZE];

    let mut created_parts = Vec::new();
    let mut total_bytes_read = 0u64;
    let mut part_number = 1usize;

    // Handle the special case of an empty (0 byte) file
    if total_size == 0 {
        let part_path = get_part_path(input_path, part_number, output_dir);
        File::create(&part_path)?;
        created_parts.push(part_path);
        return Ok((created_parts, 0));
    }

    while total_bytes_read < total_size {
        let part_path = get_part_path(input_path, part_number, output_dir);
        let out_file = File::create(&part_path)?;
        let mut writer = BufWriter::with_capacity(STREAM_BUFFER_SIZE, out_file);

        let this_part_target = cmp::min(chunk_size, total_size - total_bytes_read);
        let mut part_bytes_written = 0u64;

        while part_bytes_written < this_part_target {
            let to_read = cmp::min(
                buffer.len() as u64,
                this_part_target - part_bytes_written,
            ) as usize;

            let bytes_read = reader.read(&mut buffer[..to_read])?;
            if bytes_read == 0 {
                // Unexpected EOF before reaching expected size
                break;
            }

            writer.write_all(&buffer[..bytes_read])?;
            part_bytes_written += bytes_read as u64;
        }

        writer.flush()?;
        created_parts.push(part_path);
        total_bytes_read += part_bytes_written;
        part_number += 1;
    }

    Ok((created_parts, total_bytes_read))
}

/// Recombines an ordered sequence of raw part files into a single destination file.
///
/// Returns the total bytes written to the restored file.
pub fn join_raw(part_paths: &[PathBuf], output_path: &Path) -> io::Result<u64> {
    if part_paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No part files provided to join",
        ));
    }

    let out_file = File::create(output_path)?;
    let mut writer = BufWriter::with_capacity(STREAM_BUFFER_SIZE, out_file);
    let mut buffer = [0u8; STREAM_BUFFER_SIZE];
    let mut total_written = 0u64;

    for part in part_paths {
        let in_file = File::open(part)?;
        let mut reader = BufReader::with_capacity(STREAM_BUFFER_SIZE, in_file);

        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break; // EOF for this part
            }
            writer.write_all(&buffer[..n])?;
            total_written += n as u64;
        }
    }

    writer.flush()?;
    Ok(total_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_raw_split_and_join_roundtrip() {
        let temp_dir = std::env::temp_dir().join("doogsplit_test_raw");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let source_file = temp_dir.join("sample.txt");
        // Create 250,000 bytes of deterministic test data
        let mut test_data = Vec::with_capacity(250_000);
        for i in 0..250_000 {
            test_data.push((i % 251) as u8);
        }
        fs::write(&source_file, &test_data).unwrap();

        // Split into chunks of 60,000 bytes (should yield 5 parts: 4 * 60k + 1 * 10k)
        let chunk_size = 60_000;
        let (parts, total_split) =
            split_raw(&source_file, chunk_size, Some(&temp_dir)).unwrap();

        assert_eq!(parts.len(), 5);
        assert_eq!(total_split, 250_000);

        // Verify individual part sizes on disk
        assert_eq!(fs::metadata(&parts[0]).unwrap().len(), 60_000);
        assert_eq!(fs::metadata(&parts[1]).unwrap().len(), 60_000);
        assert_eq!(fs::metadata(&parts[2]).unwrap().len(), 60_000);
        assert_eq!(fs::metadata(&parts[3]).unwrap().len(), 60_000);
        assert_eq!(fs::metadata(&parts[4]).unwrap().len(), 10_000);

        // Join parts back together
        let restored_file = temp_dir.join("restored.txt");
        let joined_bytes = join_raw(&parts, &restored_file).unwrap();
        assert_eq!(joined_bytes, 250_000);

        let restored_data = fs::read(&restored_file).unwrap();
        assert_eq!(restored_data, test_data);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
