use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Select};

#[derive(Parser)]
#[command(name = "chkpt", about = "Filesystem checkpoint engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a checkpoint of the current workspace
    Save {
        /// Optional message for the checkpoint
        #[arg(short, long)]
        message: Option<String>,
        /// Include dependency directories (node_modules, .venv, etc.)
        #[arg(long)]
        include_deps: bool,
    },
    /// List all checkpoints
    List {
        /// Maximum number of checkpoints to show
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Restore workspace to a checkpoint
    Restore {
        /// Snapshot ID or "latest" (interactive selection if omitted)
        id: Option<String>,
        /// Show what would change without modifying files
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a checkpoint and run garbage collection
    Delete {
        /// Snapshot ID to delete
        id: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let workspace = std::env::current_dir()?;

    match cli.command {
        Commands::Save {
            message,
            include_deps,
        } => {
            let opts = chkpt_core::ops::save::SaveOptions {
                message,
                include_deps,
            };
            let result = chkpt_core::ops::save::save(&workspace, opts)?;
            println!("Checkpoint saved: {}", result.snapshot_id);
            println!(
                "  Files: {}, New objects: {}, Total bytes: {}",
                result.stats.total_files, result.stats.new_objects, result.stats.total_bytes
            );
        }
        Commands::List { limit } => {
            let snapshots = chkpt_core::ops::list::list(&workspace, limit)?;
            if snapshots.is_empty() {
                println!("No checkpoints found.");
            } else {
                println!(
                    "{:<10} {:<22} {:<8} {}",
                    "ID", "Created", "Files", "Message"
                );
                println!("{}", "-".repeat(60));
                for snap in &snapshots {
                    let short_id = &snap.id[..8.min(snap.id.len())];
                    let msg = snap.message.as_deref().unwrap_or("");
                    println!(
                        "{:<10} {:<22} {:<8} {}",
                        short_id,
                        snap.created_at.format("%Y-%m-%d %H:%M:%S"),
                        snap.stats.total_files,
                        msg
                    );
                }
                println!("\n{} checkpoint(s)", snapshots.len());
            }
        }
        Commands::Restore { id, dry_run } => {
            let snapshot_id = match id {
                Some(id) => id,
                None => {
                    let snapshots = chkpt_core::ops::list::list(&workspace, None)?;
                    if snapshots.is_empty() {
                        bail!("No checkpoints found.");
                    }

                    let items: Vec<String> = snapshots
                        .iter()
                        .map(|s| {
                            let short_id = &s.id[..8.min(s.id.len())];
                            let msg = s.message.as_deref().unwrap_or("");
                            format!(
                                "{}  {}  {} files  {}",
                                short_id,
                                s.created_at.format("%Y-%m-%d %H:%M:%S"),
                                s.stats.total_files,
                                if msg.is_empty() {
                                    String::new()
                                } else {
                                    format!("\"{}\"", msg)
                                }
                            )
                        })
                        .collect();

                    let selection = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Select checkpoint to restore")
                        .items(&items)
                        .default(0)
                        .interact()?;

                    snapshots[selection].id.clone()
                }
            };

            let opts = chkpt_core::ops::restore::RestoreOptions { dry_run };
            let result = chkpt_core::ops::restore::restore(&workspace, &snapshot_id, opts)?;
            if dry_run {
                println!("Dry run -- no changes made:");
            } else {
                println!("Restored to checkpoint {}:", result.snapshot_id);
            }
            println!(
                "  Added: {}, Changed: {}, Removed: {}, Unchanged: {}",
                result.files_added,
                result.files_changed,
                result.files_removed,
                result.files_unchanged
            );
        }
        Commands::Delete { id } => {
            chkpt_core::ops::delete::delete(&workspace, &id)?;
            println!("Checkpoint {} deleted.", id);
        }
    }

    Ok(())
}
