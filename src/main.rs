mod pipe;

use clap::{error::ErrorKind, Parser, Subcommand};
use std::path::PathBuf;

const EXIT_LLM_RUNTIME: i32 = 1;
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

fn run_passthrough(args: &SubArgs) -> anyhow::Result<()> {
    // TODO(task-10): wire LlmRenamer + Ladder
    let contents =
        pipe::read_input(&args.input).map_err(|e| anyhow::anyhow!("failed to read input: {e}"))?;
    pipe::write_output(args.output.as_deref(), &contents)
        .map_err(|e| anyhow::anyhow!("failed to write output: {e}"))?;
    Ok(())
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

    let result = match &cli.command {
        Commands::Openai(args) => run_passthrough(args),
        Commands::Gemini(args) => run_passthrough(args),
        Commands::Anthropic(args) => run_passthrough(args),
        Commands::Ollama(args) => run_passthrough(args),
        Commands::Openrouter(args) => run_passthrough(args),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(EXIT_LLM_RUNTIME);
    }
}
