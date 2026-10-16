//! The managed Console owns one bounded sink for its own and Runtime output.
//! MCP stdio is deliberately outside this path: stdout there is the protocol.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            process::ExitStatusExt,
        },
    },
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const ARCHIVES: usize = 2;
const MAX_LINE_BYTES: usize = 4096;
const REPEAT_INTERVAL: Duration = Duration::from_secs(60);

pub(super) fn install(home: &Path) -> Result<CaptureGuard> {
    install_directory(
        &home.join("logs"),
        "pcp-console.log",
        ["pcp-console-stdout.log", "pcp-console-stderr.log"],
    )
}

fn install_directory(directory: &Path, name: &str, legacy: [&str; 2]) -> Result<CaptureGuard> {
    fs::create_dir_all(&directory).context("create PCP diagnostic log directory")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let lock = private_options()
        .read(true)
        .write(true)
        .create(true)
        .open(directory.join(format!(".{name}.lock")))?;
    // A duplicate Console must not rotate the running owner's files.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error()).context("PCP diagnostic log already owned");
    }
    let mut log = RotatingLog::open(directory.join(name), MAX_FILE_BYTES)?;
    // Retain only a bounded tail of the old error file on first adoption.
    let legacy_error = directory.join(legacy[1]);
    if log.bytes == 0 && legacy_error.is_file() {
        let mut input = private_options().read(true).open(&legacy_error)?;
        let size = input.metadata()?.len();
        input.seek(SeekFrom::Start(size.saturating_sub(MAX_FILE_BYTES)))?;
        let mut tail = Vec::new();
        input.take(MAX_FILE_BYTES).read_to_end(&mut tail)?;
        log.write(&tail)?;
    }
    let (reader, writer) = pipe()?;
    let mut original = Vec::new();
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error()).context("retain original diagnostic stream");
        }
        original.push((fd, unsafe { File::from_raw_fd(duplicate) }));
    }
    let worker = thread::Builder::new()
        .name("pcp-diagnostic-log".into())
        .spawn(move || {
            let _lock = lock;
            capture(reader, log);
        })
        .context("start PCP diagnostic log writer")?;
    let guard = CaptureGuard {
        original,
        worker: Some(worker),
    };
    io::stdout().flush()?;
    io::stderr().flush()?;
    // dup2 intentionally leaves fd 1/2 inheritable. The owned Runtime then
    // shares this pipe, so child stderr cannot bypass rotation through launchd.
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        if unsafe { libc::dup2(writer.as_raw_fd(), fd) } < 0 {
            return Err(io::Error::last_os_error()).context("capture PCP diagnostic stream");
        }
    }
    drop(writer);
    for name in legacy {
        remove_if_present(&directory.join(name))?;
    }
    Ok(guard)
}

