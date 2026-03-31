use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
use chkpt_core::store::blob::BlobStore;
use chkpt_core::store::pack::pack_loose_objects;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    files: usize,
    modified_files: usize,
    dirs: usize,
    iterations: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            files: 3_000,
            modified_files: 200,
            dirs: 60,
            iterations: 3,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Metrics {
    cold_save_ms: f64,
    warm_save_ms: f64,
    incremental_save_ms: f64,
    restore_dry_run_ms: f64,
    restore_apply_ms: f64,
}

impl Metrics {
    fn add_assign(&mut self, other: Metrics) {
        self.cold_save_ms += other.cold_save_ms;
        self.warm_save_ms += other.warm_save_ms;
        self.incremental_save_ms += other.incremental_save_ms;
        self.restore_dry_run_ms += other.restore_dry_run_ms;
        self.restore_apply_ms += other.restore_apply_ms;
    }

    fn divide(self, divisor: f64) -> Self {
        Self {
            cold_save_ms: self.cold_save_ms / divisor,
            warm_save_ms: self.warm_save_ms / divisor,
            incremental_save_ms: self.incremental_save_ms / divisor,
            restore_dry_run_ms: self.restore_dry_run_ms / divisor,
            restore_apply_ms: self.restore_apply_ms / divisor,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args();
    let mut total = Metrics::default();

    println!(
        "benchmark_config files={} modified_files={} dirs={} iterations={}",
        config.files, config.modified_files, config.dirs, config.iterations
    );

    for iteration in 0..config.iterations {
        let metrics = run_iteration(config)?;
        total.add_assign(metrics);
        println!(
            "iteration={} cold_save_ms={:.2} warm_save_ms={:.2} incremental_save_ms={:.2} restore_dry_run_ms={:.2} restore_apply_ms={:.2}",
            iteration + 1,
            metrics.cold_save_ms,
            metrics.warm_save_ms,
            metrics.incremental_save_ms,
            metrics.restore_dry_run_ms,
            metrics.restore_apply_ms
        );
    }

    let average = total.divide(config.iterations as f64);
    println!(
        "average cold_save_ms={:.2} warm_save_ms={:.2} incremental_save_ms={:.2} restore_dry_run_ms={:.2} restore_apply_ms={:.2}",
        average.cold_save_ms,
        average.warm_save_ms,
        average.incremental_save_ms,
        average.restore_dry_run_ms,
        average.restore_apply_ms
    );

    Ok(())
}

fn parse_args() -> BenchConfig {
    let mut config = BenchConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--files" => {
                if let Some(value) = args.next() {
                    config.files = value.parse().unwrap_or(config.files);
                }
            }
            "--modified-files" => {
                if let Some(value) = args.next() {
                    config.modified_files = value.parse().unwrap_or(config.modified_files);
                }
            }
            "--dirs" => {
                if let Some(value) = args.next() {
                    config.dirs = value.parse().unwrap_or(config.dirs);
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

    config
}

fn run_iteration(config: BenchConfig) -> Result<Metrics, Box<dyn std::error::Error>> {
    let workspace = TempDir::new()?;
    let store_layout = benchmark_store_layout(workspace.path());
    cleanup_store(store_layout.base_dir())?;

    populate_workspace(workspace.path(), config.files, config.dirs, 0)?;

    let cold_save = timed(|| save(workspace.path(), SaveOptions::default()))?;
    let baseline_snapshot_id = cold_save.1.snapshot_id;
    let blob_store = BlobStore::new(store_layout.objects_dir());
    if !blob_store.list_loose()?.is_empty() {
        pack_loose_objects(&blob_store, &store_layout.packs_dir())?;
    }

    let warm_save = timed(|| save(workspace.path(), SaveOptions::default()))?;

    mutate_workspace(workspace.path(), config.modified_files, config.dirs, 1)?;
    let incremental_save = timed(|| save(workspace.path(), SaveOptions::default()))?;

    mutate_workspace(workspace.path(), config.modified_files, config.dirs, 2)?;
    let restore_dry_run = timed(|| {
        restore(
            workspace.path(),
            &baseline_snapshot_id,
            RestoreOptions {
                dry_run: true,
                ..Default::default()
            },
        )
    })?;

    let restore_apply = timed(|| {
        restore(
            workspace.path(),
            &baseline_snapshot_id,
            RestoreOptions {
                dry_run: false,
                ..Default::default()
            },
        )
    })?;

    cleanup_store(store_layout.base_dir())?;

    Ok(Metrics {
        cold_save_ms: cold_save.0,
        warm_save_ms: warm_save.0,
        incremental_save_ms: incremental_save.0,
        restore_dry_run_ms: restore_dry_run.0,
        restore_apply_ms: restore_apply.0,
    })
}

fn timed<T, E, F>(mut operation: F) -> Result<(f64, T), E>
where
    F: FnMut() -> Result<T, E>,
{
    let started = Instant::now();
    let result = operation()?;
    Ok((started.elapsed().as_secs_f64() * 1000.0, result))
}

fn populate_workspace(
    root: &Path,
    files: usize,
    dirs: usize,
    version: usize,
) -> std::io::Result<()> {
    for index in 0..files {
        let path = file_path(root, index, dirs);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, make_content(index, version))?;
    }
    Ok(())
}

fn mutate_workspace(
    root: &Path,
    modified_files: usize,
    dirs: usize,
    version: usize,
) -> std::io::Result<()> {
    for index in 0..modified_files {
        fs::write(file_path(root, index, dirs), make_content(index, version))?;
    }
    std::thread::sleep(Duration::from_millis(5));
    Ok(())
}

fn file_path(root: &Path, index: usize, dirs: usize) -> PathBuf {
    root.join(format!("dir_{:04}", index % dirs.max(1)))
        .join(format!("file_{:05}.txt", index))
}

fn make_content(index: usize, version: usize) -> String {
    let body = "x".repeat(4096 + version * 17 + (index % 31));
    format!("file={index}\nversion={version}\n{body}")
}

fn benchmark_store_layout(workspace_root: &Path) -> StoreLayout {
    let project_id = project_id_from_path(workspace_root);
    StoreLayout::new(&project_id)
}

fn cleanup_store(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}
