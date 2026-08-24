mod cli;
mod error;
mod exif;
mod heic;
mod info;
mod pipeline;
mod processor;

use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use pipeline::{build_pipeline, collect_input_files};
use processor::ProcessingContext;

const INFO_OUTPUT_FILE: &str = "bat_img_info.txt";

fn main() -> Result<()> {
    env_logger::init();
    let args = cli::parse();
    if !args.quiet {
        println!(
            "\n  {} {}\n",
            "bat_img_rs".bold().cyan(),
            "— multithreaded batch image processor".dimmed()
        );
    }

    // ── Collect input files ─────────────────────────────────────────────────
    let files = collect_input_files(&args)?;
    if files.is_empty() {
        eprintln!(
            "{} No image files found matching the input pattern.",
            "✖".red()
        );
        std::process::exit(1);
    }

    if !args.quiet {
        println!(
            "  {} {} file(s) found  |  {} thread(s)\n",
            "→".green(),
            files.len().to_string().bold(),
            args.threads.to_string().bold()
        );
    }

    // ── Configure Rayon thread pool ─────────────────────────────────────────
    rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads)
        .build_global()
        .unwrap();

    // ── Progress tracking ───────────────────────────────────────────────────
    let mp = Arc::new(MultiProgress::new());
    let pb_style = ProgressStyle::with_template(
        "  {spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} {wide_msg}",
    )
    .unwrap()
    .progress_chars("█▉▊▋▌▍▎▏ ");

    let pb = mp.add(ProgressBar::new(files.len() as u64));
    pb.set_style(pb_style);

    // ── Display Detailed Info & Exit. Save to INFO_OUTPUT_FILE if --quiet
    if args.info {
        let total = files.len();
        let start = Instant::now();

        // Collect info strings in parallel (no Mutex needed)
        let mut results: Vec<(PathBuf, String)> = files
            .par_iter()
            .map(|file| {
                let info = info::format_info(file);
                pb.inc(1);  // ProgressBar is thread-safe
                (file.clone(), info)
            })
            .collect();

        pb.finish_and_clear();

        // Sort by path for deterministic output
        results.sort_by(|a, b| a.0.cmp(&b.0));

        if args.quiet {
            // Write to file
            let mut content = String::new();
            for (i, (path, info)) in results.iter().enumerate() {
                content.push_str(&format!("{}\n", "─".repeat(60).dimmed()));
                content.push_str(&format!(
                    "{} [{}/{}]\n",
                    path.display().to_string().bold().cyan(),
                    i + 1,
                    total
                ));
                content.push_str(info);
            }
            std::fs::write(INFO_OUTPUT_FILE, content)
                .with_context(|| format!("Failed to write info output to {}", INFO_OUTPUT_FILE))?;
            eprintln!("Info written to {}", INFO_OUTPUT_FILE);
        } else {
            // Print to stdout
            for (i, (path, info)) in results.iter().enumerate() {
                println!("{}", "─".repeat(60).dimmed());
                println!(
                    "{} [{}/{}]",
                    path.display().to_string().bold().cyan(),
                    i + 1,
                    total
                );
                print!("{}", info);
            }
        }
        println!("  {} Done in {:.2?}", "✔".green().bold(), start.elapsed());
        return Ok(());
    }

    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));

    // ── Build the processing pipeline once (shared across threads) ──────────
    let pipeline = Arc::new(build_pipeline(&args)?);
    let args = Arc::new(args);
    let start = Instant::now();

    // ── Parallel processing ─────────────────────────────────────────────────
    files.par_iter().for_each(|input_path| {
        let ctx = ProcessingContext {
            input_path: input_path.clone(),
            pipeline: Arc::clone(&pipeline),
        };

        match ctx.process() {
            Ok(output_path) => {
                success_count.fetch_add(1, Ordering::Relaxed);
                if args.dry_run && !args.quiet {
                    pb.set_message(format!(
                        "[dry-run] {} → {}",
                        input_path.display(),
                        output_path.display()
                    ));
                } else {
                    pb.set_message(format!(
                        "{}",
                        output_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    ));
                }
            }
            Err(e) => {
                failure_count.fetch_add(1, Ordering::Relaxed);
                pb.set_message(format!(
                    "{} {}",
                    "✖".red(),
                    format!("{}: {}", input_path.display(), e).dimmed()
                ));
                if !args.quiet {
                    eprintln!(
                        "\n  {} {} — {}",
                        "Error".red().bold(),
                        input_path.display(),
                        e
                    );
                }
            }
        }

        pb.inc(1);
    });

    pb.finish_and_clear();

    // ── Summary ─────────────────────────────────────────────────────────────
    let elapsed = start.elapsed();
    let ok = success_count.load(Ordering::Relaxed);
    let fail = failure_count.load(Ordering::Relaxed);

    if !args.quiet {
        println!("  {} Done in {:.2?}", "✔".green().bold(), elapsed);
        println!(
            "  {} {} succeeded  {} failed\n",
            "│".dimmed(),
            ok.to_string().green().bold(),
            if fail > 0 {
                fail.to_string().red().bold()
            } else {
                fail.to_string().green().bold()
            }
        );
    }

    if fail > 0 {
        std::process::exit(1);
    }

    Ok(())
}
