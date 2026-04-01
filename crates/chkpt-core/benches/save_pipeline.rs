//! Benchmark: measure each phase of the save pipeline + optimization variants.
//!
//! Usage:
//!   cargo bench -p chkpt-core --bench save_pipeline -- /path/to/target/dir

use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::index::{FileEntry, FileIndex};
use chkpt_core::scanner::{scan_workspace_with_options, ScannedFile};
use chkpt_core::store::pack::PackWriter;
use chkpt_core::store::tree::{EntryType, TreeEntry, TreeStore};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::mem::MaybeUninit;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ─── read strategies ────────────────────────────────────────────────

fn read_std(scanned: &ScannedFile) -> Vec<u8> {
    let mut file = std::fs::File::open(&scanned.absolute_path).unwrap();
    let mut buf = Vec::with_capacity(scanned.size as usize);
    file.read_to_end(&mut buf).unwrap();
    buf
}

#[cfg(unix)]
fn read_openat(scanned: &ScannedFile, dir_fds: &HashMap<String, std::os::fd::OwnedFd>) -> Vec<u8> {
    use rustix::fd::AsFd;
    use rustix::fs::{openat, Mode, OFlags};
    use std::os::fd::FromRawFd;

    let parent = scanned
        .absolute_path
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let filename = scanned.absolute_path.file_name().unwrap().to_string_lossy();

    let dir_fd = dir_fds.get(&parent).unwrap();
    let file_fd = openat(dir_fd.as_fd(), &*filename, OFlags::RDONLY, Mode::empty()).unwrap();
    let mut file =
        unsafe { std::fs::File::from_raw_fd(rustix::fd::IntoRawFd::into_raw_fd(file_fd)) };
    let mut buf = Vec::with_capacity(scanned.size as usize);
    file.read_to_end(&mut buf).unwrap();
    buf
}

fn read_mmap_hybrid(scanned: &ScannedFile) -> Vec<u8> {
    if scanned.size >= 65536 {
        let file = std::fs::File::open(&scanned.absolute_path).unwrap();
        let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
        mmap.to_vec()
    } else {
        read_std(scanned)
    }
}

// ─── parallel helpers ───────────────────────────────────────────────

