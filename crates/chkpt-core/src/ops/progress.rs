/// Events emitted during save/restore operations for progress reporting.
pub enum ProgressEvent {
    // Save events
    ScanComplete { file_count: u64 },
    ProcessStart { total: u64 },
    ProcessFile { completed: u64, total: u64 },
    PackComplete,

    // Restore events
    ScanCurrentComplete { file_count: u64 },
    RestoreStart { add: u64, change: u64, remove: u64 },
    RestoreFile { completed: u64, total: u64 },
}

/// Optional progress callback. Pass `None` to disable progress reporting.
pub type ProgressCallback = Option<Box<dyn Fn(ProgressEvent) + Send + Sync>>;

#[inline]
pub fn emit(progress: &ProgressCallback, event: ProgressEvent) {
    if let Some(cb) = progress {
        cb(event);
    }
}
