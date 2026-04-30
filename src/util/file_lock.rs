// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! Cross-process advisory file lock used by `meta.toml` writers
//! (and any other read-modify-write target inside a consult dir).
//! The lock is held by the existence of a `<path>.lock` sibling
//! file (created with `O_CREAT|O_EXCL`); when the lock guard drops
//! we remove the lock file.
//!
//! Caveats:
//! - This is best-effort: if a process is killed while holding the
//!   lock, the lock file remains until manually removed (or a peer
//!   waits past the deadline). We surface a clear error in that
//!   case so the user can recover with a manual `rm`.
//! - We use a polling strategy with `tokio::time::sleep` so the
//!   waiter does not block the tokio worker thread.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    /// Try to acquire `<target>.lock`. Polls every 50 ms up to `timeout`
    /// before giving up.
    pub async fn acquire(target: &Path, timeout: Duration) -> Result<Self> {
        let lock_path = lock_path_for(target);
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => {
                    return Ok(FileLock { path: lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::time::Instant::now() > deadline {
                        anyhow::bail!(
                            "could not acquire lock on {} within {:?}; stale lock file? \
                             remove `{}` if no other a2a process is running",
                            target.display(),
                            timeout,
                            lock_path.display()
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("open lock file {}", lock_path.display())
                    });
                }
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_path_for(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let leaf = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    parent.join(format!(".{leaf}.lock"))
}

/// Atomically replace the contents of `path` with `data`.
///
/// Writes to a sibling `<path>.tmp.<pid>.<rand>` first, fsync's the
/// **writable** file descriptor, drops it, then `rename`s into place.
/// If anything fails before the rename, the destination is untouched.
pub fn atomic_write(path: &Path, data: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file"),
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let write_result: Result<()> = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(data.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        // Propagate `sync_all` errors. On a full disk or flaky
        // network FS, silently swallowing them would let `rename`
        // below replace `path` with data that was never durably
        // written.
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
        Ok(())
    })();
    if let Err(e) = write_result {
        // Best-effort cleanup so a write failure (full disk, bad
        // permissions, etc.) does not leave a `.tmp` shrapnel
        // accumulating in the directory across retries.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // If rename fails (e.g. destination is open by another program
    // on Windows, or permissions issue), remove the tmp file so
    // retries don't accumulate `.tmp.*` shrapnel.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("rename {} -> {}", tmp.display(), path.display()));
    }
    Ok(())
}
