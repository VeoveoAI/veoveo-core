//! Owned local SurrealDB process for the native Store fixture.
//!
//! Each fixture runs its own `surreal` server on a loopback port, so tests stay isolated
//! without a container runtime. Development environments install the pinned SurrealDB
//! 3.3.0 binary as `surreal` on `PATH`.
use std::{
    fmt,
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tokio::process::{Child, Command};
use uuid::Uuid;

const PROGRAM: &str = "surreal";
const STARTUP: Duration = Duration::from_secs(30);

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
    port: u16,
    child: Mutex<Option<Child>>,
}

impl Server {
    pub async fn start(storage: Storage, password: &str) -> Result<(Self, String), Failure> {
        let name = format!("veoveo-native-store-test-{}", Uuid::now_v7().simple());
        let directory = std::env::temp_dir().join(&name);
        let mut server = Self {
            name,
            storage,
            directory,
            password: password.to_owned(),
            port: 0,
            child: Mutex::new(None),
        };
        std::fs::create_dir_all(&server.directory)
            .map_err(|error| server.failure("data directory", format!("I/O {:?}", error.kind())))?;
        server.port = free_port().map_err(|error| server.failure("port selection", error))?;
        let child = server.spawn().await?;
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

    /// Restart only this guard's server on the same port and data directory.
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
        let child = self.spawn().await?;
        *self.child.lock().expect("fixture child lock") = Some(child);
        Ok(self.endpoint())
    }

    fn endpoint(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    async fn spawn(&self) -> Result<Child, Failure> {
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
                &format!("127.0.0.1:{}", self.port),
                &storage,
            ])
            .env("SURREAL_USER", "fixture_admin")
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
                return Err(
                    self.failure("server start", format!("exited with {:?}", status.code()))
                );
            }
            if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, self.port))
                .await
                .is_ok()
            {
                return Ok(child);
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(
                    self.failure("server start", format!("not listening within {STARTUP:?}"))
                );
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
