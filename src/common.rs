use std::fmt;
use std::path::{Path, PathBuf};

/// Buffer size used across all streaming I/O operations (64 KB).
/// This ensures O(1) RAM usage regardless of file size.
pub const STREAM_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug)]
pub enum SizeParseError {
    EmptyString,
    InvalidNumber(String),
    UnknownUnit(String),
    ZeroSize,
}

impl fmt::Display for SizeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeParseError::EmptyString => write!(f, "Size string cannot be empty"),
            SizeParseError::InvalidNumber(s) => write!(f, "Invalid number in size: '{}'", s),
            SizeParseError::UnknownUnit(u) => write!(f, "Unknown size unit: '{}' (use B, K/KB, M/MB, G/GB)", u),
            SizeParseError::ZeroSize => write!(f, "Size must be greater than zero"),
        }
    }
}

impl std::error::Error for SizeParseError {}

/// Parses human-readable size strings such as "10M", "500KB", "2GB", or "1048576".
pub fn parse_size_str(input: &str) -> Result<u64, SizeParseError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(SizeParseError::EmptyString);
    }

    let s_upper = s.to_uppercase();
    let num_end = s_upper
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(s_upper.len());

    let (num_str, unit_str) = s_upper.split_at(num_end);
    let number: u64 = num_str
        .parse()
        .map_err(|_| SizeParseError::InvalidNumber(num_str.to_string()))?;

    if number == 0 {
        return Err(SizeParseError::ZeroSize);
    }

    let multiplier: u64 = match unit_str.trim() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        "T" | "TB" | "TIB" => 1024 * 1024 * 1024 * 1024,
        unknown => return Err(SizeParseError::UnknownUnit(unknown.to_string())),
    };

    Ok(number.saturating_mul(multiplier))
}

/// Formats a byte count into a human-friendly string (e.g. "12.50 MB").
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        format!("{:.2} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Generates a standardized part filename, e.g., `myfile.iso.001`.
/// If the output directory is provided, the part is placed in that directory.
pub fn get_part_path(base_file: &Path, part_num: usize, output_dir: Option<&Path>) -> PathBuf {
    let file_name = base_file
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "part".to_string());

    let part_file_name = format!("{}.{:03}", file_name, part_num);

    if let Some(dir) = output_dir {
        dir.join(part_file_name)
    } else if let Some(parent) = base_file.parent() {
        parent.join(part_file_name)
    } else {
        PathBuf::from(part_file_name)
    }
}

/// Discovers sequential part files on disk starting from part 1 (`.001`).
///
/// Accepts either the path to the first part (e.g. `archive.tar.001`)
/// or the base filename (e.g. `archive.tar`).
pub fn find_sequential_parts(input: &Path) -> std::io::Result<Vec<PathBuf>> {
    let base_path = if let Some(ext) = input.extension().and_then(|e| e.to_str()) {
        if ext.chars().all(|c| c.is_ascii_digit()) {
            // Strip the `.001` or similar numeric extension to get the base path
            input.with_extension("")
        } else {
            input.to_path_buf()
        }
    } else {
        input.to_path_buf()
    };

    let mut parts = Vec::new();
    let mut part_num = 1;

    loop {
        let part_path = get_part_path(&base_path, part_num, None);
        if part_path.is_file() {
            parts.push(part_path);
            part_num += 1;
        } else {
            break;
        }
    }

    if parts.is_empty() {
        // If neither base.001 exists, check if the input itself is a single existing part
        if input.is_file() {
            parts.push(input.to_path_buf());
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No volume parts found for base path: {}", input.display()),
            ));
        }
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_valid() {
        assert_eq!(parse_size_str("100").unwrap(), 100);
        assert_eq!(parse_size_str("100B").unwrap(), 100);
        assert_eq!(parse_size_str("1K").unwrap(), 1024);
        assert_eq!(parse_size_str("2KB").unwrap(), 2048);
        assert_eq!(parse_size_str("5M").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_size_str("1GB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_errors() {
        assert!(parse_size_str("").is_err());
        assert!(parse_size_str("0").is_err());
        assert!(parse_size_str("100XYZ").is_err());
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.00 MB");
    }

    #[test]
    fn test_part_paths() {
        let base = Path::new("data/archive.tar.gz");
        let p1 = get_part_path(base, 1, None);
        assert_eq!(p1, PathBuf::from("data/archive.tar.gz.001"));
        let p2 = get_part_path(base, 42, None);
        assert_eq!(p2, PathBuf::from("data/archive.tar.gz.042"));
    }
}
