mod cli;
mod convert;
mod error;
mod formats;
mod heic;
mod metadata;

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

            if let Err(e) = convert::convert_file(
                &args.input,
                &output,
                format,
                args.quality,
                args.overwrite,
                args.strip_metadata,
            ) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }

            if !cli.quiet {
                println!("Converted {} -> {}", args.input.display(), output.display());
            }
        }
        Command::Strip(args) => {
            if let Err(e) =
                metadata::strip(&args.input, args.output.as_deref())
            {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }

            if !cli.quiet {
                if let Some(ref output) = args.output {
                    println!(
                        "Stripped metadata: {} -> {}",
                        args.input.display(),
                        output.display()
                    );
                } else {
                    println!("Stripped metadata: {}", args.input.display());
                }
            }
        }
        Command::Info(args) => {
            println!("Info: {:?}", args.file);
        }
    }
}
