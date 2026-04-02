use chkpt_core::store::snapshot::{Snapshot, SnapshotStats, SnapshotStore};
use std::time::Instant;
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    snapshots: usize,
    iterations: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            snapshots: 1_000,
            iterations: 50,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args();
    let workspace = TempDir::new()?;
    let store = SnapshotStore::new(workspace.path().to_path_buf());

    for index in 0..config.snapshots {
        let snapshot = Snapshot::new(
            Some(format!("snapshot-{index}")),
            [index as u8; 32],
            None,
            SnapshotStats {
                total_files: index as u64,
                total_bytes: (index as u64) * 4096,
                new_objects: 1,
            },
        );
        store.save(&snapshot)?;
    }

    let started = Instant::now();
    for _ in 0..config.iterations {
        let latest = store.latest()?.expect("snapshots seeded");
        std::hint::black_box(latest.id);
    }
    let latest_ms = started.elapsed().as_secs_f64() * 1000.0 / config.iterations as f64;

    println!(
        "average latest_ms={:.4} snapshots={} iterations={}",
        latest_ms, config.snapshots, config.iterations
    );
    Ok(())
}

fn parse_args() -> BenchConfig {
    let mut config = BenchConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--snapshots" => {
                if let Some(value) = args.next() {
                    config.snapshots = value.parse().unwrap_or(config.snapshots);
                }
            }
            "--iterations" => {
                if let Some(value) = args.next() {
                    config.iterations = value.parse().unwrap_or(config.iterations);
                }
            }
            _ => {}
        }
    }

    config.snapshots = config.snapshots.max(1);
    config.iterations = config.iterations.max(1);
    config
}
