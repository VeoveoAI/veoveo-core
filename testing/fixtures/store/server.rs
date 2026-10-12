//! Owned local SurrealDB process for the native Store fixture.
//!
//! Each fixture runs its own `surreal` server on a loopback port, so tests stay isolated
//! without a container runtime. Development environments install the pinned SurrealDB
//! 3.3.0 binary as `surreal` on `PATH`.
//!
//! A port chosen by binding port 0 is released before `surreal` binds it, so another
//! process can take it in between. The fixture therefore accepts a new server only when
//! its own child is still running and an HTTP sign-in with the fixture's generated
//! credentials succeeds, and it starts again on a fresh port when the child exits. A
//! restarted persistent server may hold credentials that a test replaced, so a restart
//! accepts the server once its own child is running and SurrealDB reports healthy.
use std::{
    fmt,
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
    process::Stdio,
    sync::{
        Mutex,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};
use uuid::Uuid;

const PROGRAM: &str = "surreal";
const STARTUP: Duration = Duration::from_secs(30);
const USERNAME: &str = "fixture_admin";
/// Ports tried before startup fails; each exit during startup moves to a fresh port.
const PORT_ATTEMPTS: usize = 5;
const PROBE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub enum Storage {
    Memory,
    #[allow(
        dead_code,
        reason = "Each fixture consumer selects its storage qualification"
    )]
    RocksDb,
}

#[derive(Debug)]
pub struct Failure {
    pub stage: &'static str,
    server: String,
    reason: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Store fixture {}: {}: {}",
            self.server, self.stage, self.reason
        )
    }
}

impl std::error::Error for Failure {}

pub struct Server {
    name: String,
    storage: Storage,
    directory: PathBuf,
    password: String,
    port: AtomicU16,
    child: Mutex<Option<Child>>,
}

/// One launch attempt either yields a ready child, or the child exited during startup
/// (usually because another process holds the port), or the fixture cannot proceed.
enum Launch {
    Ready(Child),
    Exited(Option<i32>),
}

/// What the fixture requires before it accepts a running child as ready.
#[derive(Clone, Copy)]
enum Readiness {
    /// The fixture's generated credentials sign in.
    SignIn,
    /// SurrealDB answers its health endpoint.
    Health,
}

impl Server {
    pub async fn start(storage: Storage, password: &str) -> Result<(Self, String), Failure> {
        let name = format!("veoveo-native-store-test-{}", Uuid::now_v7().simple());
        let directory = std::env::temp_dir().join(&name);
        let server = Self {
            name,
            storage,
            directory,
            password: password.to_owned(),
            port: AtomicU16::new(0),
            child: Mutex::new(None),
        };
        std::fs::create_dir_all(&server.directory)
            .map_err(|error| server.failure("data directory", format!("I/O {:?}", error.kind())))?;
        let child = server.launch(false, Readiness::SignIn).await?;
        *server.child.lock().expect("fixture child lock") = Some(child);
        let endpoint = server.endpoint();
        Ok((server, endpoint))
    }

