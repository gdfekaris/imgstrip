mod cli;
mod convert;
mod error;
mod formats;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Convert(args) => {
            println!("Convert: {:?} -> {}", args.input, args.format);
            if let Some(ref output) = args.output {
                println!("  Output: {:?}", output);
            }
            println!("  Quality: {}", args.quality);
            println!("  Strip metadata: {}", args.strip_metadata);
            println!("  Recursive: {}", args.recursive);
            println!("  Overwrite: {}", args.overwrite);
            println!("  Dry run: {}", args.dry_run);
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
