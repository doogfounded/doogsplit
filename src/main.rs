use std::fs;
use std::process;

mod cli;
mod common;
mod container;
mod crc32;
mod header;
mod raw;

use cli::{Command, SplitMode};
use common::{find_sequential_parts, format_bytes};

fn main() {
    let args = std::env::args().skip(1);
    let command = match cli::parse_args(args) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("Error: {}", err);
            eprintln!("Run 'doogsplit --help' for usage instructions.");
            process::exit(1);
        }
    };

    if let Err(err) = execute(command) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}

fn execute(cmd: Command) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Command::Help(subcmd) => {
            cli::print_help(subcmd.as_deref());
        }
        Command::Version => {
            println!("doogsplit version 0.1.0");
        }
        Command::Split {
            input,
            mode,
            output_dir,
            raw,
        } => {
            if !input.is_file() {
                return Err(format!("Input file not found: '{}'", input.display()).into());
            }

            let meta = fs::metadata(&input)?;
            let total_size = meta.len();

            let chunk_size = match mode {
                SplitMode::ByBytes(bytes) => bytes,
                SplitMode::ByParts(parts) => {
                    if total_size == 0 {
                        1
                    } else {
                        // Integer ceiling division: ceil(total_size / parts)
                        (total_size + (parts as u64) - 1) / (parts as u64)
                    }
                }
            };

            println!(
                "Splitting '{}' ({}) into {} chunks...",
                input.display(),
                format_bytes(total_size),
                format_bytes(chunk_size)
            );
            println!("Mode: {}", if raw { "Raw Byte-Slices (no headers)" } else { "DOOG Container (headers + CRC-32)" });

            if let Some(ref dir) = output_dir {
                fs::create_dir_all(dir)?;
            }

            let (parts, processed_bytes) = if raw {
                raw::split_raw(&input, chunk_size, output_dir.as_deref())?
            } else {
                container::split_container(&input, chunk_size, output_dir.as_deref())?
            };

            println!(
                "Successfully generated {} volume part(s) (total payload: {}):",
                parts.len(),
                format_bytes(processed_bytes)
            );
            for part in &parts {
                let part_size = fs::metadata(part)?.len();
                println!("  - {} ({})", part.display(), format_bytes(part_size));
            }
        }
        Command::Join { input, output, raw } => {
            let parts = find_sequential_parts(&input)?;
            println!("Discovered {} sequential part file(s) to join:", parts.len());
            for part in &parts {
                println!("  - {}", part.display());
            }

            println!("Mode: {}", if raw { "Raw Concatenation" } else { "DOOG Container Validation" });

            if raw {
                let out_path = match output {
                    Some(p) => p,
                    None => {
                        return Err(
                            "In --raw mode, you must explicitly specify target output file with -o/--output"
                                .into(),
                        );
                    }
                };

                let written = raw::join_raw(&parts, &out_path)?;
                println!(
                    "Successfully reconstructed '{}' ({}) via raw byte streaming.",
                    out_path.display(),
                    format_bytes(written)
                );
            } else {
                let (restored_path, written) =
                    container::join_container(&parts, output.as_deref())?;
                println!(
                    "Verification Passed: Header, Volume CRC-32, and Overall CRC-32 validated successfully!"
                );
                println!(
                    "Successfully reconstructed '{}' ({}).",
                    restored_path.display(),
                    format_bytes(written)
                );
            }
        }
        Command::Inspect { input } => {
            let parts = find_sequential_parts(&input)?;
            println!("Inspecting {} volume file(s)...", parts.len());
            println!();

            let reports = container::inspect_container(&parts);
            let mut all_ok = true;

            for (idx, report) in reports.iter().enumerate() {
                println!("Volume Part #{}: {}", idx + 1, report.path.display());
                match &report.header {
                    Ok(header) => {
                        println!("  Format Version   : {}", header.version);
                        println!("  Volume Index     : {} of {}", header.volume_index, header.total_volumes);
                        println!("  Is Last Volume   : {}", if header.is_last_volume() { "Yes" } else { "No" });
                        println!("  Original Filename: {}", header.filename);
                        println!("  Original Size    : {} ({} bytes)", format_bytes(header.original_size), header.original_size);
                        println!("  Payload Size     : {} ({} bytes)", format_bytes(header.payload_size), header.payload_size);
                        println!("  Payload On Disk  : {} bytes", report.actual_payload_size);
                        println!("  Volume CRC-32    : 0x{:08X}", header.volume_crc32);
                        println!("  Overall CRC-32   : 0x{:08X}", header.overall_crc32);

                        if let Some(actual_crc) = report.actual_volume_crc32 {
                            println!("  Computed CRC-32  : 0x{:08X}", actual_crc);
                        }

                        if report.is_valid {
                            println!("  Integrity Check  : [PASS] Volume payload and header CRC valid");
                        } else {
                            all_ok = false;
                            println!(
                                "  Integrity Check  : [FAIL] {}",
                                report.error_message.as_deref().unwrap_or("Integrity check failed")
                            );
                        }
                    }
                    Err(e) => {
                        println!("  Header Status    : [ERROR] {}", e);
                        all_ok = false;
                    }
                }
                println!();
            }

            if all_ok {
                println!("All {} volume(s) verified successfully. Archive set is intact!", reports.len());
            } else {
                return Err("One or more volumes failed verification or had header corruption!".into());
            }
        }
    }

    Ok(())
}