    #[allow(
        dead_code,
        reason = "Only write-cost measurements inspect retained storage"
    )]
    pub async fn storage_bytes(&self) -> Result<u64, Failure> {
        directory_bytes(&self.directory).map_err(|error| {
            self.failure(
                "measurement storage size",
                format!("I/O {:?}", error.kind()),
            )
        })
    }

    #[allow(
        dead_code,
        reason = "Only Linux write-cost measurements inspect process I/O"
    )]
    pub async fn local_process_id(&self) -> Result<u32, Failure> {
        self.child
            .lock()
            .expect("fixture child lock")
            .as_ref()
            .and_then(Child::id)
            .ok_or_else(|| {
                self.failure("measurement process identity", "expected a running process")
            })
    }

    /// Restart only this guard's server on its data directory, on the same port when it is
    /// still free and otherwise on a fresh one; the returned endpoint names the port.
    #[allow(
        dead_code,
        reason = "Only retained-store qualification restarts its server"
    )]
    pub async fn restart(&self) -> Result<String, Failure> {
        let previous = self.child.lock().expect("fixture child lock").take();
        if let Some(mut child) = previous {
            child
                .kill()
                .await
                .map_err(|error| self.failure("server stop", format!("I/O {:?}", error.kind())))?;
        }
        let child = self.launch(true, Readiness::Health).await?;
        *self.child.lock().expect("fixture child lock") = Some(child);
        Ok(self.endpoint())
    }

    fn port(&self) -> u16 {
        self.port.load(Ordering::SeqCst)
    }

    fn endpoint(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port())
    }

    /// Start the server, moving to a fresh port whenever the child exits during startup.
    async fn launch(&self, keep_port: bool, readiness: Readiness) -> Result<Child, Failure> {
        let mut exits = Vec::new();
        for attempt in 0..PORT_ATTEMPTS {
            if attempt > 0 || !keep_port {
                let port = free_port().map_err(|error| self.failure("port selection", error))?;
                self.port.store(port, Ordering::SeqCst);
            }
            match self.spawn(readiness).await? {
                Launch::Ready(child) => return Ok(child),
                Launch::Exited(code) => exits.push(format!("port {} exit {code:?}", self.port())),
            }
        }
        Err(self.failure(
            "server exited during startup",
            format!("on {PORT_ATTEMPTS} ports: {}", exits.join(", ")),
        ))
    }

    async fn spawn(&self, readiness: Readiness) -> Result<Launch, Failure> {
        let storage = match self.storage {
            Storage::Memory => "memory".to_owned(),
            Storage::RocksDb => format!("rocksdb:{}", self.directory.join("data").display()),
        };
        // Never print arguments, environment or child output: they carry the password.
        let mut child = Command::new(PROGRAM)
            .args([
                "start",
                "--log",
                "error",
                "--bind",
                &format!("127.0.0.1:{}", self.port()),
                &storage,
            ])
            .env("SURREAL_USER", USERNAME)
            .env("SURREAL_PASS", &self.password)
            .env("SURREAL_ROCKSDB_BLOCK_CACHE_SIZE", "67108864")
            .env("SURREAL_ROCKSDB_WRITE_BUFFER_SIZE", "16777216")
            .env("SURREAL_ROCKSDB_MAX_WRITE_BUFFER_NUMBER", "2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                self.failure(
                    "server start",
                    format!(
                        "I/O {:?}; the fixture requires the SurrealDB 3.3.0 `{PROGRAM}` binary on PATH",
                        error.kind()
                    ),
                )
            })?;
        let deadline = tokio::time::Instant::now() + STARTUP;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Ok(Launch::Exited(status.code()));
            }
            // Only our own server accepts the generated credentials, and the child must
            // still be running afterwards, so a listener started by another process on a
            // reused port is never mistaken for the fixture's server.
            let ready = match readiness {
                Readiness::SignIn => signs_in(self.port(), &self.password).await,
                Readiness::Health => healthy(self.port()).await,
            };
            if ready && matches!(child.try_wait(), Ok(None)) {
                return Ok(Launch::Ready(child));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(self.failure("server start", format!("not ready within {STARTUP:?}")));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn failure(&self, stage: &'static str, reason: impl Into<String>) -> Failure {
        Failure {
            stage,
            server: self.name.clone(),
            reason: reason.into(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.get_mut().ok().and_then(Option::take) {
            let _ = child.start_kill();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Sign in over SurrealDB's HTTP endpoint with the fixture's credentials.
async fn signs_in(port: u16, password: &str) -> bool {
    let body = format!(
        "{{\"user\":\"{}\",\"pass\":\"{}\"}}",
        json_escape(USERNAME),
        json_escape(password)
    );
    answers_ok(port, "POST", "/signin", &body).await
}

/// SurrealDB's health endpoint answers once the server accepts requests.
async fn healthy(port: u16) -> bool {
    answers_ok(port, "GET", "/health", "").await
}

async fn answers_ok(port: u16, method: &str, path: &str, body: &str) -> bool {
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .ok()?;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut status = [0u8; 12];
        stream.read_exact(&mut status).await.ok()?;
        Some(status.starts_with(b"HTTP/1.1 200"))
    };
    matches!(tokio::time::timeout(PROBE, probe).await, Ok(Some(true)))
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            control if control.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", u32::from(control)))
            }
            other => escaped.push(other),
        }
    }
    escaped
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("I/O {:?}", error.kind()))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("I/O {:?}", error.kind()))
}

fn directory_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total += if metadata.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            metadata.len()
        };
    }
    Ok(total)
}
