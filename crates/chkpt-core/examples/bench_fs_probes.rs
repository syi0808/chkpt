use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    paths: usize,
    iterations: usize,
    bytes_per_file: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            paths: 20_000,
            iterations: 20,
            bytes_per_file: 32,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BenchResult {
    read_existing_exists_then_read_ms: f64,
    read_existing_direct_read_ms: f64,
    read_missing_exists_then_read_ms: f64,
    read_missing_direct_read_ms: f64,
    remove_existing_exists_then_remove_ms: f64,
    remove_existing_direct_remove_ms: f64,
    remove_missing_exists_then_remove_ms: f64,
    remove_missing_direct_remove_ms: f64,
}

impl BenchResult {
    fn add_assign(&mut self, other: BenchResult) {
        self.read_existing_exists_then_read_ms += other.read_existing_exists_then_read_ms;
        self.read_existing_direct_read_ms += other.read_existing_direct_read_ms;
        self.read_missing_exists_then_read_ms += other.read_missing_exists_then_read_ms;
        self.read_missing_direct_read_ms += other.read_missing_direct_read_ms;
        self.remove_existing_exists_then_remove_ms += other.remove_existing_exists_then_remove_ms;
        self.remove_existing_direct_remove_ms += other.remove_existing_direct_remove_ms;
        self.remove_missing_exists_then_remove_ms += other.remove_missing_exists_then_remove_ms;
        self.remove_missing_direct_remove_ms += other.remove_missing_direct_remove_ms;
    }

    fn divide(self, divisor: f64) -> Self {
        Self {
            read_existing_exists_then_read_ms: self.read_existing_exists_then_read_ms / divisor,
            read_existing_direct_read_ms: self.read_existing_direct_read_ms / divisor,
            read_missing_exists_then_read_ms: self.read_missing_exists_then_read_ms / divisor,
            read_missing_direct_read_ms: self.read_missing_direct_read_ms / divisor,
            remove_existing_exists_then_remove_ms: self.remove_existing_exists_then_remove_ms
                / divisor,
            remove_existing_direct_remove_ms: self.remove_existing_direct_remove_ms / divisor,
            remove_missing_exists_then_remove_ms: self.remove_missing_exists_then_remove_ms
                / divisor,
            remove_missing_direct_remove_ms: self.remove_missing_direct_remove_ms / divisor,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args();
    let mut total = BenchResult::default();

    for _ in 0..config.iterations {
        total.add_assign(run_iteration(config)?);
    }

    let average = total.divide(config.iterations as f64);
    println!(
        "average read_existing_exists_then_read_ms={:.4} read_existing_direct_read_ms={:.4} read_missing_exists_then_read_ms={:.4} read_missing_direct_read_ms={:.4} remove_existing_exists_then_remove_ms={:.4} remove_existing_direct_remove_ms={:.4} remove_missing_exists_then_remove_ms={:.4} remove_missing_direct_remove_ms={:.4} paths={} iterations={} bytes_per_file={}",
        average.read_existing_exists_then_read_ms,
        average.read_existing_direct_read_ms,
        average.read_missing_exists_then_read_ms,
        average.read_missing_direct_read_ms,
        average.remove_existing_exists_then_remove_ms,
        average.remove_existing_direct_remove_ms,
        average.remove_missing_exists_then_remove_ms,
        average.remove_missing_direct_remove_ms,
        config.paths,
        config.iterations,
        config.bytes_per_file,
    );

    Ok(())
}

fn parse_args() -> BenchConfig {
    let mut config = BenchConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--paths" => {
                if let Some(value) = args.next() {
                    config.paths = value.parse().unwrap_or(config.paths);
                }
            }
            "--iterations" => {
                if let Some(value) = args.next() {
                    config.iterations = value.parse().unwrap_or(config.iterations);
                }
            }
            "--bytes-per-file" => {
                if let Some(value) = args.next() {
                    config.bytes_per_file = value.parse().unwrap_or(config.bytes_per_file);
                }
            }
            _ => {}
        }
    }

    config.paths = config.paths.max(1);
    config.iterations = config.iterations.max(1);
    config
}

