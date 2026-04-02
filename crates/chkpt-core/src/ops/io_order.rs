use crate::scanner::ScannedFile;

#[inline]
fn inode_sort_key(inode: Option<u64>) -> u64 {
    inode.unwrap_or(u64::MAX)
}

pub(crate) fn sort_scanned_refs_for_locality(scanned_files: &mut [&ScannedFile]) {
    scanned_files.sort_unstable_by(|left, right| {
        inode_sort_key(left.inode)
            .cmp(&inode_sort_key(right.inode))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
}

pub(crate) fn sort_scanned_for_locality(scanned_files: &mut [ScannedFile]) {
    scanned_files.sort_unstable_by(|left, right| {
        inode_sort_key(left.inode)
            .cmp(&inode_sort_key(right.inode))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_sort_scanned_refs_for_locality_orders_by_inode_then_path() {
        let f1 = scanned("b.txt", Some(2));
        let f2 = scanned("a.txt", Some(2));
        let f3 = scanned("z.txt", None);
        let f4 = scanned("c.txt", Some(1));

        let mut refs = vec![&f1, &f2, &f3, &f4];
        sort_scanned_refs_for_locality(&mut refs);

        let paths: Vec<&str> = refs
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["c.txt", "a.txt", "b.txt", "z.txt"]);
    }

    #[test]
    fn test_sort_scanned_for_locality_orders_by_inode_then_path() {
        let mut files = vec![
            scanned("b.txt", Some(2)),
            scanned("a.txt", Some(2)),
            scanned("z.txt", None),
            scanned("c.txt", Some(1)),
        ];

        sort_scanned_for_locality(&mut files);

        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["c.txt", "a.txt", "b.txt", "z.txt"]);
    }

    fn scanned(relative_path: &str, inode: Option<u64>) -> ScannedFile {
        ScannedFile {
            relative_path: relative_path.to_string(),
            absolute_path: PathBuf::from(relative_path),
            size: 0,
            mtime_secs: 0,
            mtime_nanos: 0,
            device: None,
            inode,
            mode: 0o100644,
            is_symlink: false,
        }
    }
}
