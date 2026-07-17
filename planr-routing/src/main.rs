use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use planr_routing::{
    RegistrySignature, catalog_json, compile_json, evaluate_policy, list_policies, probe_host,
    show_policy, sign_registry, verify_registry_signature,
};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "planr-routing", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Policy(PolicyArgs),
    Compile(CompileArgs),
    Probe(ProbeArgs),
    Evaluate(PolicySelector),
    Catalog(CatalogArgs),
    Registry(RegistryArgs),
}

#[derive(Args)]
struct PolicyArgs {
    #[command(subcommand)]
    command: PolicyCommand,
}

#[derive(Subcommand)]
enum PolicyCommand {
    List,
    Show(PolicySelector),
}

#[derive(Args)]
struct PolicySelector {
    policy: String,
    #[arg(long)]
    host: String,
}

#[derive(Args)]
struct CompileArgs {
    policy: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct ProbeArgs {
    host: String,
    #[arg(long)]
    command: Option<String>,
}

#[derive(Args)]
struct CatalogArgs {
    #[command(subcommand)]
    command: CatalogCommand,
}

#[derive(Subcommand)]
enum CatalogCommand {
    Build(OutputArgs),
    Verify(FileArgs),
}

#[derive(Args)]
struct RegistryArgs {
    #[command(subcommand)]
    command: RegistryCommand,
}

#[derive(Subcommand)]
enum RegistryCommand {
    Sign(SignArgs),
    Verify(VerifyArgs),
}

#[derive(Args)]
struct OutputArgs {
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct FileArgs {
    file: PathBuf,
}

#[derive(Args)]
struct SignArgs {
    file: PathBuf,
    #[arg(long)]
    signer: String,
    #[arg(long)]
    private_key_file: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Args)]
struct VerifyArgs {
    file: PathBuf,
    #[arg(long)]
    signature: PathBuf,
    #[arg(long)]
    trusted_signer: String,
    #[arg(long)]
    trusted_public_key_file: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Policy(args) => match args.command {
            PolicyCommand::List => println!("{}", serde_json::to_string_pretty(&list_policies()?)?),
            PolicyCommand::Show(selector) => println!(
                "{}",
                serde_json::to_string_pretty(&show_policy(&selector.policy, &selector.host)?)?
            ),
        },
        Command::Compile(args) => {
            let output = compile_json(&args.policy, &args.host)?;
            if let Some(path) = args.output {
                fs::write(path, output)?;
            } else {
                print!("{output}");
            }
        }
        Command::Probe(args) => println!(
            "{}",
            serde_json::to_string_pretty(&probe_host(&args.host, args.command.as_deref())?)?
        ),
        Command::Evaluate(selector) => println!(
            "{}",
            serde_json::to_string_pretty(&evaluate_policy(&selector.policy, &selector.host)?)?
        ),
        Command::Catalog(args) => match args.command {
            CatalogCommand::Build(args) => {
                let catalog = catalog_json()?;
                if let Some(output) = args.output {
                    fs::write(output, catalog)?;
                } else {
                    print!("{catalog}");
                }
            }
            CatalogCommand::Verify(args) => {
                let current = fs::read_to_string(args.file)?;
                if current != catalog_json()? {
                    anyhow::bail!("catalog does not match package-owned generated sources");
                }
                println!("catalog verified");
            }
        },
        Command::Registry(args) => match args.command {
            RegistryCommand::Sign(args) => {
                let content = fs::read(&args.file)?;
                let private_key = fs::read_to_string(args.private_key_file)?;
                let signature = sign_registry(&content, &args.signer, &private_key)?;
                fs::write(args.output, serde_json::to_vec_pretty(&signature)?)?;
            }
            RegistryCommand::Verify(args) => {
                let content = fs::read(args.file)?;
                let signature: RegistrySignature =
                    serde_json::from_slice(&fs::read(args.signature)?)?;
                let trusted_public_key = fs::read_to_string(args.trusted_public_key_file)?;
                verify_registry_signature(
                    &content,
                    &signature,
                    &args.trusted_signer,
                    &trusted_public_key,
                )?;
                println!("registry signature verified");
            }
        },
    }
    Ok(())
}
