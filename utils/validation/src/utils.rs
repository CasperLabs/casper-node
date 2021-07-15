use std::{
    fs::{self, DirEntry},
    io,
    path::Path,
};

/// Like [`fs::read_dir]` but recursive.
pub fn recursive_read_dir(dir: &Path) -> io::Result<Vec<DirEntry>> {
    let mut result = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            result.append(&mut recursive_read_dir(&path)?);
        } else {
            result.push(entry);
        }
    }

    Ok(result)
}
