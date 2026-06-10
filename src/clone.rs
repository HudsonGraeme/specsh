use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clonefile(
        src: *const std::os::raw::c_char,
        dst: *const std::os::raw::c_char,
        flags: std::os::raw::c_uint,
    ) -> std::os::raw::c_int;
}

pub struct WorkClone {
    pub path: PathBuf,
    keep: bool,
}

impl WorkClone {
    pub fn create(src: &Path, keep: bool) -> io::Result<WorkClone> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dst = std::env::temp_dir().join(format!("specsh-{}-{}", std::process::id(), nanos));
        cow_copy(src, &dst)?;
        Ok(WorkClone { path: dst, keep })
    }
}

#[cfg(target_os = "macos")]
fn cow_copy(src: &Path, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let s = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in path"))?;
    let d = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in path"))?;
    let rc = unsafe { clonefile(s.as_ptr(), d.as_ptr(), 0) };
    if rc == 0 {
        return Ok(());
    }
    let status = Command::new("/bin/cp")
        .arg("-R")
        .arg(src)
        .arg(dst)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "clonefile failed and cp -R fallback failed",
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn cow_copy(src: &Path, dst: &Path) -> io::Result<()> {
    let status = Command::new("cp")
        .args(["-a", "--reflink=auto"])
        .arg(src)
        .arg(dst)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("cp --reflink=auto failed"))
    }
}

impl Drop for WorkClone {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_copies_and_drop_removes() {
        let src = std::env::temp_dir().join(format!("specsh-test-src-{}", std::process::id()));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/file.txt"), "payload").unwrap();
        let clone_path;
        {
            let c = WorkClone::create(&src, false).unwrap();
            clone_path = c.path.clone();
            assert_eq!(
                std::fs::read_to_string(c.path.join("sub/file.txt")).unwrap(),
                "payload"
            );
            std::fs::write(c.path.join("sub/file.txt"), "mutated").unwrap();
            assert_eq!(
                std::fs::read_to_string(src.join("sub/file.txt")).unwrap(),
                "payload"
            );
        }
        assert!(!clone_path.exists());
        std::fs::remove_dir_all(&src).unwrap();
    }
}
