// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Atomic file writer using temporary file and atomic rename.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub fn write_atomic<P: AsRef<Path>>(path: P, content: &[u8]) -> io::Result<()> {
    let dest_path = path.as_ref();
    let parent_dir = dest_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent_dir)?;

    let tmp_path = dest_path.with_extension(format!("tmp.{}", std::process::id()));

    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    // Atomic rename
    if let Err(_e) = fs::rename(&tmp_path, dest_path) {
        #[cfg(windows)]
        {
            let _ = fs::remove_file(dest_path);
            if let Err(e2) = fs::rename(&tmp_path, dest_path) {
                let _ = fs::remove_file(&tmp_path);
                return Err(e2);
            }
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(_e);
        }
    }

    Ok(())
}
