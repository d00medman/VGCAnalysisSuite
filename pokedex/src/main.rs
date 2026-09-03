use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pokedex::{migrate, regulation, Db};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pokedex", about = "SQLite pokedex schema and regulation-aware ingest")]
struct Cli {
    /// Database file. Defaults to the container state boundary.
    #[arg(long, env = "POKEDEX_DB", default_value = "/data/pokedex.db", global = true)]
    db: PathBuf,

    /// Apply pending migrations automatically before running the command.
    #[arg(long, env = "POKEDEX_AUTO_MIGRATE", global = true)]
    auto_migrate: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the database and apply all migrations.
    Init,
    /// Apply pending migrations.
    Migrate,
    /// Show schema version and row counts.
    Status,
    /// Manage regulations.
    #[command(subcommand)]
    Regulation(RegulationCmd),
}

#[derive(Subcommand)]
enum RegulationCmd {
    /// Add a regulation. Dates must be ISO-8601; ordering is derived from them.
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        effective_from: String,
        #[arg(long)]
        notes: Option<String>,
    },
    /// List regulations with their derived time windows.
    List,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(parent) = cli.db.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }

    let mut db = Db::open(&cli.db).with_context(|| format!("opening {}", cli.db.display()))?;

    if matches!(cli.command, Command::Init | Command::Migrate) || cli.auto_migrate {
        let applied = db.migrate()?;
        if !applied.is_empty() {
            for name in &applied {
                println!("applied {name}");
            }
        }
    }

    match cli.command {
        Command::Init | Command::Migrate => {
            println!("schema version {}", db.schema_version()?);
        }
        Command::Status => status(&db)?,
        Command::Regulation(cmd) => {
            db.require_current_schema()?;
            match cmd {
                RegulationCmd::Add { name, effective_from, notes } => {
                    let id = regulation::add(
                        db.conn(),
                        &name,
                        &effective_from,
                        notes.as_deref(),
                    )?;
                    println!("added regulation {id}: {name} effective {effective_from}");
                }
                RegulationCmd::List => {
                    for w in regulation::windows(db.conn())? {
                        println!(
                            "{:<24} {} -> {}",
                            w.name,
                            w.effective_from,
                            w.effective_to.as_deref().unwrap_or("(current)")
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn status(db: &Db) -> Result<()> {
    let version = db.schema_version()?;
    println!("schema version : {version} / {}", migrate::LATEST_VERSION);
    let pending = migrate::pending(db.conn())?;
    if !pending.is_empty() {
        println!("pending        : {}", pending.join(", "));
    }
    if version == 0 {
        return Ok(());
    }
    for table in [
        "regulation", "type", "type_chart_rule", "nature", "variant_kind",
        "pokemon", "variant", "ability", "move", "pokemon_stats", "move_data",
        "pokemon_type", "pokemon_ability", "pokemon_move",
    ] {
        let n: i64 = db
            .conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        println!("{table:<16}: {n}");
    }
    Ok(())
}