pub(super) struct CaptureGuard {
    original: Vec<(i32, File)>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        for (fd, file) in &self.original {
            unsafe { libc::dup2(file.as_raw_fd(), *fd) };
        }
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(250);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

/// The existing tunnel LaunchAgent uses this same writer, retaining its exact
/// command/environment and forwarding shutdown to its owned child.
pub(super) async fn run_service(arguments: impl Iterator<Item = OsString>) -> Result<i32> {
    let mut arguments = arguments;
    anyhow::ensure!(
        arguments.next().as_deref() == Some(std::ffi::OsStr::new("--log-dir")),
        "logged tunnel service requires --log-dir"
    );
    let directory = PathBuf::from(arguments.next().context("missing tunnel log directory")?);
    anyhow::ensure!(
        arguments.next().as_deref() == Some(std::ffi::OsStr::new("--")),
        "logged tunnel service requires -- before its command"
    );
    let binary = arguments
        .next()
        .context("missing tunnel service executable")?;
    let _capture = install_directory(&directory, "pcp-tunnel.log", ["stdout.log", "stderr.log"])?;
    let result = async {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut child = tokio::process::Command::new(binary)
            .args(arguments)
            .kill_on_drop(true)
            .spawn()
            .context("start logged tunnel service")?;
        let status = tokio::select! {
            result=child.wait()=>result?,
            _=terminate.recv()=>stop_logged_child(&mut child,libc::SIGTERM).await?,
            _=interrupt.recv()=>stop_logged_child(&mut child,libc::SIGINT).await?,
        };
        Ok(status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
    }
    .await;
    if let Err(error) = &result {
        eprintln!("PCP tunnel service failed: {error:#}");
    }
    result
}

async fn stop_logged_child(
    child: &mut tokio::process::Child,
    signal: i32,
) -> Result<std::process::ExitStatus> {
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    if let Some(pid) = child.id() {
        unsafe { libc::kill(pid as i32, signal) };
    }
    match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(result) => Ok(result?),
        Err(_) => {
            child.start_kill()?;
            Ok(child.wait().await?)
        }
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    options
}

fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Ownership transfers exactly once to File, including all failure paths.
    let read = unsafe { File::from_raw_fd(fds[0]) };
    let write = unsafe { File::from_raw_fd(fds[1]) };
    for file in [&read, &write] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((read, write))
}

fn capture(mut reader: File, log: RotatingLog) {
    // Read fixed chunks, not lines: an unterminated/error-storm line is bounded too.
    let mut bytes = [0; 8192];
    let mut sink = DiagnosticLines::new(log);
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                // Never recursively log a logging error. Keep draining so a
                // full/unavailable disk cannot deadlock Console or its Runtime.
                let _ = sink.push(&bytes[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = sink.finish();
}

struct DiagnosticLines {
    log: RotatingLog,
    line: Vec<u8>,
    skipping_line: bool,
    previous: Vec<u8>,
    repeats: u64,
    last_summary: Instant,
}

impl DiagnosticLines {
    fn new(log: RotatingLog) -> Self {
        Self {
            log,
            line: Vec::with_capacity(MAX_LINE_BYTES),
            skipping_line: false,
            previous: Vec::new(),
            repeats: 0,
            last_summary: Instant::now(),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &byte in bytes {
            if self.skipping_line {
                if byte == b'\n' {
                    self.skipping_line = false;
                }
                continue;
            }
            if byte == b'\n' {
                self.emit_line()?;
            } else if self.line.len() == MAX_LINE_BYTES {
                let mut keep = MAX_LINE_BYTES - b" [line truncated]".len();
                if let Err(error) = std::str::from_utf8(&self.line[..keep])
                    && error.error_len().is_none()
                {
                    keep = error.valid_up_to();
                }
                self.line.truncate(keep);
                self.line.extend_from_slice(b" [line truncated]");
                self.skipping_line = true;
                self.emit_line()?;
            } else {
                self.line.push(byte);
            }
        }
        Ok(())
    }

    fn emit_line(&mut self) -> io::Result<()> {
        let mut line = std::mem::take(&mut self.line);
        let result = self.emit_value(&line);
        line.clear();
        self.line = line;
        result
    }

    fn emit_value(&mut self, line: &[u8]) -> io::Result<()> {
        if line.is_empty() {
            return Ok(());
        }
        if line == self.previous && !self.previous.is_empty() {
            self.repeats = self.repeats.saturating_add(1);
            if self.last_summary.elapsed() >= REPEAT_INTERVAL {
                self.flush_repeats()?;
            }
            return Ok(());
        }
        self.flush_repeats()?;
        let mut entry = chrono::Utc::now().to_rfc3339().into_bytes();
        entry.push(b' ');
        entry.extend_from_slice(line);
        entry.push(b'\n');
        self.log.write_record(&entry)?;
        self.previous.clear();
        self.previous.extend_from_slice(line);
        self.last_summary = Instant::now();
        Ok(())
    }

    fn flush_repeats(&mut self) -> io::Result<()> {
        if self.repeats > 0 {
            self.log.write_record(
                format!(
                    "{} Previous diagnostic repeated {} times.\n",
                    chrono::Utc::now().to_rfc3339(),
                    self.repeats
                )
                .as_bytes(),
            )?;
            self.repeats = 0;
        }
        self.last_summary = Instant::now();
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        if !self.line.is_empty() {
            self.emit_line()?;
        }
        self.flush_repeats()
    }
}

struct RotatingLog {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
}

impl RotatingLog {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if max_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty diagnostic log budget",
            ));
        }
        for n in 0..=ARCHIVES {
            let item = archive(&path, n);
            if item.try_exists()? {
                let mut file = private_options().read(true).write(true).open(&item)?;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
                let size = file.metadata()?.len();
                if size > max_bytes {
                    file.seek(SeekFrom::Start(size - max_bytes))?;
                    let mut tail = Vec::new();
                    (&mut file).take(max_bytes).read_to_end(&mut tail)?;
                    file.set_len(0)?;
                    file.seek(SeekFrom::Start(0))?;
                    file.write_all(&tail)?;
                }
            }
        }
        let file = private_options().append(true).create(true).open(&path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
            max_bytes,
        })
    }

    fn write_record(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 <= self.max_bytes && self.bytes + bytes.len() as u64 > self.max_bytes
        {
            self.rotate()?;
        }
        self.write(bytes)
    }

    fn write(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            if self.bytes >= self.max_bytes {
                self.rotate()?;
            }
            if self.file.is_none() {
                self.file = Some(
                    private_options()
                        .append(true)
                        .create(true)
                        .open(&self.path)?,
                );
            }
            let n = bytes.len().min((self.max_bytes - self.bytes) as usize);
            let file = self.file.as_mut().unwrap();
            if let Err(error) = file.write_all(&bytes[..n]) {
                // write_all may have made partial progress before an I/O error.
                self.bytes = file.metadata().map(|m| m.len()).unwrap_or(self.max_bytes);
                return Err(error);
            }
            self.bytes += n as u64;
            bytes = &bytes[n..];
        }
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        remove_if_present(&archive(&self.path, ARCHIVES))?;
        for n in (1..=ARCHIVES).rev() {
            let from = archive(&self.path, n - 1);
            if from.try_exists()? {
                fs::rename(from, archive(&self.path, n))?;
            }
        }
        self.file = Some(
            private_options()
                .append(true)
                .create(true)
                .open(&self.path)?,
        );
        self.bytes = 0;
        Ok(())
    }
}

