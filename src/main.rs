mod cli;
mod error;
mod exif;
mod heic;
mod pipeline;
mod processor;

use anyhow::Result;
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use pipeline::{build_pipeline, collect_input_files};
use processor::ProcessingContext;

const TAB_WIDTH: usize = 15;

fn main() -> Result<()> {
    env_logger::init();

    let args = cli::parse();

    // ── Banner ──────────────────────────────────────────────────────────────
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

    // ── Display Detailed Info & Exit ─────────────────────────────────────────
    let width = TAB_WIDTH;
    if args.info {
        let total = files.len();
        for (i, file) in files.iter().enumerate() {
            // Separator between files
            println!("{}", "─".repeat(60).dimmed());
            println!(
                "{} [{}/{}]",
                file.display().to_string().bold().cyan(),
                i + 1,
                total
            );
            println!("{}", "─".repeat(60).dimmed());

            // 1. File System Metadata
            if let Ok(metadata) = std::fs::metadata(file) {
                let bytes = metadata.len();
                let readable_size = if bytes >= 1_048_576 {
                    format!("{:.2} MB", bytes as f64 / 1_048_576.0)
                } else if bytes >= 1024 {
                    format!("{:.2} KB", bytes as f64 / 1024.0)
                } else {
                    format!("{} B", bytes)
                };
                println!(
                    "  {:<width$} : {} ({})",
                    "File Size".bold(),
                    readable_size,
                    format!("{} bytes", bytes).dimmed()
                );
            }

            // 2. Format & Dimensions
            let is_heic_file = heic::is_heic(file);
            let format_str = if is_heic_file {
                "HEIC".to_string()
            } else {
                image::ImageFormat::from_path(file)
                    .map(|f| format!("{:?}", f))
                    .unwrap_or_else(|_| "Unknown".to_string())
            };
            println!("  {:<width$} : {}", "Format".bold(), format_str.blue());

            // Extract dimensions and color type (handling HEIC via libheif decode)
            let (dimensions, color_type) = if is_heic_file {
                if let Ok((img, _, _)) = heic::decode(file) {
                    (Some((img.width(), img.height())), Some(img.color()))
                } else {
                    (None, None)
                }
            } else {
                let dims = image::ImageReader::open(file)
                    .ok()
                    .and_then(|r| r.with_guessed_format().ok())
                    .and_then(|d| d.into_dimensions().ok());
                let color = image::open(file).ok().map(|img| img.color());
                (dims, color)
            };
            if let Some((w, h)) = dimensions {
                let megapixels = (w as f64 * h as f64) / 1_000_000.0;
                println!(
                    "  {:<width$} : {}x{} ({:.2} MP)",
                    "Dimensions".bold(),
                    w,
                    h,
                    megapixels
                );
            }
            if let Some(color) = color_type {
                println!("  {:<width$} : {:?}", "Color Type".bold(), color);
            }

            // 3. EXIF Data
            println!("\n  {}", "[ EXIF Metadata ]".bold().underline().dimmed());
            if let Some(exif_data) = exif::read_exif(file) {
                let mut found_any = false;

                if let Some(ref make) = exif_data.make {
                    println!("    {:<width$} : {}", "Make", make);
                    found_any = true;
                }
                if let Some(ref model) = exif_data.model {
                    println!("    {:<width$} : {}", "Model", model);
                    found_any = true;
                }
                if let Some(ref date) = exif_data.date_time {
                    println!("    {:<width$} : {}", "Date/Time", date);
                    found_any = true;
                }
                if let Some(ref iso) = exif_data.iso {
                    println!("    {:<width$} : ISO {}", "ISO Speed", iso);
                    found_any = true;
                }
                if let Some(ref exp) = exif_data.exposure {
                    let clean_exp = exp.trim().trim_end_matches('s').trim();
                    println!("    {:<width$} : {} s", "Exposure", clean_exp);
                    found_any = true;
                }
                if let Some(ref f) = exif_data.f_number {
                    let clean_f = f.trim().trim_start_matches("f/").trim_start_matches('f').trim();
                    let formatted_f = if let Ok(val) = clean_f.parse::<f64>() {
                        format!("{:.2}", val).trim_end_matches('0').trim_end_matches('.').to_string()
                    } else {
                        clean_f.to_string()
                    };
                    println!("    {:<width$} : f/{}", "Aperture", formatted_f);
                    found_any = true;
                }
                if let Some(ref fl) = exif_data.focal_length {
                    let clean_fl = fl.trim().trim_end_matches("mm").trim();
                    let formatted_fl = if let Ok(val) = clean_fl.parse::<f64>() {
                        format!("{:.2} mm", val)
                    } else {
                        format!("{} mm", clean_fl)
                    };
                    println!("    {:<width$} : {}", "Focal Length", formatted_fl);
                    found_any = true;
                }
                if exif_data.gps_present {
                    println!("    {:<width$} : {}", "GPS Data".red(), "Present");
                    found_any = true;
                }

                if !found_any {
                    println!("    {}", "No standard camera tags found in EXIF.".dimmed());
                }
            } else {
                println!("    {}", "None (or unreadable EXIF header)".dimmed());
            }
            println!(); // Blank spacing line between files
        }
        return Ok(());
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
