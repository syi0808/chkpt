use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
use std::fs;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    files: usize,
    modified_files: usize,
    dirs: usize,
    iterations: usize,
    include_deps: bool,
    hardlink_fanout: usize,
    break_deps_hardlinks: bool,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            files: 3_000,
            modified_files: 200,
            dirs: 60,
            iterations: 3,
            include_deps: false,
            hardlink_fanout: 1,
            break_deps_hardlinks: false,
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
        "benchmark_config source_files={} deps_files={} modified_files={} dirs={} iterations={} include_deps={} hardlink_fanout={} break_deps_hardlinks={}",
        source_file_count(config),
        deps_file_count(config),
        config.modified_files,
        config.dirs,
        config.iterations,
        u8::from(config.include_deps),
        config.hardlink_fanout,
        u8::from(config.break_deps_hardlinks)
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
        "average cold_save_ms={:.2} warm_save_ms={:.2} incremental_save_ms={:.2} restore_dry_run_ms={:.2} restore_apply_ms={:.2}{}",
        average.cold_save_ms,
        average.warm_save_ms,
        average.incremental_save_ms,
        average.restore_dry_run_ms,
        average.restore_apply_ms,
        peak_rss_suffix()
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
            "--include-deps" => {
                config.include_deps = true;
            }
            "--hardlink-fanout" => {
                if let Some(value) = args.next() {
                    config.hardlink_fanout = value.parse().unwrap_or(config.hardlink_fanout);
                }
            }
            "--break-deps-hardlinks" => {
                config.break_deps_hardlinks = true;
            }
            _ => {}
        }
    }

    config.hardlink_fanout = config.hardlink_fanout.max(1);
    config
}

fn run_iteration(config: BenchConfig) -> Result<Metrics, Box<dyn std::error::Error>> {
    let workspace = TempDir::new()?;
    let store_layout = benchmark_store_layout(workspace.path());
    cleanup_store(store_layout.base_dir())?;

    populate_workspace(workspace.path(), config)?;

    let cold_save = timed(|| save(workspace.path(), save_options(config)))?;
    let baseline_snapshot_id = cold_save.1.snapshot_id;

    let warm_save = timed(|| save(workspace.path(), save_options(config)))?;

    mutate_workspace(workspace.path(), config, 1)?;
    let incremental_save = timed(|| save(workspace.path(), save_options(config)))?;

    mutate_workspace(workspace.path(), config, 2)?;
    if config.break_deps_hardlinks {
        break_deps_hardlinks(workspace.path(), config, 2)?;
    }
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

fn save_options(config: BenchConfig) -> SaveOptions {
    SaveOptions {
        include_deps: config.include_deps,
        ..Default::default()
    }
}

fn populate_workspace(root: &Path, config: BenchConfig) -> std::io::Result<()> {
    for index in 0..source_file_count(config) {
        let path = source_file_path(root, index, config);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, make_content(index, 0))?;
    }

    if !config.include_deps {
        return Ok(());
    }

    for index in 0..deps_file_count(config) {
        let path = deps_file_path(root, index, config);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let group_start = hardlink_group_start(index, config.hardlink_fanout);
        if config.hardlink_fanout > 1 && index != group_start {
            let source = deps_file_path(root, group_start, config);
            fs::hard_link(source, path)?;
        } else {
            fs::write(path, make_content(group_start, 0))?;
        }
    }

    let bin_dir = root.join("node_modules/.bin");
    fs::create_dir_all(&bin_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        for index in 0..deps_file_count(config).min(32) {
            let target = format!(
                "../dir_{:04}/file_{:05}.txt",
                index % config.dirs.max(1),
                index
            );
            symlink(target, bin_dir.join(format!("fixture-{index:02}")))?;
        }
    }
    Ok(())
}

fn mutate_workspace(root: &Path, config: BenchConfig, version: usize) -> std::io::Result<()> {
    for index in 0..config.modified_files.min(source_file_count(config)) {
        let path = source_file_path(root, index, config);
        fs::write(path, make_content(index, version))?;
    }
    std::thread::sleep(Duration::from_millis(5));
    Ok(())
}

fn source_file_count(config: BenchConfig) -> usize {
    config.files
}

fn deps_file_count(config: BenchConfig) -> usize {
    if config.include_deps {
        config.files
    } else {
        0
    }
}

fn source_file_path(root: &Path, index: usize, config: BenchConfig) -> PathBuf {
    root.join("src")
        .join(format!("dir_{:04}", index % config.dirs.max(1)))
        .join(format!("file_{:05}.txt", index))
}

fn deps_file_path(root: &Path, index: usize, config: BenchConfig) -> PathBuf {
    root.join("node_modules")
        .join(format!("dir_{:04}", index % config.dirs.max(1)))
        .join(format!("file_{:05}.txt", index))
}

fn hardlink_group_start(index: usize, hardlink_fanout: usize) -> usize {
    if hardlink_fanout <= 1 {
        index
    } else {
        (index / hardlink_fanout) * hardlink_fanout
    }
}

fn break_deps_hardlinks(root: &Path, config: BenchConfig, version: usize) -> std::io::Result<()> {
    if !config.include_deps || config.hardlink_fanout <= 1 {
        return Ok(());
    }

    let mut broken = 0usize;
    for index in 0..deps_file_count(config) {
        if index == hardlink_group_start(index, config.hardlink_fanout) {
            continue;
        }

        let path = deps_file_path(root, index, config);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::write(path, make_content(index, version + 100))?;
        broken += 1;

        if broken >= config.modified_files {
            break;
        }
    }

    std::thread::sleep(Duration::from_millis(5));
    Ok(())
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

fn peak_rss_suffix() -> String {
    peak_rss_kb()
        .map(|peak_rss_kb| format!(" peak_rss_kb={peak_rss_kb}"))
        .unwrap_or_default()
}

#[cfg(unix)]
fn peak_rss_kb() -> Option<u64> {
    let mut usage = MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };

    #[cfg(target_os = "macos")]
    {
        Some((usage.ru_maxrss as u64).div_ceil(1024))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Some(usage.ru_maxrss as u64)
    }
}

#[cfg(not(unix))]
fn peak_rss_kb() -> Option<u64> {
    None
}
