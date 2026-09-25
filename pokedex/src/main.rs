use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pokedex::model::Snapshot;
use pokedex::{ingest, migrate, regulation, Db};
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pokedex", about = "Postgres pokedex schema and regulation-aware ingest")]
struct Cli {
    /// Postgres connection URL, e.g. postgres://pokedex:pokedex@localhost:5432/pokedex
    #[arg(long, env = "DATABASE_URL", hide_env_values = true, global = true)]
    database_url: String,

    /// Apply pending migrations automatically before running the command.
    #[arg(
        long,
        env = "POKEDEX_AUTO_MIGRATE",
        value_parser = truthy,
        num_args = 0..=1,
        default_value = "false",
        default_missing_value = "true",
        global = true
    )]
    auto_migrate: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply all migrations to an empty database.
    Init,
    /// Apply pending migrations.
    Migrate,
    /// Show schema version and row counts.
    Status,
    /// Manage regulations.
    #[command(subcommand)]
    Regulation(RegulationCmd),
    /// Apply a complete snapshot for one regulation.
    ///
    /// The snapshot is treated as COMPLETE: any set-valued field that is present but
    /// omits an entry closes that entry's interval. Omit the field entirely (JSON
    /// `null` or absent) to leave stored data untouched.
    Import {
        /// Regulation name; overrides the snapshot's own `regulation` field.
        #[arg(long)]
        regulation: Option<String>,
        /// JSON file. Reads stdin when absent or when given as `-`.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Apply, report, then roll back.
        #[arg(long)]
        dry_run: bool,
    },
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

    // Not echoing the URL on failure: it carries the password.
    let mut db = Db::connect(&cli.database_url).context("connecting to postgres")?;

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
        Command::Status => status(&mut db)?,
        Command::Import { regulation, file, dry_run } => {
            db.require_current_schema()?;
            let raw = match file.as_deref() {
                None => read_stdin()?,
                Some(p) if p.as_os_str() == "-" => read_stdin()?,
                Some(p) => std::fs::read_to_string(p)
                    .with_context(|| format!("reading {}", p.display()))?,
            };
            let mut snap: Snapshot =
                serde_json::from_str(&raw).context("parsing snapshot JSON")?;
            if let Some(name) = regulation {
                snap.regulation = name;
            }

            let report = if dry_run {
                // Roll back by applying inside a transaction that is never committed.
                let mut tx = db.client().transaction()?;
                let r = ingest::apply_in(&mut tx, &snap)?;
                tx.rollback()?;
                r
            } else {
                ingest::apply(db.client(), &snap)?
            };

            println!("regulation      : {}", snap.regulation);
            println!("pokemon         : {} upserted", report.pokemon_upserted);
            println!("variants        : {} upserted", report.variants_upserted);
            println!(
                "stats           : {} written, {} unchanged",
                report.stats_written, report.stats_unchanged
            );
            println!(
                "move data       : {} written, {} unchanged",
                report.move_data_written, report.move_data_unchanged
            );
            println!(
                "types           : {} opened, {} closed",
                report.types_opened, report.types_closed
            );
            println!(
                "abilities       : {} opened, {} closed",
                report.abilities_opened, report.abilities_closed
            );
            println!(
                "learnset        : {} opened, {} closed",
                report.learnset_opened, report.learnset_closed
            );
            println!(
                "items           : {} upserted, {} made legal, {} made illegal",
                report.items_upserted, report.items_legal_opened, report.items_legal_closed
            );
            if report.is_noop() {
                println!("=> no-op: snapshot matched stored state exactly");
            }
            if dry_run {
                println!("=> dry run, rolled back");
            }
        }
        Command::Regulation(cmd) => {
            db.require_current_schema()?;
            match cmd {
                RegulationCmd::Add { name, effective_from, notes } => {
                    let id = regulation::add(
                        db.client(),
                        &name,
                        &effective_from,
                        notes.as_deref(),
                    )?;
                    println!("added regulation {id}: {name} effective {effective_from}");
                }
                RegulationCmd::List => {
                    for w in regulation::windows(db.client())? {
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

/// Accept the shell conventions for booleans, not just clap's `true`/`false`.
/// `POKEDEX_AUTO_MIGRATE=1` is what anyone writing a compose file will reach for.
fn truthy(s: &str) -> std::result::Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" | "" => Ok(false),
        other => Err(format!(
            "expected a boolean (1/0, true/false, yes/no, on/off), got {other:?}"
        )),
    }
}

fn read_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).context("reading stdin")?;
    Ok(s)
}

fn status(db: &mut Db) -> Result<()> {
    let version = db.schema_version()?;
    println!("schema version : {version} / {}", migrate::LATEST_VERSION);
    let pending = migrate::pending(db.client())?;
    if !pending.is_empty() {
        println!("pending        : {}", pending.join(", "));
    }
    if version == 0 {
        return Ok(());
    }
    for table in [
        "regulation", "type", "type_chart_rule", "nature", "variant_kind",
        "pokemon", "variant", "ability", "move", "pokemon_stats", "move_data",
        "pokemon_type", "pokemon_ability", "pokemon_move", "item", "item_legality",
        "video", "battle", "transcript", "transcript_line",
    ] {
        let n: i64 = db
            .client()
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])?
            .get(0);
        println!("{table:<16}: {n}");
    }
    Ok(())
}
