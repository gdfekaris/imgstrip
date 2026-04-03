mod batch;
mod cli;
mod convert;
mod error;
mod formats;
mod heic;
mod info;
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

            if args.input.is_dir() {
                let operation = batch::Operation::Convert {
                    format,
                    quality: args.quality,
                    overwrite: args.overwrite,
                    strip_metadata: args.strip_metadata,
                };
                let options = batch::BatchOptions {
                    recursive: args.recursive,
                    dry_run: args.dry_run,
                    output_dir: args.output,
                    verbose: cli.verbose,
                };
                match batch::process_directory(&args.input, &operation, &options) {
                    Ok(report) => print_report(&report, cli.quiet),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
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
        }
        Command::Strip(args) => {
            if args.input.is_dir() {
                let operation = batch::Operation::Strip;
                let options = batch::BatchOptions {
                    recursive: args.recursive,
                    dry_run: args.dry_run,
                    output_dir: args.output,
                    verbose: cli.verbose,
                };
                match batch::process_directory(&args.input, &operation, &options) {
                    Ok(report) => print_report(&report, cli.quiet),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                if let Err(e) = metadata::strip(&args.input, args.output.as_deref()) {
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
        }
        Command::Info(args) => {
            if let Err(e) = info::display_info(&args.file) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn print_report(report: &batch::BatchReport, quiet: bool) {
    if !quiet {
        println!("Processed {} file(s) successfully", report.succeeded);
    }

    if !report.failed.is_empty() {
        for (path, error) in &report.failed {
            eprintln!("Error: {}: {error}", path.display());
        }
        if !quiet {
            eprintln!("{} file(s) failed", report.failed.len());
        }
        std::process::exit(1);
    }
}
