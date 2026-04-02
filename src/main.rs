mod cli;
mod convert;
mod error;
mod formats;
mod heic;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Convert(args) => {
            let format = match formats::parse_output_format(&args.format) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };

            let output = args
                .output
                .unwrap_or_else(|| convert::derive_output_path(&args.input, format));

            if let Err(e) =
                convert::convert_file(&args.input, &output, format, args.quality, args.overwrite)
            {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }

            if !cli.quiet {
                println!("Converted {} -> {}", args.input.display(), output.display());
            }
        }
        Command::Strip(args) => {
            println!("Strip: {:?}", args.input);
            if let Some(ref output) = args.output {
                println!("  Output: {:?}", output);
            }
            println!("  Recursive: {}", args.recursive);
            println!("  Dry run: {}", args.dry_run);
        }
        Command::Info(args) => {
            println!("Info: {:?}", args.file);
        }
    }
}
