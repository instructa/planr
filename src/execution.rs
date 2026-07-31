use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    process::Child,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub(crate) struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub(crate) fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
pub(crate) struct BoundedProcessInput<'a> {
    pub(crate) cwd: &'a Path,
    pub(crate) argv: &'a [String],
    pub(crate) env: Vec<(&'a str, String)>,
    pub(crate) timeout: Duration,
    pub(crate) output_limit_bytes: usize,
    pub(crate) stdout_limit_bytes: Option<usize>,
    pub(crate) stderr_limit_bytes: Option<usize>,
    pub(crate) cancellation: &'a CancellationToken,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundedProcessOutput {
    pub(crate) argv: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) interrupted: bool,
    #[allow(dead_code)]
    pub(crate) output_limit_exceeded: bool,
    pub(crate) stdout_digest: String,
    pub(crate) stderr_digest: String,
    pub(crate) stdout_excerpt: String,
    pub(crate) stderr_excerpt: String,
    #[allow(dead_code)]
    pub(crate) stdout_bytes: usize,
    #[allow(dead_code)]
    pub(crate) stderr_bytes: usize,
    #[allow(dead_code)]
    pub(crate) stdout_truncated: bool,
    #[allow(dead_code)]
    pub(crate) stderr_truncated: bool,
    #[cfg(test)]
    pub(crate) process_tree_term_grace_sleeps: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct BoundedProcessError {
    kind: &'static str,
    output: Option<BoundedProcessOutput>,
}

#[allow(dead_code)]
impl BoundedProcessError {
    fn output_limit_exceeded(output: BoundedProcessOutput) -> Self {
        Self {
            kind: "output_limit_exceeded",
            output: Some(output),
        }
    }

    pub(crate) fn is_output_limit_exceeded(&self) -> bool {
        self.kind == "output_limit_exceeded"
    }

    pub(crate) fn output(&self) -> Option<&BoundedProcessOutput> {
        self.output.as_ref()
    }
}

impl std::fmt::Display for BoundedProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.kind)
    }
}

impl std::error::Error for BoundedProcessError {}

pub(crate) fn run_bounded_process(input: BoundedProcessInput<'_>) -> Result<BoundedProcessOutput> {
    let mut command = Command::new(&input.argv[0]);
    command
        .args(&input.argv[1..])
        .env_clear()
        .current_dir(input.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in input.env {
        command.env(name, value);
    }
    configure_process_group(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("spawning {}", input.argv[0]))?;
    let stdout = child.stdout.take().context("capturing child stdout")?;
    let stderr = child.stderr.take().context("capturing child stderr")?;
    let output_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_handle = drain_limited(
        stdout,
        input.stdout_limit_bytes.unwrap_or(input.output_limit_bytes),
        Arc::clone(&output_exceeded),
    );
    let stderr_handle = drain_limited(
        stderr,
        input.stderr_limit_bytes.unwrap_or(input.output_limit_bytes),
        Arc::clone(&output_exceeded),
    );
    let term_grace_sleeps = ProcessTreeTermGraceSleeps::new();
    let deadline = Instant::now() + input.timeout;
    let mut timed_out = false;
    let mut interrupted = false;
    let mut child_status: Option<ExitStatus> = None;
    loop {
        if let Some(status) = child.try_wait()? {
            child_status = Some(status);
            terminate_process_tree(&mut child, &term_grace_sleeps);
            break;
        }
        if input.cancellation.is_cancelled() {
            interrupted = true;
            terminate_process_tree(&mut child, &term_grace_sleeps);
            break;
        }
        if output_exceeded.load(Ordering::SeqCst) {
            terminate_process_tree(&mut child, &term_grace_sleeps);
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate_process_tree(&mut child, &term_grace_sleeps);
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let status = match child_status {
        Some(status) => status,
        None => child.wait()?,
    };
    let stdout = join_drain(stdout_handle)?;
    let stderr = join_drain(stderr_handle)?;
    let output_limit_exceeded = output_exceeded.load(Ordering::SeqCst)
        || stdout.truncated
        || stderr.truncated
        || stdout.bytes.len() > input.stdout_limit_bytes.unwrap_or(input.output_limit_bytes)
        || stderr.bytes.len() > input.stderr_limit_bytes.unwrap_or(input.output_limit_bytes);
    let output = BoundedProcessOutput {
        argv: input.argv.to_vec(),
        exit_code: status.code(),
        timed_out,
        interrupted,
        output_limit_exceeded,
        stdout_digest: sha256_prefixed(&stdout.bytes),
        stderr_digest: sha256_prefixed(&stderr.bytes),
        stdout_excerpt: bounded_utf8_excerpt(&stdout.bytes, input.output_limit_bytes),
        stderr_excerpt: bounded_utf8_excerpt(&stderr.bytes, input.output_limit_bytes),
        stdout_bytes: stdout.bytes.len(),
        stderr_bytes: stderr.bytes.len(),
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        #[cfg(test)]
        process_tree_term_grace_sleeps: term_grace_sleeps.load(),
    };
    if output_limit_exceeded {
        return Err(BoundedProcessError::output_limit_exceeded(output).into());
    }
    Ok(output)
}

pub(crate) fn sha256_prefixed(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn bounded_utf8_excerpt(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).to_string()
}

#[derive(Debug)]
struct DrainedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain_limited<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    output_exceeded: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<DrainedOutput>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if bytes.len() + read > limit {
                let allowed = limit.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..allowed]);
                truncated = true;
                output_exceeded.store(true, Ordering::SeqCst);
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(DrainedOutput { bytes, truncated })
    })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[derive(Debug)]
