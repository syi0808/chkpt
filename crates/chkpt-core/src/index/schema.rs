pub const CREATE_TABLES: &str = "
CREATE TABLE IF NOT EXISTS file_index (
    path        TEXT PRIMARY KEY,
    blob_hash   BLOB NOT NULL,
    size        INTEGER NOT NULL,
    mtime_secs  INTEGER NOT NULL,
    mtime_nanos INTEGER NOT NULL,
    inode       INTEGER,
    mode        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
