//! Synthetic control-plane measurement only; never inference, real workspace content or tools.

use carapana_storage::{
    PreparationAdmissionPolicy, PreparationAdmissionStore, PreparationQueuePolicy,
    WorkspaceMetadata,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = fs::canonicalize(std::env::temp_dir())?.join(format!(
            "carapana-admission-probe-{}-{nonce}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        Ok(Self(root))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() {
    if run().is_err() {
        // Never print paths, SQL, raw payloads or environment on failure.
        eprintln!("synthetic admission resource probe failed");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("this probe accepts no workload parameters".into());
    }
    const ITERATIONS: usize = 16;
    const BYTES: u64 = 1024 * 1024;
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.0.join("a"))?;
    fs::create_dir(fixture.0.join("b"))?;
    let a = WorkspaceMetadata::capture(fixture.0.join("a"))?.directory_identity();
    let b = WorkspaceMetadata::capture(fixture.0.join("b"))?.directory_identity();
    let policy = PreparationAdmissionPolicy::with_workspace_limits(64 * BYTES, 4, 32 * BYTES, 2)?;
    let started = Instant::now();
    let mut store = PreparationAdmissionStore::open(fixture.0.join("sessions.sqlite3"), policy)?;
    let open_us = started.elapsed().as_micros();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let lease = store.reserve_unexported_for_workspace(BYTES, 1, &a)?;
        // No consumers exist. This exercises bookkeeping barriers, not execution or RSS payloads.
        lease.mark_exported()?;
        lease.mark_cleanup_confirmed()?;
        lease.release()?;
    }
    let immediate_us = started.elapsed().as_micros();
    store.configure_queue(PreparationQueuePolicy::new(
        32,
        16,
        Duration::from_secs(60),
    )?)?;
    let started = Instant::now();
    let mut first = Vec::new();
    let mut second = Vec::new();
    for _ in 0..ITERATIONS / 2 {
        first.push(store.enqueue_preparation(&a, BYTES, 1)?);
        second.push(store.enqueue_preparation(&b, BYTES, 1)?);
    }
    let enqueue_us = started.elapsed().as_micros();
    let started = Instant::now();
    for (one, two) in first.iter().zip(&second) {
        store
            .claim_queued_preparation(one, BYTES, 2, &a)?
            .release()?;
        store
            .claim_queued_preparation(two, BYTES, 2, &b)?
            .release()?;
    }
    let claim_us = started.elapsed().as_micros();
    let remaining_bytes = store.usage()?.reserved_bytes;
    let remaining_waiting = store.pending_preparations(2)?.len();
    if remaining_bytes != 0 || remaining_waiting != 0 {
        return Err("synthetic probe leaked capacity".into());
    }
    let mut report = serde_json::json!({
        "schema_version": 1,
        "scope": "sqlite-admission-control-plane-only",
        "iterations": ITERATIONS,
        "nominal_bytes_per_intent": BYTES,
        "open_us": open_us,
        "immediate_total_us": immediate_us,
        "enqueue_total_us": enqueue_us,
        "claim_total_us": claim_us,
        "remaining_reserved_bytes": remaining_bytes,
        "remaining_waiting": remaining_waiting,
        "production_defaults_calibrated": false,
    });
    if let Some(bytes) = self_rss_peak()? {
        report["self_rss_peak_bytes"] = serde_json::json!(bytes);
    }
    println!("{report}");
    Ok(())
}

#[cfg(target_os = "linux")]
fn self_rss_peak() -> Result<Option<u64>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut status = String::new();
    fs::File::open("/proc/self/status")?
        .take(64 * 1024)
        .read_to_string(&mut status)?;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmHWM:") {
            let mut fields = value.split_ascii_whitespace();
            let kib: u64 = fields.next().ok_or("missing RSS")?.parse()?;
            if fields.next() != Some("kB") || fields.next().is_some() {
                return Err("invalid RSS unit".into());
            }
            return Ok(Some(kib.checked_mul(1024).ok_or("RSS overflow")?));
        }
    }
    Err("RSS unavailable".into())
}

#[cfg(not(target_os = "linux"))]
fn self_rss_peak() -> Result<Option<u64>, Box<dyn std::error::Error>> {
    Ok(None)
}
