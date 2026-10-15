//! Custom zero-dependency command line interface parser for `doogsplit`.

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum SplitMode {
    ByBytes(u64),
    ByParts(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Split {
        input: PathBuf,
        mode: SplitMode,
        output_dir: Option<PathBuf>,
        raw: bool,
    },
    Join {
        input: PathBuf,
        output: Option<PathBuf>,
        raw: bool,
    },
    Inspect {
        input: PathBuf,
    },
    Help(Option<String>),
    Version,
}

pub fn print_help(subcommand: Option<&str>) {
    match subcommand {
        Some("split") => {
            println!("Usage: doogsplit split <input_file> (-b <bytes> | -n <parts>) [OPTIONS]");
            println!();
            println!("Splits a file into sequential parts.");
            println!();
            println!("Arguments:");
            println!("  <input_file>         Path to the file to split");
            println!();
            println!("Options:");
            println!("  -b, --bytes <size>   Target chunk size (e.g. 500K, 10M, 1GB, 65536)");
            println!("  -n, --parts <count>  Split evenly into N parts");
            println!("  -o, --output <dir>   Directory to write split parts into (default: same as input)");
            println!("      --raw            Raw byte-slicing mode (GNU split style, no container headers)");
            println!("  -h, --help           Show this help message");
            println!();
            println!("Examples:");
            println!("  doogsplit split backup.tar.gz -b 100M");
            println!("  doogsplit split movie.mkv -n 4 -o ./volumes");
            println!("  doogsplit split archive.zip -b 10M --raw");
        }
        Some("join") => {
            println!("Usage: doogsplit join <first_part_or_base> [OPTIONS]");
            println!();
            println!("Recombines split parts into the original file.");
            println!();
            println!("Arguments:");
            println!("  <first_part>         Path to part .001 or the base filename");
            println!();
            println!("Options:");
            println!("  -o, --output <file>  Target output path (defaults to original filename from header)");
            println!("      --raw            Raw concatenation mode (required if split with --raw)");
            println!("  -h, --help           Show this help message");
            println!();
            println!("Examples:");
            println!("  doogsplit join backup.tar.gz.001");
            println!("  doogsplit join archive.zip.001 --raw -o restored.zip");
        }
        Some("inspect") | Some("verify") => {
            println!("Usage: doogsplit inspect <first_part_or_base>");
            println!();
            println!("Inspects volume headers, structure, and verifies CRC-32 integrity without writing output.");
            println!();
            println!("Arguments:");
            println!("  <first_part>         Path to part .001 or the base filename");
            println!();
            println!("Examples:");
            println!("  doogsplit inspect backup.tar.gz.001");
        }
        _ => {
            println!("doogsplit 0.1.0 - Educational zero-dependency file splitter & joiner");
            println!("Learn how file splitting and recombining works from the ground up.");
            println!();
            println!("Usage: doogsplit <COMMAND> [OPTIONS]");
            println!();
            println!("Commands:");
            println!("  split    Split a file by byte size or number of parts");
            println!("  join     Recombine split parts into the original file");
            println!("  inspect  Inspect headers and verify integrity (CRC-32)");
            println!("  help     Print this message or help for a given subcommand");
            println!();
            println!("Run 'doogsplit <COMMAND> --help' for details on a specific command.");
        }
    }
}

pub fn parse_args<I>(mut args: I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    // Skip program name if present
    let subcmd = match args.next() {
        Some(s) => s,
        None => return Ok(Command::Help(None)),
    };

    match subcmd.as_str() {
        "-h" | "--help" | "help" => {
            let next = args.next();
            Ok(Command::Help(next))
        }
        "-v" | "--version" | "version" => Ok(Command::Version),
        "split" => parse_split(args),
        "join" => parse_join(args),
        "inspect" | "verify" => parse_inspect(args),
        unknown => Err(format!(
            "Unknown command '{}'. Run 'doogsplit --help' for usage.",
            unknown
        )),
    }
}

fn parse_split<I>(mut args: I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    let mut input_path: Option<PathBuf> = None;
    let mut bytes_arg: Option<String> = None;
    let mut parts_arg: Option<usize> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut raw = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(Some("split".to_string()))),
            "-b" | "--bytes" | "--size" => {
                let val = args.next().ok_or_else(|| "Missing value for --bytes".to_string())?;
                bytes_arg = Some(val);
            }
            "-n" | "--parts" => {
                let val = args.next().ok_or_else(|| "Missing value for --parts".to_string())?;
                let count: usize = val
                    .parse()
                    .map_err(|_| format!("Invalid parts count '{}'", val))?;
                if count == 0 {
                    return Err("Number of parts must be greater than 0".to_string());
                }
                parts_arg = Some(count);
            }
            "-o" | "--output" => {
                let val = args.next().ok_or_else(|| "Missing value for --output".to_string())?;
                output_dir = Some(PathBuf::from(val));
            }
            "--raw" => {
                raw = true;
            }
            other if !other.starts_with('-') && input_path.is_none() => {
                input_path = Some(PathBuf::from(other));
            }
            unknown => return Err(format!("Unexpected argument: '{}'", unknown)),
        }
    }

    let input = input_path.ok_or_else(|| "Missing required input file to split".to_string())?;

    let mode = match (bytes_arg, parts_arg) {
        (Some(b), None) => {
            let bytes = crate::common::parse_size_str(&b)
                .map_err(|e| format!("Invalid size specification: {}", e))?;
            SplitMode::ByBytes(bytes)
        }
        (None, Some(p)) => SplitMode::ByParts(p),
        (Some(_), Some(_)) => {
            return Err("Cannot specify both --bytes and --parts at the same time".to_string())
        }
        (None, None) => {
            return Err("Must specify either --bytes <size> or --parts <count>".to_string())
        }
    };

    Ok(Command::Split {
        input,
        mode,
        output_dir,
        raw,
    })
}

