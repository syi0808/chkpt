use chkpt_core::store::catalog::{BlobLocation, CatalogSnapshot, ManifestEntry, MetadataCatalog};
use chkpt_core::store::snapshot::SnapshotStats;
use chrono::{Duration, TimeZone, Utc};
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::time::Instant;
use tempfile::TempDir;

#[derive(Debug, Clone, Copy)]
struct BenchConfig {
    manifest_entries: usize,
    blob_count: usize,
    seeded_snapshots: usize,
    iterations: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            manifest_entries: 3_000,
            blob_count: 3_000,
            seeded_snapshots: 64,
            iterations: 5,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Metrics {
    open_ms: f64,
    bulk_upsert_ms: f64,
    insert_snapshot_ms: f64,
    latest_snapshot_ms: f64,
    resolve_prefix_ms: f64,
    list_snapshots_ms: f64,
    snapshot_manifest_ms: f64,
    blob_lookup_ms: f64,
}

impl Metrics {
    fn add_assign(&mut self, other: Metrics) {
        self.open_ms += other.open_ms;
        self.bulk_upsert_ms += other.bulk_upsert_ms;
        self.insert_snapshot_ms += other.insert_snapshot_ms;
        self.latest_snapshot_ms += other.latest_snapshot_ms;
        self.resolve_prefix_ms += other.resolve_prefix_ms;
        self.list_snapshots_ms += other.list_snapshots_ms;
        self.snapshot_manifest_ms += other.snapshot_manifest_ms;
        self.blob_lookup_ms += other.blob_lookup_ms;
    }

