use anyhow::{bail, Result};
use chkpt_core::ops::progress::{ProgressCallback, ProgressEvent};
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
        /// Show full snapshot IDs
        #[arg(long)]
        full: bool,
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

fn new_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn new_bar(total: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg} [{bar:30.cyan/dim}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    pb.set_message(msg.to_string());
    pb
}

fn save_progress() -> ProgressCallback {
    if !std::io::stderr().is_terminal() {
        return None;
    }
    let bar: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let bar_ref = bar.clone();
    Some(Box::new(move |event| match event {
        ProgressEvent::ScanComplete { file_count } => {
            let spinner = new_spinner(&format!("Scanned {} files", file_count));
            spinner.finish_and_clear();
        }
        ProgressEvent::ProcessStart { total } => {
            if total > 0 {
                *bar_ref.lock().unwrap() = Some(new_bar(total, "Processing"));
            }
        }
        ProgressEvent::ProcessFile { completed, .. } => {
            if let Some(pb) = bar_ref.lock().unwrap().as_ref() {
                pb.set_position(completed);
            }
        }
        ProgressEvent::PackComplete => {
            if let Some(pb) = bar_ref.lock().unwrap().take() {
                pb.finish_and_clear();
            }
        }
        _ => {}
    }))
}

fn restore_progress() -> ProgressCallback {
    if !std::io::stderr().is_terminal() {
        return None;
    }
    let bar: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let bar_ref = bar.clone();
    Some(Box::new(move |event| match event {
        ProgressEvent::ScanCurrentComplete { file_count } => {
            let spinner = new_spinner(&format!("Scanned {} files", file_count));
            spinner.finish_and_clear();
        }
        ProgressEvent::RestoreStart {
            add,
            change,
            remove,
        } => {
            let total = add + change + remove;
            if total > 0 {
                *bar_ref.lock().unwrap() = Some(new_bar(total, "Restoring"));
            }
        }
        ProgressEvent::RestoreFile { completed, .. } => {
            if let Some(pb) = bar_ref.lock().unwrap().as_ref() {
                pb.set_position(completed);
            }
        }
        _ => {}
    }))
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
                progress: save_progress(),
            };
            let result = chkpt_core::ops::save::save(&workspace, opts)?;
            println!("Checkpoint saved: {}", result.snapshot_id);
            println!(
                "  Files: {}, New objects: {}, Total bytes: {}",
                result.stats.total_files, result.stats.new_objects, result.stats.total_bytes
            );
        }
        Commands::List { limit, full } => {
            let snapshots = chkpt_core::ops::list::list(&workspace, limit)?;
            if snapshots.is_empty() {
                println!("No checkpoints found.");
            } else {
                let id_width = if full { 38 } else { 10 };
                println!(
                    "{:<w$} {:<22} {:<8} Message",
                    "ID",
                    "Created",
                    "Files",
                    w = id_width
                );
                println!("{}", "-".repeat(if full { 86 } else { 60 }));
                for snap in &snapshots {
                    let display_id = if full {
                        snap.id.as_str()
                    } else {
                        &snap.id[..8.min(snap.id.len())]
                    };
                    let msg = snap.message.as_deref().unwrap_or("");
                    println!(
                        "{:<w$} {:<22} {:<8} {}",
                        display_id,
                        snap.created_at.format("%Y-%m-%d %H:%M:%S"),
                        snap.stats.total_files,
                        msg,
                        w = id_width
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

            let opts = chkpt_core::ops::restore::RestoreOptions {
                dry_run,
                progress: if dry_run { None } else { restore_progress() },
            };
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
