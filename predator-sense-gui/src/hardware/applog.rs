//! Minimal persistent logging for the app side (issue #7). Complements the
//! Rust hotkey service's own logging - together they cover the pieces that
//! were invisible during issue #4's remote debugging (profile switches, RGB
//! HID writes, permission errors), without needing dmesg or live reproduction.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const BACKUPS: u32 = 3;

/// Off by default - not everyone needs this, and it's meant for remote
/// debugging sessions, not always-on disk writes.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Where the log lives.
///
/// `PREDATOR_SENSE_LOG_DIR` redirects it. That exists for this module's own
/// test, which deletes the log, forces a rotation by appending five megabytes
/// and then asserts on the result - all of which it used to do to the real
/// file in the user's home, so running the test suite wiped whatever had been
/// captured and left a five megabyte file of padding behind.
fn log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PREDATOR_SENSE_LOG_DIR") {
        return PathBuf::from(dir);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".local/share"));
    base.join("predator-sense")
}

fn log_path() -> PathBuf {
    log_dir().join("app.log")
}

fn timestamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
            tm.tm_hour, tm.tm_min, tm.tm_sec
        )
    }
}

fn rotate_if_needed() {
    let path = log_path();
    let Ok(meta) = fs::metadata(&path) else { return };
    if meta.len() <= MAX_BYTES {
        return;
    }
    for i in (1..BACKUPS).rev() {
        let from = log_dir().join(format!("app.log.{}", i));
        let to = log_dir().join(format!("app.log.{}", i + 1));
        let _ = fs::rename(&from, &to);
    }
    let _ = fs::rename(&path, log_dir().join("app.log.1"));
}

fn write_line(level: &str, msg: &str) {
    if !is_enabled() {
        return;
    }
    let _ = fs::create_dir_all(log_dir());
    rotate_if_needed();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(f, "[{}] {} {}", timestamp(), level, msg);
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates() {
        // Never the real log directory: this test deletes the log and pads it
        // past the rotation threshold, which in a user's home destroys exactly
        // the history the log exists to keep.
        let scratch = std::env::temp_dir().join(format!(
            "predator-sense-applog-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&scratch).unwrap();
        std::env::set_var("PREDATOR_SENSE_LOG_DIR", &scratch);
        assert_eq!(log_dir(), scratch, "the test must not touch the real log");

        set_enabled(true);
        let _ = fs::remove_file(log_path());
        for i in 1..=3 { let _ = fs::remove_file(log_dir().join(format!("app.log.{}", i))); }

        info("test line 1");
        error("test error line");
        let content = fs::read_to_string(log_path()).unwrap();
        assert!(content.contains("INFO test line 1"));
        assert!(content.contains("ERROR test error line"));
        assert!(content.lines().next().unwrap().starts_with('['));

        // Force rotation
        {
            let mut f = OpenOptions::new().append(true).open(log_path()).unwrap();
            let filler = vec![b'x'; (MAX_BYTES + 1) as usize];
            f.write_all(&filler).unwrap();
        }
        info("after rotation");
        assert!(log_dir().join("app.log.1").exists(), "expected app.log.1 after rotation");
        let new_content = fs::read_to_string(log_path()).unwrap();
        assert!(new_content.contains("after rotation"));
        assert!(!new_content.contains("test line 1"), "old content should have rotated out");

        // Disabled by default (and when explicitly turned off): no write at all.
        set_enabled(false);
        let _ = fs::remove_file(log_path());
        info("should not appear");
        assert!(!log_path().exists(), "disabled logging must not create the file");

        std::env::remove_var("PREDATOR_SENSE_LOG_DIR");
        let _ = fs::remove_dir_all(&scratch);
    }
}
