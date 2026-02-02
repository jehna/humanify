//! humanify CLI - JavaScript deobfuscator using local LLM inference
//!
//! Usage:
//!   humanify local <input>     Process a file with local LLM
//!   humanify download <model>  Download a model (2b or 8b)

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use humanify_rs::llm::{
    local_rename, ModelConfig, Prompt, DEFAULT_CONTEXT_WINDOW_SIZE, MODELS,
};
use humanify_rs::llm::{ensure_model_directory, get_model_definition, get_model_path, is_model_downloaded};

/// JavaScript deobfuscator using local LLM inference
#[derive(Parser)]
#[command(name = "humanify")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Use a local LLM to unminify code
    Local {
        /// The input minified JavaScript file
        input: PathBuf,

        /// The output file (defaults to <input>.humanified.js)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// The model to use (2b or 8b)
        #[arg(short, long, default_value = "2b")]
        model: String,

        /// Seed for reproducible results
        #[arg(short, long)]
        seed: Option<u32>,

        /// Disable GPU acceleration
        #[arg(long)]
        disable_gpu: bool,

        /// Context window size for surrounding code
        #[arg(long, default_value_t = DEFAULT_CONTEXT_WINDOW_SIZE)]
        context_size: usize,

        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Download a model for local inference
    Download {
        /// The model to download (2b or 8b)
        model: String,

        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },

    /// List available models
    Models,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Local {
            input,
            output,
            model,
            seed,
            disable_gpu,
            context_size,
            verbose,
        } => {
            if let Err(e) = run_local(input, output, model, seed, disable_gpu, context_size, verbose) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Download { model, verbose } => {
            if let Err(e) = run_download(&model, verbose) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Models => {
            println!("Available models:");
            for model in MODELS {
                let status = if is_model_downloaded(model.name).unwrap_or(false) {
                    "downloaded"
                } else {
                    "not downloaded"
                };
                println!("  {} - {} ({})", model.name, model.filename, status);
            }
        }
    }
}

fn run_local(
    input: PathBuf,
    output: Option<PathBuf>,
    model: String,
    seed: Option<u32>,
    disable_gpu: bool,
    context_size: usize,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate model exists
    get_model_definition(&model)?;

    // Check if model is downloaded
    if !is_model_downloaded(&model)? {
        return Err(format!(
            "Model '{}' is not downloaded. Run: humanify download {}",
            model, model
        )
        .into());
    }

    // Read input file
    let code = fs::read_to_string(&input)?;

    if verbose {
        println!("Processing {} ({} bytes)", input.display(), code.len());
        println!("Model: {}", model);
        println!("Context size: {}", context_size);
    }

    // Configure model
    let mut config = ModelConfig::new(&model);
    if let Some(s) = seed {
        config = config.with_seed(s);
    }
    if disable_gpu {
        config = config.with_gpu_disabled();
    }

    // Load model and create prompt
    println!("Loading model...");
    let prompt = Prompt::new(config)?;

    // Create progress bar
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {percent}% {msg}")?
            .progress_chars("=>-"),
    );

    // Process code
    println!("Processing code...");
    let result = local_rename(&prompt, &code, context_size, Some(|progress: f64| {
        pb.set_position((progress * 100.0) as u64);
    }))?;

    pb.finish_with_message("Done!");

    // Determine output path
    let output_path = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let parent = input.parent().unwrap_or(std::path::Path::new("."));
        parent.join(format!("{}.humanified.js", stem))
    });

    // Write output
    fs::write(&output_path, &result)?;
    println!("Output written to: {}", output_path.display());

    Ok(())
}

fn run_download(model: &str, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Validate model exists
    let definition = get_model_definition(model)?;

    // Check if already downloaded
    if is_model_downloaded(model)? {
        println!("Model '{}' is already downloaded.", model);
        return Ok(());
    }

    // Ensure model directory exists
    ensure_model_directory()?;

    let model_path = get_model_path(model)?;
    let temp_path = model_path.with_extension("gguf.part");

    if verbose {
        println!("Downloading from: {}", definition.url);
        println!("Saving to: {}", model_path.display());
    }

    println!("Downloading model '{}'...", model);

    // Download with progress
    let response = ureq::get(definition.url).call()?;

    let total_size = response
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let pb = if total_size > 0 {
        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
                .progress_chars("=>-"),
        );
        Some(pb)
    } else {
        println!("Downloading (unknown size)...");
        None
    };

    // Read response body and write to file
    let mut file = fs::File::create(&temp_path)?;
    let mut reader = response.into_body().into_reader();
    let mut buffer = [0u8; 8192];
    let mut downloaded: u64 = 0;

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])?;
        downloaded += bytes_read as u64;
        if let Some(ref pb) = pb {
            pb.set_position(downloaded);
        }
    }

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete!");
    }

    // Rename temp file to final path
    fs::rename(&temp_path, &model_path)?;
    println!("Model '{}' downloaded to: {}", model, model_path.display());

    Ok(())
}
