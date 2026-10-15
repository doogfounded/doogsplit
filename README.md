# doogsplit

A zero-dependency file splitter, recombiner, and integrity inspector built from scratch in Rust.

`doogsplit` demonstrates the low-level mechanics behind file splitting tools like GNU `split` and multi-volume archive formats like 7-Zip (`.7z.001`), RAR, and ZIP.

---

## Key Concepts & Architecture

### 1. Constant Memory Footprint ($O(1)$ RAM)
Instead of buffering whole files into memory, `doogsplit` streams data through fixed-size $64\text{ KB}$ user-space buffers (`STREAM_BUFFER_SIZE`) using `std::io::BufReader` and `std::io::BufWriter`. Files of any size (from a few bytes to hundreds of gigabytes) can be split and joined with minimal memory usage.

### 2. The Two Splitting Modes

| Feature | Raw Mode (`--raw`) | Container Mode (Default) |
|---|---|---|
| **Analogy** | GNU `split` / `cat` | 7-Zip (`.7z.001`) / WinRAR |
| **Overhead** | 0 bytes | ~46 bytes + filename per volume |
| **Integrity Checks** | None (silent corruption possible) | Header CRC-32 + Volume payload CRC-32 + Full file CRC-32 |
| **Volume Tracking** | Relies entirely on filenames | Embedded volume indices (`i of N`) prevent missing/swapped parts |
| **Reconstruction** | Requires manual output filename | Automatically preserves and restores original filename |

---

## Binary Header Format (`DOOG` v1)

In container mode, every volume file begins with a structured binary header (encoded in Little-Endian):

```text
Offset  Size  Field          Description
-------------------------------------------------------------
0x00    4     magic          Fixed ASCII string: "DOOG"
0x04    2     version        Format version (currently 1)
0x06    2     flags          Bit 0: IS_LAST_VOLUME
0x08    4     volume_index   1-based index (1 ..= total_volumes)
0x0C    4     total_volumes  Total number of volumes in the split set
0x10    8     payload_size   Bytes of raw payload in this volume
0x18    8     original_size  Total size in bytes of the original file
0x20    4     volume_crc32   CRC-32 checksum of this volume's payload
0x24    4     overall_crc32  CRC-32 checksum of the complete original file
0x28    2     filename_len   Length in bytes of original filename (N)
0x2A    N     filename       UTF-8 encoded original filename
0x2A+N  4     header_crc32   CRC-32 checksum of all preceding header bytes
```

---

## Handcrafted CRC-32 Engine

`doogsplit` does not use any third-party crates (`crc32fast`, etc.). It includes a compile-time generated 256-entry lookup table for the standard IEEE 802.3 reversed polynomial (`0xEDB88320`):

* **Pre-inversion:** Initial state `0xFFFFFFFF`
* **Table indexing:** `((state ^ byte) & 0xFF)`
* **Post-inversion:** `state ^ 0xFFFFFFFF`
* **Test Vector Verified:** ASCII `"123456789"` produces `0xCBF43926`.

---

## Building and Testing

```bash
# Run the 17 built-in unit and integration tests
cargo test

# Build optimized release binary
cargo build --release
```

Binary location: `target/release/doogsplit` (or `doogsplit.exe` on Windows).

---

## Usage Examples

### 1. Split by Size (Container Mode)
```bash
# Split an ISO into 100 MB volumes
doogsplit split ubuntu.iso -b 100M

# Supports units: B, K/KB, M/MB, G/GB
doogsplit split database.sql -b 500K -o ./parts
```

### 2. Split into $N$ Equal Parts
```bash
# Split evenly into 4 parts
doogsplit split large_video.mp4 -n 4
```

### 3. Inspect Volumes & Verify Integrity
```bash
# Verify headers, volume indices, and payload CRCs without writing output
doogsplit inspect large_video.mp4.001
```

### 4. Recombine / Join Volumes
```bash
# Automatically reconstructs the file with its original filename
doogsplit join large_video.mp4.001

# Or specify a custom output destination
doogsplit join large_video.mp4.001 -o /path/to/restored.mp4
```

### 5. Raw GNU Split Mode
```bash
# Split with zero metadata
doogsplit split archive.tar -b 10M --raw

# Join raw slices
doogsplit join archive.tar.001 --raw -o restored.tar
```