fn parse_join<I>(mut args: I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    let mut input_path: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut raw = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(Some("join".to_string()))),
            "-o" | "--output" => {
                let val = args.next().ok_or_else(|| "Missing value for --output".to_string())?;
                output = Some(PathBuf::from(val));
            }
            "--raw" => {
                raw = true;
            }
            other if !other.starts_with('-') && input_path.is_none() => {
                input_path = Some(PathBuf::from(other));
            }
            unknown => return Err(format!("Unexpected argument: '{}'", unknown)),
        }
    }

    let input = input_path.ok_or_else(|| "Missing part file or base path to join".to_string())?;

    Ok(Command::Join {
        input,
        output,
        raw,
    })
}

fn parse_inspect<I>(mut args: I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    let mut input_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help(Some("inspect".to_string()))),
            other if !other.starts_with('-') && input_path.is_none() => {
                input_path = Some(PathBuf::from(other));
            }
            unknown => return Err(format!("Unexpected argument: '{}'", unknown)),
        }
    }

    let input = input_path.ok_or_else(|| "Missing part file to inspect".to_string())?;

    Ok(Command::Inspect { input })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_split_bytes() {
        let args = vec![
            "split".into(),
            "data.bin".into(),
            "-b".into(),
            "10M".into(),
            "--raw".into(),
        ];
        let cmd = parse_args(args.into_iter()).unwrap();
        assert_eq!(
            cmd,
            Command::Split {
                input: PathBuf::from("data.bin"),
                mode: SplitMode::ByBytes(10 * 1024 * 1024),
                output_dir: None,
                raw: true,
            }
        );
    }

    #[test]
    fn test_parse_join() {
        let args = vec![
            "join".into(),
            "data.bin.001".into(),
            "-o".into(),
            "restored.bin".into(),
        ];
        let cmd = parse_args(args.into_iter()).unwrap();
        assert_eq!(
            cmd,
            Command::Join {
                input: PathBuf::from("data.bin.001"),
                output: Some(PathBuf::from("restored.bin")),
                raw: false,
            }
        );
    }
}
