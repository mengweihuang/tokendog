use std::fs;
use std::fs::File;
use std::io::Write;

use router::config::logging::RotatingFileWriter;

#[test]
fn test_write_and_rotate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.log");
    let mut writer = RotatingFileWriter::new(&path, 100).unwrap();

    // Write under the threshold — no rotation.
    writer.write_all(b"hello").unwrap();
    writer.flush().unwrap();
    assert!(path.exists());
    let size_after = path.metadata().unwrap().len();
    assert!(size_after > 0 && size_after < 100);

    // Write enough to trigger rotation (first write was 5 bytes, max is 100).
    let large = vec![b'x'; 96];
    writer.write_all(&large).unwrap();
    writer.flush().unwrap();

    // Original file should be truncated (only new content after rotation).
    let final_size = path.metadata().unwrap().len();
    assert!(
        final_size < 100,
        "File should be truncated, got {final_size}"
    );

    // A backup file should exist.
    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".gz"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(backups.len(), 1, "Expected one gzip backup");
}

#[test]
fn test_no_rotation_on_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.log");
    // Create an empty file.
    File::create(&path).unwrap();

    let mut writer = RotatingFileWriter::new(&path, 10).unwrap();
    let large = vec![b'a'; 20];
    // This write exceeds max_size, but file was empty (bytes_written == 0),
    // so it should NOT rotate — it just writes.
    writer.write_all(&large).unwrap();
    writer.flush().unwrap();

    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".gz"))
                .unwrap_or(false)
        })
        .collect();
    assert!(backups.is_empty(), "Should not rotate on empty file");
}