fn parallel_map<T, R, F>(items: &[T], worker_count: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    if worker_count <= 1 || items.is_empty() {
        return items.iter().map(&f).collect();
    }
    let chunk_size = items.len().div_ceil(worker_count);
    let results: Vec<Vec<R>> = std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(|| chunk.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    results.into_iter().flatten().collect()
}

fn bench_read_hash(
    label: &str,
    files: &[&ScannedFile],
    threads: usize,
    read_fn: impl Fn(&ScannedFile) -> Vec<u8> + Sync,
) -> u128 {
    let t = Instant::now();
    let _results: Vec<[u8; 32]> = parallel_map(files, threads, |s| {
        let content = read_fn(s);
        *blake3::hash(&content).as_bytes()
    });
    let ms = t.elapsed().as_millis();
    println!("  {:<30} {:>6}ms  ({} threads)", label, ms, threads);
    ms
}

// ─── openat dir FD cache builder ────────────────────────────────────

#[cfg(unix)]
fn build_dir_fd_cache(files: &[&ScannedFile]) -> HashMap<String, std::os::fd::OwnedFd> {
    use rustix::fs::{openat, Mode, OFlags, CWD};

    let mut dirs: HashSet<String> = HashSet::new();
    for f in files {
        let parent = f
            .absolute_path
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        dirs.insert(parent);
    }

    let mut cache = HashMap::with_capacity(dirs.len());
    for dir in dirs {
        let fd = openat(
            CWD,
            &*dir,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .unwrap();
        cache.insert(dir, fd);
    }
    cache
}

// ─── main ───────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target_dir = args.get(1).expect("Usage: save_pipeline <target_dir>");
    let target_path = std::path::Path::new(target_dir);
    assert!(target_path.is_dir(), "Target must be a directory");

    let default_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    println!("=== chkpt save pipeline benchmark ===");
    println!("Target: {}", target_dir);
    println!("CPUs: {}", default_threads);
    println!();

    // ── Phase 1: Scan ──
    let t = Instant::now();
    let scanned_files = scan_workspace_with_options(target_path, None, true).unwrap();
    let scan_ms = t.elapsed().as_millis();

    let under_1k = scanned_files.iter().filter(|f| f.size < 1024).count();
    let over_64k = scanned_files.iter().filter(|f| f.size >= 65536).count();
    let total_bytes: u64 = scanned_files.iter().map(|f| f.size).sum();
    println!(
        "[scan]          {:>6}ms  ({} files, <1KB: {}({:.0}%), ≥64KB: {}({:.0}%), {:.0}MB)",
        scan_ms,
        scanned_files.len(),
        under_1k,
        under_1k as f64 / scanned_files.len() as f64 * 100.0,
        over_64k,
        over_64k as f64 / scanned_files.len() as f64 * 100.0,
        total_bytes as f64 / 1_048_576.0,
    );

    // ── Phase 2: Index check ──
    let project_id = project_id_from_path(target_path);
    let layout = StoreLayout::new(&project_id);
    let t = Instant::now();
    let index = FileIndex::open(layout.index_path()).unwrap();
    let cached_entries = index.entries_by_path().unwrap();
    let index_load_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let mut files_to_prepare: Vec<&ScannedFile> = Vec::new();
    let mut cached_count = 0u64;
    for scanned in &scanned_files {
        if let Some(cached) = cached_entries.get(&scanned.relative_path) {
            if cached.mtime_secs == scanned.mtime_secs
                && cached.mtime_nanos == scanned.mtime_nanos
                && cached.size == scanned.size
                && cached.inode == scanned.inode
            {
                cached_count += 1;
                continue;
            }
        }
        files_to_prepare.push(scanned);
    }
    let index_check_ms = t.elapsed().as_millis();
    println!(
        "[index]         {:>6}ms  (load: {}ms, check: {}ms, cached: {}, new: {})",
        index_load_ms + index_check_ms,
        index_load_ms,
        index_check_ms,
        cached_count,
        files_to_prepare.len()
    );

    if files_to_prepare.is_empty() {
        println!("\nAll files cached. Delete index to benchmark cold save:");
        println!("  rm {}", layout.index_path().display());
        return;
    }

    // ── Phase 3: read+hash optimization comparison ──
    println!();
    println!(
        "--- read+hash variants ({} files) ---",
        files_to_prepare.len()
    );

    // 3a: Baseline (current code)
    let baseline_ms = bench_read_hash(
        "baseline (std::fs, path order)",
        &files_to_prepare,
        default_threads,
        read_std,
    );

    // 3b: Inode-sorted
    let mut inode_sorted = files_to_prepare.clone();
    inode_sorted.sort_unstable_by_key(|f| f.inode.unwrap_or(u64::MAX));
    let inode_ms = bench_read_hash("inode-sorted", &inode_sorted, default_threads, read_std);

    // 3c: Thread count sweep (with inode sort)
    println!();
    println!("--- thread count sweep (inode-sorted) ---");
    let thread_counts: Vec<usize> = vec![2, 4, 6, 8, 10, 12, 16];
    let mut best_threads = default_threads;
    let mut best_thread_ms = u128::MAX;
    for &tc in &thread_counts {
        if tc > default_threads * 2 {
            continue;
        }
        let ms = bench_read_hash(&format!("{} threads", tc), &inode_sorted, tc, read_std);
        if ms < best_thread_ms {
            best_thread_ms = ms;
            best_threads = tc;
        }
    }
    println!("  >> best: {} threads ({}ms)", best_threads, best_thread_ms);

    // 3d: openat + dir FD cache (with inode sort + best thread count)
    #[cfg(unix)]
    let openat_ms = {
        println!();
        println!("--- openat + dir FD cache ---");
        let t = Instant::now();
        let dir_fds = build_dir_fd_cache(&inode_sorted);
        let cache_build_ms = t.elapsed().as_millis();
        println!(
            "  dir FD cache build: {}ms ({} dirs)",
            cache_build_ms,
            dir_fds.len()
        );

        let dir_fds = Arc::new(dir_fds);
        let dir_fds_ref = &dir_fds;
        let ms = bench_read_hash("openat + inode-sorted", &inode_sorted, best_threads, |s| {
            read_openat(s, dir_fds_ref)
        });
        ms + cache_build_ms
    };

    // 3e: mmap hybrid (with inode sort + best thread count)
    println!();
    println!("--- mmap hybrid (≥64KB → mmap, <64KB → std) ---");
    let mmap_ms = bench_read_hash(
        "mmap hybrid + inode-sorted",
        &inode_sorted,
        best_threads,
        read_mmap_hybrid,
    );

    // 3f: All combined: openat + mmap + inode + best threads
    #[cfg(unix)]
    let combined_ms = {
        println!();
        println!(
            "--- combined: openat + mmap + inode + {} threads ---",
            best_threads
        );
        let dir_fds = Arc::new(build_dir_fd_cache(&inode_sorted));
        let dir_fds_ref = &dir_fds;
        bench_read_hash("ALL COMBINED", &inode_sorted, best_threads, |s| {
            if s.size >= 65536 {
                // mmap for large files
                let file = std::fs::File::open(&s.absolute_path).unwrap();
                let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
                mmap.to_vec()
            } else {
                // openat for small files
                read_openat(s, dir_fds_ref)
            }
        })
    };

    // ── Phase 4: compress + pack (using best config) ──
    println!();
    println!("--- compress + pack (best config) ---");
    let seen: HashSet<[u8; 32]> = HashSet::new();
    let seen = Arc::new(Mutex::new(seen));

    let t = Instant::now();
    let compress_results: Vec<Option<(String, Vec<u8>)>> =
        parallel_map(&inode_sorted, best_threads, |s| {
            let content = read_std(s);
            let hash = blake3::hash(&content);
            let hash_bytes: [u8; 32] = *hash.as_bytes();
            let is_new = {
                let mut set = seen.lock().unwrap();
                set.insert(hash_bytes)
            };
            if is_new {
                let compressed = zstd::encode_all(&content[..], 1).unwrap();
                Some((hash.to_hex().to_string(), compressed))
            } else {
                None
            }
        });
    let compress_ms = t.elapsed().as_millis();
    let unique_count = compress_results.iter().filter(|r| r.is_some()).count();
    let dup_count = compress_results.iter().filter(|r| r.is_none()).count();
    println!(
        "  [read+hash+compress]          {:>6}ms  (unique: {}, dup: {})",
        compress_ms, unique_count, dup_count
    );

    let tmp_dir = tempfile::tempdir().unwrap();
    let packs_dir = tmp_dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let t = Instant::now();
    let mut pack_writer = PackWriter::new(&packs_dir).unwrap();
    for result in &compress_results {
        if let Some((hash_hex, compressed)) = result {
            pack_writer
                .add_pre_compressed(hash_hex.clone(), compressed.clone())
                .unwrap();
        }
    }
    if !pack_writer.is_empty() {
        pack_writer.finish().unwrap();
    }
    let pack_ms = t.elapsed().as_millis();
    println!("  [pack_write]                  {:>6}ms", pack_ms);

    // ── Phase 5: Build tree ──
    struct PF {
        relative_path: String,
        blob_hash_bytes: [u8; 32],
        size: u64,
        mode: u32,
    }
    // Use first hash run results for tree (need to re-hash for consistency)
    let hash_results: Vec<[u8; 32]> = parallel_map(&inode_sorted, best_threads, |s| {
        let content = read_std(s);
        *blake3::hash(&content).as_bytes()
    });
    let processed: Vec<PF> = inode_sorted
        .iter()
        .zip(hash_results.iter())
        .map(|(s, hash)| PF {
            relative_path: s.relative_path.clone(),
            blob_hash_bytes: *hash,
            size: s.size,
            mode: s.mode,
        })
        .collect();

    let trees_dir = tmp_dir.path().join("trees");
    std::fs::create_dir_all(&trees_dir).unwrap();
    let tree_store = TreeStore::new(trees_dir);

    let t = Instant::now();
    let mut dir_files: BTreeMap<String, Vec<&PF>> = BTreeMap::new();
    let mut all_dirs: BTreeSet<String> = BTreeSet::new();
    let mut child_dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    all_dirs.insert(String::new());

    for pf in &processed {
        let parent = if let Some(pos) = pf.relative_path.rfind('/') {
            pf.relative_path[..pos].to_string()
        } else {
            String::new()
        };
        dir_files.entry(parent.clone()).or_default().push(pf);
        register_dir_hierarchy(&parent, &mut all_dirs, &mut child_dirs);
    }

    let mut dir_list: Vec<String> = all_dirs.into_iter().collect();
    dir_list.sort_unstable_by(|a, b| {
        let da = if a.is_empty() {
            0
        } else {
            a.matches('/').count() + 1
        };
        let db = if b.is_empty() {
            0
        } else {
            b.matches('/').count() + 1
        };
        db.cmp(&da).then_with(|| a.cmp(b))
    });

    let mut dir_hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut pack_entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(dir_list.len());
    let mut known_hashes: HashSet<String> = HashSet::with_capacity(dir_list.len());

    for dir in &dir_list {
        let mut entries: Vec<TreeEntry> = Vec::new();
        if let Some(files) = dir_files.get(dir) {
            for pf in files {
                let name = if let Some(pos) = pf.relative_path.rfind('/') {
                    pf.relative_path[pos + 1..].to_string()
                } else {
                    pf.relative_path.clone()
                };
                entries.push(TreeEntry {
                    name,
                    entry_type: EntryType::File,
                    hash: pf.blob_hash_bytes,
                    size: pf.size,
                    mode: pf.mode,
                });
            }
        }
        if let Some(children) = child_dirs.get(dir) {
            for sub_dir in children {
                let sub_hash = dir_hashes.get(sub_dir).unwrap();
                let sub_name = if let Some(pos) = sub_dir.rfind('/') {
                    sub_dir[pos + 1..].to_string()
                } else {
                    sub_dir.clone()
                };
                let mut hash_bytes = [0u8; 32];
                for i in 0..32 {
                    hash_bytes[i] = u8::from_str_radix(&sub_hash[i * 2..i * 2 + 2], 16).unwrap();
                }
                entries.push(TreeEntry {
                    name: sub_name,
                    entry_type: EntryType::Dir,
                    hash: hash_bytes,
                    size: 0,
                    mode: 0o040755,
                });
            }
        }
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let encoded = bitcode::encode(&entries);
        let hash_hex = blake3::hash(&encoded).to_hex().to_string();
        dir_hashes.insert(dir.clone(), hash_hex.clone());
        if known_hashes.insert(hash_hex.clone()) {
            pack_entries.push((hash_hex, encoded));
        }
    }
    tree_store.write_pack(&pack_entries).unwrap();
    let tree_ms = t.elapsed().as_millis();
    println!(
        "  [build_tree]                  {:>6}ms  ({} dirs)",
        tree_ms,
        dir_list.len()
    );

    // ── Phase 6: Index flush ──
    let t = Instant::now();
    let mut entries_vec: Vec<FileEntry> = Vec::with_capacity(files_to_prepare.len());
    for (s, hash) in inode_sorted.iter().zip(hash_results.iter()) {
        entries_vec.push(FileEntry {
            path: s.relative_path.clone(),
            blob_hash: *hash,
            size: s.size,
            mtime_secs: s.mtime_secs,
            mtime_nanos: s.mtime_nanos,
            inode: s.inode,
            mode: s.mode,
        });
    }
    let encoded = bitcode::encode(&entries_vec);
    let idx_path = tmp_dir.path().join("index.bin");
    std::fs::write(&idx_path, &encoded).unwrap();
    let index_flush_ms = t.elapsed().as_millis();
    println!(
        "  [index_flush]                 {:>6}ms  ({:.1}MB)",
        index_flush_ms,
        encoded.len() as f64 / 1_048_576.0,
    );

    // ── Summary ──
    println!();
    println!("╔══════════════════════════════════════════════════╗");
    println!("║  read+hash comparison summary                   ║");
    println!("╠══════════════════════════════════════════════════╣");
    println!(
        "║  baseline (path order, {} threads) {:>6}ms     ║",
        default_threads, baseline_ms
    );
    println!(
        "║  + inode sort                      {:>6}ms {:>+4.0}% ║",
        inode_ms,
        pct(inode_ms, baseline_ms)
    );
    println!(
        "║  + best threads ({:>2})              {:>6}ms {:>+4.0}% ║",
        best_threads,
        best_thread_ms,
        pct(best_thread_ms, baseline_ms)
    );
    #[cfg(unix)]
    println!(
        "║  + openat                         {:>6}ms {:>+4.0}% ║",
        openat_ms,
        pct(openat_ms, baseline_ms)
    );
    println!(
        "║  + mmap hybrid                    {:>6}ms {:>+4.0}% ║",
        mmap_ms,
        pct(mmap_ms, baseline_ms)
    );
    #[cfg(unix)]
    println!(
        "║  ALL COMBINED                     {:>6}ms {:>+4.0}% ║",
        combined_ms,
        pct(combined_ms, baseline_ms)
    );
    println!("╚══════════════════════════════════════════════════╝");
    if let Some(peak_rss_kb) = peak_rss_kb() {
        println!("peak_rss_kb={peak_rss_kb}");
    }
}

fn pct(new: u128, old: u128) -> f64 {
    (new as f64 - old as f64) / old as f64 * 100.0
}

fn register_dir_hierarchy(
    dir: &str,
    all_dirs: &mut BTreeSet<String>,
    child_dirs: &mut BTreeMap<String, Vec<String>>,
) {
    if dir.is_empty() {
        return;
    }
    let mut parent = String::new();
    for segment in dir.split('/') {
        let current = if parent.is_empty() {
            segment.to_string()
        } else {
            format!("{}/{}", parent, segment)
        };
        if all_dirs.insert(current.clone()) {
            child_dirs
                .entry(parent.clone())
                .or_default()
                .push(current.clone());
        }
        parent = current;
    }
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
