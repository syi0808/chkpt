//! Benchmark: BLAKE3 vs XXH3-128 and zstd vs LZ4 comparison.
//!
//! Usage:
//!   cargo bench -p chkpt-core --bench hash_compress_compare -- /path/to/target/dir
//!
//! Reads all files from the given directory, then measures:
//!   1. Hash throughput: BLAKE3 vs XXH3-128 (by file size bucket)
//!   2. Compression throughput: zstd(1) vs LZ4
//!   3. Decompression throughput: zstd vs LZ4
//!   4. Compression ratio: zstd(1) vs LZ4
//!   5. Full pipeline: read→hash→compress

use chkpt_core::scanner::{scan_workspace_with_options, ScannedFile};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn read_file(s: &ScannedFile) -> Vec<u8> {
    let mut file = std::fs::File::open(&s.absolute_path).unwrap();
    let mut buf = Vec::with_capacity(s.size as usize);
    file.read_to_end(&mut buf).unwrap();
    buf
}

fn parallel_map<T: Send + Sync, R: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    if items.is_empty() {
        return Vec::new();
    }
    let results: Arc<Mutex<Vec<(usize, R)>>> = Arc::new(Mutex::new(Vec::with_capacity(items.len())));
    let chunk_size = items.len().div_ceil(threads.max(1));

    std::thread::scope(|scope| {
        for (chunk_idx, chunk) in items.chunks(chunk_size).enumerate() {
            let results = Arc::clone(&results);
            let f = &f;
            scope.spawn(move || {
                for (i, item) in chunk.iter().enumerate() {
                    let r = f(item);
                    results.lock().unwrap().push((chunk_idx * chunk_size + i, r));
                }
            });
        }
    });

    let mut results = Arc::try_unwrap(results).ok().unwrap().into_inner().unwrap();
    results.sort_unstable_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target_dir = args
        .get(1)
        .expect("Usage: cargo bench --bench hash_compress_compare -- /path/to/dir");

    let target = std::path::Path::new(target_dir);
    println!("Target: {}", target.display());

    let scanned = scan_workspace_with_options(target, None, false).unwrap();
    let refs: Vec<&ScannedFile> = scanned.iter().filter(|s| !s.is_symlink && s.size > 0).collect();
    let total_bytes: u64 = refs.iter().map(|s| s.size).sum();

    println!(
        "Files: {}  Total: {:.1} MB",
        refs.len(),
        total_bytes as f64 / 1_048_576.0
    );

    let small: Vec<&ScannedFile> = refs.iter().filter(|s| s.size < 1024).copied().collect();
    let medium: Vec<&ScannedFile> = refs
        .iter()
        .filter(|s| s.size >= 1024 && s.size < 256 * 1024)
        .copied()
        .collect();
    let large: Vec<&ScannedFile> = refs
        .iter()
        .filter(|s| s.size >= 256 * 1024)
        .copied()
        .collect();

    println!(
        "Buckets: small(<1KB)={}, medium(1-256KB)={}, large(>=256KB)={}",
        small.len(),
        medium.len(),
        large.len()
    );

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    // ── 1. Hash throughput ──
    println!();
    println!("=== Hash Throughput ({} threads) ===", threads);

    for (label, bucket) in [
        ("small", &small),
        ("medium", &medium),
        ("large", &large),
        ("all", &refs),
    ] {
        if bucket.is_empty() {
            continue;
        }
        let bucket_bytes: u64 = bucket.iter().map(|s| s.size).sum();

        // Pre-read all files into memory so timing reflects hash-only cost,
        // not I/O, and both algorithms see equally warm buffers.
        let bucket_contents: Vec<Vec<u8>> = bucket.iter().map(|s| read_file(s)).collect();

        let t = Instant::now();
        let _: Vec<[u8; 32]> = parallel_map(&bucket_contents, threads, |content| {
            *blake3::hash(content).as_bytes()
        });
        let blake3_ms = t.elapsed().as_millis();

        let t = Instant::now();
        let _: Vec<u128> = parallel_map(&bucket_contents, threads, |content| {
            xxhash_rust::xxh3::xxh3_128(content)
        });
        let xxh3_ms = t.elapsed().as_millis();

        let speedup = if xxh3_ms > 0 {
            (blake3_ms as f64 / xxh3_ms as f64 - 1.0) * 100.0
        } else {
            f64::INFINITY
        };
        let mb = bucket_bytes as f64 / 1_048_576.0;
        println!(
            "  {:<8} BLAKE3: {:>5}ms ({:.0} MB/s)  XXH3: {:>5}ms ({:.0} MB/s)  {:>+.1}%",
            label,
            blake3_ms,
            if blake3_ms > 0 {
                mb / (blake3_ms as f64 / 1000.0)
            } else {
                0.0
            },
            xxh3_ms,
            if xxh3_ms > 0 {
                mb / (xxh3_ms as f64 / 1000.0)
            } else {
                0.0
            },
            speedup,
        );
    }

    // ── 2. Compression throughput ──
    println!();
    println!("=== Compression Throughput ({} threads) ===", threads);

    let all_contents: Vec<Vec<u8>> = refs.iter().map(|s| read_file(s)).collect();

    let t = Instant::now();
    let zstd_compressed: Vec<Vec<u8>> = parallel_map(&all_contents, threads, |content| {
        zstd::encode_all(&content[..], 1).unwrap()
    });
    let zstd_comp_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let lz4_compressed: Vec<Vec<u8>> = parallel_map(&all_contents, threads, |content| {
        lz4_flex::compress_prepend_size(content)
    });
    let lz4_comp_ms = t.elapsed().as_millis();

    let zstd_total: u64 = zstd_compressed.iter().map(|c| c.len() as u64).sum();
    let lz4_total: u64 = lz4_compressed.iter().map(|c| c.len() as u64).sum();
    let mb = total_bytes as f64 / 1_048_576.0;

    let comp_speedup = if lz4_comp_ms > 0 {
        (zstd_comp_ms as f64 / lz4_comp_ms as f64 - 1.0) * 100.0
    } else {
        f64::INFINITY
    };
    println!(
        "  zstd(1): {:>5}ms ({:.0} MB/s)  ratio: {:.2}",
        zstd_comp_ms,
        if zstd_comp_ms > 0 {
            mb / (zstd_comp_ms as f64 / 1000.0)
        } else {
            0.0
        },
        total_bytes as f64 / zstd_total as f64,
    );
    println!(
        "  LZ4:     {:>5}ms ({:.0} MB/s)  ratio: {:.2}  speedup: {:>+.1}%",
        lz4_comp_ms,
        if lz4_comp_ms > 0 {
            mb / (lz4_comp_ms as f64 / 1000.0)
        } else {
            0.0
        },
        total_bytes as f64 / lz4_total as f64,
        comp_speedup,
    );
    println!(
        "  Size delta: zstd={:.1}MB  LZ4={:.1}MB  ({:>+.1}%)",
        zstd_total as f64 / 1_048_576.0,
        lz4_total as f64 / 1_048_576.0,
        (lz4_total as f64 / zstd_total as f64 - 1.0) * 100.0,
    );

    // ── 3. Decompression throughput ──
    println!();
    println!("=== Decompression Throughput ({} threads) ===", threads);

    let t = Instant::now();
    let _: Vec<Vec<u8>> = parallel_map(&zstd_compressed, threads, |compressed| {
        zstd::decode_all(&compressed[..]).unwrap()
    });
    let zstd_decomp_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let _: Vec<Vec<u8>> = parallel_map(&lz4_compressed, threads, |compressed| {
        lz4_flex::decompress_size_prepended(compressed).unwrap()
    });
    let lz4_decomp_ms = t.elapsed().as_millis();

    let decomp_speedup = if lz4_decomp_ms > 0 {
        (zstd_decomp_ms as f64 / lz4_decomp_ms as f64 - 1.0) * 100.0
    } else {
        f64::INFINITY
    };
    println!(
        "  zstd: {:>5}ms ({:.0} MB/s)  LZ4: {:>5}ms ({:.0} MB/s)  {:>+.1}%",
        zstd_decomp_ms,
        if zstd_decomp_ms > 0 {
            mb / (zstd_decomp_ms as f64 / 1000.0)
        } else {
            0.0
        },
        lz4_decomp_ms,
        if lz4_decomp_ms > 0 {
            mb / (lz4_decomp_ms as f64 / 1000.0)
        } else {
            0.0
        },
        decomp_speedup,
    );

    // ── 4. Full pipeline: read → hash → compress ──
    println!();
    println!(
        "=== Full Pipeline: read → hash → compress ({} threads) ===",
        threads
    );

    // Warmup pass: read all files once so both pipelines start with a warm
    // OS page cache and neither benefits from the other's reads.
    let _warmup: Vec<Vec<u8>> = refs.iter().map(|s| read_file(s)).collect();
    drop(_warmup);

    let t = Instant::now();
    let _: Vec<([u8; 32], Vec<u8>)> = parallel_map(&refs, threads, |s| {
        let content = read_file(s);
        let hash = *blake3::hash(&content).as_bytes();
        let compressed = zstd::encode_all(&content[..], 1).unwrap();
        (hash, compressed)
    });
    let blake3_zstd_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let _: Vec<(u128, Vec<u8>)> = parallel_map(&refs, threads, |s| {
        let content = read_file(s);
        let hash = xxhash_rust::xxh3::xxh3_128(&content);
        let compressed = lz4_flex::compress_prepend_size(&content);
        (hash, compressed)
    });
    let xxh3_lz4_ms = t.elapsed().as_millis();

    let pipeline_speedup = if xxh3_lz4_ms > 0 {
        (blake3_zstd_ms as f64 / xxh3_lz4_ms as f64 - 1.0) * 100.0
    } else {
        f64::INFINITY
    };
    println!(
        "  BLAKE3+zstd: {:>5}ms  XXH3+LZ4: {:>5}ms  {:>+.1}%",
        blake3_zstd_ms, xxh3_lz4_ms, pipeline_speedup,
    );

    // ── Summary ──
    println!();
    println!("╔════════════════════════════════════════╗");
    println!("║  Decision Summary                      ║");
    println!("╠════════════════════════════════════════╣");
    let ratio_ok = (lz4_total as f64 / zstd_total as f64 - 1.0) * 100.0 < 30.0;
    println!(
        "║  Pipeline speedup:      {:>+6.1}%       ║",
        pipeline_speedup
    );
    println!(
        "║  Compress ratio delta:  {:>+6.1}%       ║",
        (lz4_total as f64 / zstd_total as f64 - 1.0) * 100.0
    );
    println!(
        "║  Ratio within 30%:      {}            ║",
        if ratio_ok { "YES" } else { "NO " }
    );
    println!("╚════════════════════════════════════════╝");
}