struct ProcessTreeTermGraceSleeps {
    #[cfg(test)]
    count: std::sync::atomic::AtomicUsize,
}

impl ProcessTreeTermGraceSleeps {
    fn new() -> Self {
        Self {
            #[cfg(test)]
            count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn record(&self) {
        #[cfg(test)]
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn load(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

#[cfg(unix)]
fn terminate_process_tree(child: &mut Child, term_grace_sleeps: &ProcessTreeTermGraceSleeps) {
    let process_group_id = child.id() as i32;
    if !signal_process_group(process_group_id, libc::SIGTERM) {
        return;
    }
    sleep_process_tree_term_grace(term_grace_sleeps);
    signal_process_group(process_group_id, libc::SIGKILL);
}

#[cfg(not(unix))]
fn terminate_process_tree(child: &mut Child, _term_grace_sleeps: &ProcessTreeTermGraceSleeps) {
    let _ = child.kill();
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn signal_process_group(process_group_id: i32, signal: i32) -> bool {
    let target = -process_group_id;
    // SAFETY: kill(2) is called with a process-group target derived from the
    // spawned child id and a constant signal. ESRCH means the child and any
    // descendants are already gone, so there is no grace window to wait out.
    unsafe { libc::kill(target, signal) == 0 }
}

#[cfg(unix)]
fn sleep_process_tree_term_grace(term_grace_sleeps: &ProcessTreeTermGraceSleeps) {
    term_grace_sleeps.record();
    thread::sleep(Duration::from_millis(50));
}

fn join_drain(handle: thread::JoinHandle<Result<DrainedOutput>>) -> Result<DrainedOutput> {
    handle
        .join()
        .map_err(|_| anyhow::anyhow!("output drain thread panicked"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[cfg(unix)]
    use std::{
        fs,
        path::Path,
        process::{Command as StdCommand, Stdio as StdStdio},
        time::Instant,
    };

    fn run(
        argv: &[&str],
        timeout: Duration,
        output_limit_bytes: usize,
    ) -> Result<BoundedProcessOutput> {
        let argv = argv
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let cwd = tempfile::tempdir()?;
        run_bounded_process(BoundedProcessInput {
            cwd: cwd.path(),
            argv: &argv,
            env: Vec::new(),
            timeout,
            output_limit_bytes,
            stdout_limit_bytes: None,
            stderr_limit_bytes: None,
            cancellation: &CancellationToken::new(),
        })
    }

    #[test]
    fn bounded_process_records_exit_output_digest_and_excerpt() {
        let output = run(&["printf", "hello"], Duration::from_secs(1), 16).unwrap();

        assert_eq!(output.argv, vec!["printf", "hello"]);
        assert_eq!(output.exit_code, Some(0));
        assert!(!output.timed_out);
        assert!(!output.interrupted);
        assert!(!output.output_limit_exceeded);
        assert_eq!(output.stdout_excerpt, "hello");
        assert_eq!(output.stdout_bytes, 5);
        assert_eq!(output.stderr_bytes, 0);
        assert!(!output.stdout_truncated);
        assert!(!output.stderr_truncated);
        assert_eq!(
            output.stdout_digest,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(
            output.stderr_digest,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn bounded_process_marks_timeout_and_interrupts_on_cancellation() {
        let timed_out = run(&["sleep", "1"], Duration::from_millis(20), 16).unwrap();
        assert!(timed_out.timed_out);
        assert!(!timed_out.interrupted);

        let cwd = tempfile::tempdir().unwrap();
        let argv = vec!["sleep".to_string(), "1".to_string()];
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let interrupted = run_bounded_process(BoundedProcessInput {
            cwd: cwd.path(),
            argv: &argv,
            env: Vec::new(),
            timeout: Duration::from_secs(1),
            output_limit_bytes: 16,
            stdout_limit_bytes: None,
            stderr_limit_bytes: None,
            cancellation: &cancellation,
        })
        .unwrap();
        assert!(interrupted.interrupted);
        assert!(!interrupted.timed_out);
    }

    #[test]
    fn bounded_process_fails_closed_when_output_exceeds_limit() {
        let error = run(&["printf", "abcdef"], Duration::from_secs(1), 3).unwrap_err();
        assert_eq!(error.to_string(), "output_limit_exceeded");
        let process_error = error.downcast_ref::<BoundedProcessError>().unwrap();
        let output = process_error.output().unwrap();
        assert!(process_error.is_output_limit_exceeded());
        assert!(output.output_limit_exceeded);
        assert_eq!(output.stdout_excerpt, "abc");
        assert_eq!(output.stdout_bytes, 3);
        assert_eq!(output.stderr_bytes, 0);
        assert!(output.stdout_truncated);
        assert!(!output.stderr_truncated);
        assert_eq!(
            output.stdout_digest,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_normal_exit_without_descendants_skips_cleanup_grace() {
        let output = run(&["printf", "ok"], Duration::from_secs(1), 16).unwrap();

        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout_excerpt, "ok");
        assert_eq!(
            output.process_tree_term_grace_sleeps, 0,
            "descendant-free normal exit must not wait out TERM cleanup grace"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_timeout_cleans_up_descendant_processes() {
        let cwd = tempfile::tempdir().unwrap();
        let pid_path = cwd.path().join("grandchild.pid");
        let script = format!(
            "sh -c 'echo $$ > {}; trap \"\" TERM; while :; do sleep 1; done' & while [ ! -s {} ]; do sleep 0.01; done; while :; do sleep 1; done",
            pid_path.display(),
            pid_path.display()
        );
        let cancellation = CancellationToken::new();
        let runner_cancellation = cancellation.clone();
        let cwd_path = cwd.path().to_path_buf();
        let output_handle = thread::spawn(move || {
            let argv = vec!["sh".to_string(), "-c".to_string(), script];
            run_bounded_process(BoundedProcessInput {
                cwd: &cwd_path,
                argv: &argv,
                env: Vec::new(),
                timeout: Duration::from_secs(5),
                output_limit_bytes: 1024,
                stdout_limit_bytes: None,
                stderr_limit_bytes: None,
                cancellation: &runner_cancellation,
            })
        });

        let pid = read_published_pid(&pid_path);
        let started = Instant::now();
        cancellation.cancel();
        let output = output_handle.join().unwrap().unwrap();

        assert!(output.interrupted);
        assert!(!output.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "process-tree cleanup blocked until descendant pipe closure"
        );
        assert_process_is_gone(pid);
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_parent_exit_cleans_up_descendant_before_joining_drains() {
        let cwd = tempfile::tempdir().unwrap();
        let pid_path = cwd.path().join("grandchild.pid");
        let script = format!(
            "sh -c 'echo $$ > {}; trap \"\" TERM; while :; do sleep 1; done' & while [ ! -s {} ]; do sleep 0.01; done",
            pid_path.display(),
            pid_path.display()
        );
        let argv = vec!["sh".to_string(), "-c".to_string(), script];
        let started = Instant::now();

        let output = run_bounded_process(BoundedProcessInput {
            cwd: cwd.path(),
            argv: &argv,
            env: Vec::new(),
            timeout: Duration::from_secs(5),
            output_limit_bytes: 1024,
            stdout_limit_bytes: None,
            stderr_limit_bytes: None,
            cancellation: &CancellationToken::new(),
        })
        .unwrap();

        assert_eq!(output.exit_code, Some(0));
        assert!(!output.timed_out);
        assert!(!output.interrupted);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "direct-child exit must not block on descendant-held pipes"
        );
        let pid = read_published_pid(&pid_path);
        assert_process_is_gone(pid);
    }

    #[cfg(unix)]
    fn read_published_pid(pid_path: &Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match fs::read_to_string(pid_path) {
                Ok(content) if !content.trim().is_empty() => {
                    return content.trim().parse::<i32>().unwrap();
                }
                _ if Instant::now() >= deadline => {
                    panic!("descendant did not publish pid at {}", pid_path.display());
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    #[cfg(unix)]
    fn assert_process_is_gone(pid: i32) {
        for _ in 0..20 {
            let status = StdCommand::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stderr(StdStdio::null())
                .status()
                .unwrap();
            if !status.success() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("descendant process {pid} survived bounded-process cleanup");
    }
}