fn run_iteration(config: BenchConfig) -> Result<BenchResult, Box<dyn std::error::Error>> {
    let temp = TempDir::new()?;
    let existing_dir = temp.path().join("existing");
    let missing_dir = temp.path().join("missing");
    std::fs::create_dir_all(&existing_dir)?;
    std::fs::create_dir_all(&missing_dir)?;

    let existing_paths = create_files(&existing_dir, config.paths, config.bytes_per_file)?;
    let missing_paths = create_missing_paths(&missing_dir, config.paths);

    let read_existing_exists_then_read_ms =
        elapsed_ms(|| read_existing_exists_then_read(&existing_paths));
    let read_existing_direct_read_ms = elapsed_ms(|| read_existing_direct_read(&existing_paths));
    let read_missing_exists_then_read_ms =
        elapsed_ms(|| read_missing_exists_then_read(&missing_paths));
    let read_missing_direct_read_ms = elapsed_ms(|| read_missing_direct_read(&missing_paths));

    let remove_existing_exists_then_remove_ms = benchmark_remove_existing(
        temp.path(),
        "remove_exists_then_remove",
        config.paths,
        config.bytes_per_file,
        remove_existing_exists_then_remove,
    )?;
    let remove_existing_direct_remove_ms = benchmark_remove_existing(
        temp.path(),
        "remove_direct_remove",
        config.paths,
        config.bytes_per_file,
        remove_existing_direct_remove,
    )?;
    let remove_missing_exists_then_remove_ms =
        elapsed_ms(|| remove_missing_exists_then_remove(&missing_paths));
    let remove_missing_direct_remove_ms = elapsed_ms(|| remove_missing_direct_remove(&missing_paths));

    Ok(BenchResult {
        read_existing_exists_then_read_ms,
        read_existing_direct_read_ms,
        read_missing_exists_then_read_ms,
        read_missing_direct_read_ms,
        remove_existing_exists_then_remove_ms,
        remove_existing_direct_remove_ms,
        remove_missing_exists_then_remove_ms,
        remove_missing_direct_remove_ms,
    })
}

fn create_files(
    dir: &Path,
    count: usize,
    bytes_per_file: usize,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = Vec::with_capacity(count);
    let content = vec![b'x'; bytes_per_file];
    for index in 0..count {
        let path = dir.join(format!("file-{index:06}.bin"));
        std::fs::write(&path, &content)?;
        paths.push(path);
    }
    Ok(paths)
}

fn create_missing_paths(dir: &Path, count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|index| dir.join(format!("missing-{index:06}.bin")))
        .collect()
}

fn benchmark_remove_existing<F>(
    root: &Path,
    label: &str,
    count: usize,
    bytes_per_file: usize,
    mut remover: F,
) -> Result<f64, Box<dyn std::error::Error>>
where
    F: FnMut(&[PathBuf]),
{
    let dir = root.join(label);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    let paths = create_files(&dir, count, bytes_per_file)?;
    Ok(elapsed_ms(|| remover(&paths)))
}

fn elapsed_ms<F>(mut f: F) -> f64
where
    F: FnMut(),
{
    let started = Instant::now();
    f();
    started.elapsed().as_secs_f64() * 1000.0
}

fn read_existing_exists_then_read(paths: &[PathBuf]) {
    let mut total = 0usize;
    for path in paths {
        if path.exists() {
            let bytes = std::fs::read(path).expect("read existing file");
            total += bytes.len();
        }
    }
    std::hint::black_box(total);
}

fn read_existing_direct_read(paths: &[PathBuf]) {
    let mut total = 0usize;
    for path in paths {
        let bytes = std::fs::read(path).expect("read existing file");
        total += bytes.len();
    }
    std::hint::black_box(total);
}

fn read_missing_exists_then_read(paths: &[PathBuf]) {
    let mut misses = 0usize;
    for path in paths {
        if path.exists() {
            let _ = std::fs::read(path).expect("unexpected existing file");
        } else {
            misses += 1;
        }
    }
    std::hint::black_box(misses);
}

fn read_missing_direct_read(paths: &[PathBuf]) {
    let mut misses = 0usize;
    for path in paths {
        match std::fs::read(path) {
            Ok(_) => panic!("unexpected existing file"),
            Err(error) if error.kind() == ErrorKind::NotFound => misses += 1,
            Err(error) => panic!("unexpected read error: {error}"),
        }
    }
    std::hint::black_box(misses);
}

fn remove_existing_exists_then_remove(paths: &[PathBuf]) {
    let mut removed = 0usize;
    for path in paths {
        if path.exists() {
            std::fs::remove_file(path).expect("remove existing file");
            removed += 1;
        }
    }
    std::hint::black_box(removed);
}

fn remove_existing_direct_remove(paths: &[PathBuf]) {
    let mut removed = 0usize;
    for path in paths {
        std::fs::remove_file(path).expect("remove existing file");
        removed += 1;
    }
    std::hint::black_box(removed);
}

fn remove_missing_exists_then_remove(paths: &[PathBuf]) {
    let mut misses = 0usize;
    for path in paths {
        if path.exists() {
            std::fs::remove_file(path).expect("unexpected existing file");
        } else {
            misses += 1;
        }
    }
    std::hint::black_box(misses);
}

fn remove_missing_direct_remove(paths: &[PathBuf]) {
    let mut misses = 0usize;
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => panic!("unexpected existing file"),
            Err(error) if error.kind() == ErrorKind::NotFound => misses += 1,
            Err(error) => panic!("unexpected remove error: {error}"),
        }
    }
    std::hint::black_box(misses);
}