    fn divide(self, divisor: f64) -> Self {
        Self {
            open_ms: self.open_ms / divisor,
            bulk_upsert_ms: self.bulk_upsert_ms / divisor,
            insert_snapshot_ms: self.insert_snapshot_ms / divisor,
            latest_snapshot_ms: self.latest_snapshot_ms / divisor,
            resolve_prefix_ms: self.resolve_prefix_ms / divisor,
            list_snapshots_ms: self.list_snapshots_ms / divisor,
            snapshot_manifest_ms: self.snapshot_manifest_ms / divisor,
            blob_lookup_ms: self.blob_lookup_ms / divisor,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args();
    let mut total = Metrics::default();

    println!(
        "benchmark_config manifest_entries={} blob_count={} seeded_snapshots={} iterations={}",
        config.manifest_entries, config.blob_count, config.seeded_snapshots, config.iterations
    );

    for iteration in 0..config.iterations {
        let metrics = run_iteration(config)?;
        total.add_assign(metrics);
        println!(
            "iteration={} open_ms={:.2} bulk_upsert_ms={:.2} insert_snapshot_ms={:.2} latest_snapshot_ms={:.2} resolve_prefix_ms={:.2} list_snapshots_ms={:.2} snapshot_manifest_ms={:.2} blob_lookup_ms={:.2}",
            iteration + 1,
            metrics.open_ms,
            metrics.bulk_upsert_ms,
            metrics.insert_snapshot_ms,
            metrics.latest_snapshot_ms,
            metrics.resolve_prefix_ms,
            metrics.list_snapshots_ms,
            metrics.snapshot_manifest_ms,
            metrics.blob_lookup_ms,
        );
    }

    let average = total.divide(config.iterations as f64);
    println!(
        "average open_ms={:.2} bulk_upsert_ms={:.2} insert_snapshot_ms={:.2} latest_snapshot_ms={:.2} resolve_prefix_ms={:.2} list_snapshots_ms={:.2} snapshot_manifest_ms={:.2} blob_lookup_ms={:.2}{}",
        average.open_ms,
        average.bulk_upsert_ms,
        average.insert_snapshot_ms,
        average.latest_snapshot_ms,
        average.resolve_prefix_ms,
        average.list_snapshots_ms,
        average.snapshot_manifest_ms,
        average.blob_lookup_ms,
        peak_rss_suffix()
    );

    Ok(())
}

fn parse_args() -> BenchConfig {
    let mut config = BenchConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest-entries" => {
                if let Some(value) = args.next() {
                    config.manifest_entries = value.parse().unwrap_or(config.manifest_entries);
                }
            }
            "--blob-count" => {
                if let Some(value) = args.next() {
                    config.blob_count = value.parse().unwrap_or(config.blob_count);
                }
            }
            "--seeded-snapshots" => {
                if let Some(value) = args.next() {
                    config.seeded_snapshots = value.parse().unwrap_or(config.seeded_snapshots);
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

    config.manifest_entries = config.manifest_entries.max(1);
    config.blob_count = config.blob_count.max(1);
    config.seeded_snapshots = config.seeded_snapshots.max(1);
    config.iterations = config.iterations.max(1);
    config
}

fn run_iteration(config: BenchConfig) -> Result<Metrics, Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let catalog_path = tempdir.path().join("catalog.sqlite");
    let manifest = build_manifest(config.manifest_entries, config.blob_count);
    let blob_locations = build_blob_locations(config.blob_count);
    let target_snapshot = build_snapshot(config.seeded_snapshots + 1, config.manifest_entries, config.blob_count);

    let (open_ms, catalog) = timed(|| MetadataCatalog::open(&catalog_path))?;
    seed_snapshots(
        &catalog,
        config.seeded_snapshots,
        &manifest,
        config.blob_count,
    )?;

    let (bulk_upsert_ms, ()) = timed(|| catalog.bulk_upsert_blob_locations(&blob_locations))?;
    let (insert_snapshot_ms, ()) = timed(|| catalog.insert_snapshot(&target_snapshot, &manifest))?;
    let prefix = &target_snapshot.id[..16];
    let (latest_snapshot_ms, _latest) = timed(|| catalog.latest_snapshot())?;
    let (resolve_prefix_ms, resolved) = timed(|| catalog.resolve_snapshot_ref(prefix))?;
    let (list_snapshots_ms, listed) = timed(|| catalog.list_snapshots(Some(config.seeded_snapshots + 1)))?;
    let (snapshot_manifest_ms, loaded_manifest) =
        timed(|| catalog.snapshot_manifest(&target_snapshot.id))?;
    let lookup_hash = manifest[manifest.len() / 2].blob_hash;
    let (blob_lookup_ms, blob_location) = timed(|| catalog.blob_location(&lookup_hash))?;

    assert_eq!(resolved.id, target_snapshot.id);
    assert_eq!(listed.len(), config.seeded_snapshots + 1);
    assert_eq!(loaded_manifest.len(), manifest.len());
    assert!(blob_location.is_some());

    Ok(Metrics {
        open_ms,
        bulk_upsert_ms,
        insert_snapshot_ms,
        latest_snapshot_ms,
        resolve_prefix_ms,
        list_snapshots_ms,
        snapshot_manifest_ms,
        blob_lookup_ms,
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

fn seed_snapshots(
    catalog: &MetadataCatalog,
    count: usize,
    manifest: &[ManifestEntry],
    blob_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    for index in 0..count {
        let snapshot = build_snapshot(index, manifest.len(), blob_count);
        catalog.insert_snapshot(&snapshot, manifest)?;
    }
    Ok(())
}

fn build_manifest(entry_count: usize, blob_count: usize) -> Vec<ManifestEntry> {
    let mut manifest = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        manifest.push(ManifestEntry {
            path: manifest_path(index),
            blob_hash: hash_bytes(index % blob_count),
            size: 4096 + (index % 1024) as u64,
            mode: 0o100644,
        });
    }
    manifest
}

fn build_blob_locations(blob_count: usize) -> Vec<([u8; 32], BlobLocation)> {
    let mut blobs = Vec::with_capacity(blob_count);
    for index in 0..blob_count {
        let pack_hash = if index % 5 == 0 {
            None
        } else {
            Some(format!("pack-{:04x}", index % 512))
        };
        blobs.push((
            hash_bytes(index),
            BlobLocation {
                pack_hash,
                size: 4096 + (index % 1024) as u64,
            },
        ));
    }
    blobs
}

fn build_snapshot(index: usize, entry_count: usize, blob_count: usize) -> CatalogSnapshot {
    let created_at = Utc
        .with_ymd_and_hms(2026, 4, 1, 0, 0, 0)
        .unwrap()
        + Duration::seconds(index as i64);
    CatalogSnapshot {
        id: snapshot_id(index),
        created_at,
        message: Some(format!("snapshot-{index}")),
        parent_snapshot_id: (index > 0).then(|| snapshot_id(index - 1)),
        manifest_snapshot_id: None,
        stats: SnapshotStats {
            total_files: entry_count as u64,
            total_bytes: (entry_count as u64) * 4096,
            new_objects: blob_count.min(entry_count) as u64,
        },
    }
}

fn snapshot_id(index: usize) -> String {
    format!("019d{:08x}-0000-7000-8000-000000000000", index)
}

fn manifest_path(index: usize) -> String {
    let mut path = PathBuf::from("node_modules");
    path.push(format!("pkg_{:05}", index % 2048));
    path.push(format!("dir_{:03}", index % 128));
    path.push(format!("file_{index:06}.js"));
    path.to_string_lossy().into_owned()
}

fn hash_bytes(index: usize) -> [u8; 32] {
    let mut hash = [0u8; 32];
    for (offset, byte) in hash.iter_mut().enumerate() {
        *byte = index.wrapping_mul(131).wrapping_add(offset) as u8;
    }
    hash
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
