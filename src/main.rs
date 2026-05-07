use clap::{error::ErrorKind, Parser, Subcommand};
use humanify::cli::openai;
use std::path::PathBuf;

const EXIT_CLI_USAGE: i32 = 64;

#[derive(Parser)]
#[command(
    name = "humanify",
    version,
    about = "Un-minify JavaScript with LLM help"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Openai(SubArgs),
    Gemini(SubArgs),
    Anthropic(SubArgs),
    Ollama(SubArgs),
    Openrouter(SubArgs),
}

#[derive(Parser)]
struct SubArgs {
    /// Filename, or `-` for stdin
    input: String,

    /// Output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Override preset's default model
    #[arg(short, long)]
    model: Option<String>,

    /// Override env-var-based API key
    #[arg(short = 'k', long)]
    api_key: Option<String>,

    /// Override preset's base URL
    #[arg(long)]
    base_url: Option<String>,

    /// Surrounding code chars per identifier
    #[arg(long, default_value_t = 500)]
    context_size: usize,

    /// JSON strategy mode
    #[arg(long, default_value = "ladder")]
    json_mode: String,

    /// Debug log to stderr
    #[arg(short, long)]
    verbose: bool,
}

fn into_openai_args(a: SubArgs) -> openai::Args {
    openai::Args {
        input: a.input,
        output: a.output,
        model: a.model,
        api_key: a.api_key,
        base_url: a.base_url,
        context_size: a.context_size,
        json_mode: a.json_mode,
        verbose: a.verbose,
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                e.exit();
            }
            _ => {
                let _ = e.print();
                std::process::exit(EXIT_CLI_USAGE);
            }
        },
    };

    let exit_code = match cli.command {
        Commands::Openai(args) => openai::run(into_openai_args(args)),
        Commands::Gemini(args) => openai::run(into_openai_args(args)),
        Commands::Anthropic(args) => openai::run(into_openai_args(args)),
        Commands::Ollama(args) => openai::run(into_openai_args(args)),
        Commands::Openrouter(args) => openai::run(into_openai_args(args)),
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