fn archive(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return path.to_owned();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn root() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pcp-log-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn continuous_output_and_unterminated_lines_keep_only_three_bounded_files() {
        let root = root();
        let path = root.join("console.log");
        let mut log = RotatingLog::open(path.clone(), 64).unwrap();
        for _ in 0..100 {
            log.write(&[b'x'; 97]).unwrap();
        }
        log.write(b"latest diagnostic").unwrap();
        drop(log);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 3);
        for n in 0..=ARCHIVES {
            assert!(fs::metadata(archive(&path, n)).unwrap().len() <= 64);
        }
        assert!(fs::read(&path).unwrap().ends_with(b"latest diagnostic"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_bounds_existing_files_and_retains_their_newest_tail() {
        let root = root();
        let path = root.join("console.log");
        for n in 0..=ARCHIVES {
            fs::write(
                archive(&path, n),
                [vec![b'a'; 1000], b"last bytes".to_vec()].concat(),
            )
            .unwrap();
        }
        let log = RotatingLog::open(path.clone(), 64).unwrap();
        drop(log);
        for n in 0..=ARCHIVES {
            let bytes = fs::read(archive(&path, n)).unwrap();
            assert_eq!(bytes.len(), 64);
            assert!(bytes.ends_with(b"last bytes"));
            assert_eq!(
                fs::metadata(archive(&path, n))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_stdout_and_stderr_are_captured_in_the_same_bounded_sink() {
        let root = root();
        let path = root.join("console.log");
        let log = RotatingLog::open(path.clone(), 1024).unwrap();
        let (reader, writer) = pipe().unwrap();
        let task = thread::spawn(move || capture(reader, log));
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf 'startup\\n'; printf 'failure\\n' >&2"])
            .stdout(Stdio::from(writer.try_clone().unwrap()))
            .stderr(Stdio::from(writer));
        assert!(command.status().unwrap().success());
        drop(command);
        task.join().unwrap();
        let output = fs::read_to_string(path).unwrap();
        assert!(output.contains("startup\n") && output.contains("failure\n"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_errors_are_counted_and_long_lines_never_accumulate_in_memory() {
        let root = root();
        let path = root.join("console.log");
        let mut sink =
            DiagnosticLines::new(RotatingLog::open(path.clone(), MAX_FILE_BYTES).unwrap());
        for _ in 0..10_000 {
            sink.push(b"same error\n").unwrap();
        }
        sink.push(&vec![b'x'; 100_000]).unwrap();
        assert!(sink.line.len() <= MAX_LINE_BYTES + 32);
        sink.push(b"\nnext error\n").unwrap();
        sink.finish().unwrap();
        drop(sink);
        let output = fs::read_to_string(path).unwrap();
        assert_eq!(output.matches("same error").count(), 1);
        assert!(output.contains("repeated 9999 times"));
        assert!(output.contains("[line truncated]"));
        assert!(output.contains("next error"));
        assert!(output.len() < 5000);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_sink_cannot_turn_truncated_lines_into_unbounded_buffers() {
        let root = root();
        let path = root.join("console.log");
        let mut log = RotatingLog::open(path.clone(), MAX_FILE_BYTES).unwrap();
        log.file = Some(File::open(&path).unwrap());
        let mut sink = DiagnosticLines::new(log);
        for _ in 0..1000 {
            let _ = sink.push(&vec![b'x'; MAX_LINE_BYTES * 2]);
            let _ = sink.push(b"\nnext failed diagnostic\n");
            assert!(sink.line.len() <= MAX_LINE_BYTES);
            assert!(sink.previous.len() <= MAX_LINE_BYTES);
        }
        sink.log.file = Some(private_options().append(true).open(&path).unwrap());
        sink.push(b"\nrecovered sink\n").unwrap();
        sink.finish().unwrap();
        drop(sink);
        assert!(fs::read_to_string(path).unwrap().contains("recovered sink"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utf8_lines_stay_readable_across_truncation_and_rotation() {
        let root = root();
        let path = root.join("console.log");
        let mut sink = DiagnosticLines::new(RotatingLog::open(path.clone(), 5000).unwrap());
        for n in 0..4 {
            let line = format!("{n}{}\n", "界".repeat(2000));
            sink.push(line.as_bytes()).unwrap();
        }
        sink.finish().unwrap();
        drop(sink);
        for n in 0..=ARCHIVES {
            let content = fs::read_to_string(archive(&path, n)).unwrap();
            assert!(content.contains("[line truncated]"));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
