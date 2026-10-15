//! Private PostgreSQL clusters for tests.
//!
//! The first fixture on a machine runs `initdb`, migrates a template database and stops
//! the server, keeping the data directory as a template keyed by the PostgreSQL version
//! and the kernel migrations. Each test process copies that directory and starts its own
//! `postgres` on a free loopback port with a private socket directory. Clusters whose
//! owning process died are stopped and removed by the next fixture that starts.
use std::{
    fmt,
    fs::{self, File},
    io::{BufRead, BufReader, ErrorKind},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, Connection, PgConnection,
    postgres::{PgConnectOptions, PgSslMode},
};
use uuid::Uuid;
use veoveo_store::{InstanceGrace, MIGRATOR, PlatformStore, StoreConfig};

/// Changes when the template's layout changes, so older templates are not reused.
const TEMPLATE_FORMAT: &str = "veoveo-test-fixtures/postgres/1";
pub(super) const OWNER: &str = "veoveo_owner";
pub(super) const TEMPLATE_DATABASE: &str = "veoveo_template";
const SUPERUSER: &str = "postgres";
const STARTUP: Duration = Duration::from_secs(30);
const COMMAND: Duration = Duration::from_secs(60);
const PORT_ATTEMPTS: usize = 5;

/// A fixture step that failed. The diagnostic names the step and the cause and never
/// includes a password.
#[derive(Debug)]
pub struct Failure {
    pub stage: &'static str,
    pub reason: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Store fixture {}: {}", self.stage, self.reason)
    }
}

impl std::error::Error for Failure {}

impl Failure {
    pub(crate) fn new(stage: &'static str, reason: impl fmt::Display) -> Self {
        Self {
            stage,
            reason: reason.to_string(),
        }
    }
}

fn failure(stage: &'static str, reason: impl fmt::Display) -> Failure {
    Failure::new(stage, reason)
}

/// The PostgreSQL server programs, all from the directory of the `postgres` on `PATH`.
struct Programs {
    postgres: PathBuf,
    initdb: PathBuf,
    pg_ctl: PathBuf,
    createdb: PathBuf,
    dropdb: PathBuf,
    version: String,
}

impl Programs {
    fn find() -> Result<Self, Failure> {
        let requirement = "the fixture requires PostgreSQL 18 `postgres`, `initdb`, `pg_ctl`, \
                           `createdb` and `dropdb` in one directory on PATH";
        let path = std::env::var_os("PATH").unwrap_or_default();
        let directory = std::env::split_paths(&path)
            .find(|directory| directory.join("postgres").is_file())
            .ok_or_else(|| failure("PostgreSQL programs", requirement))?;
        let program = |name: &str| {
            let program = directory.join(name);
            if program.is_file() {
                Ok(program)
            } else {
                Err(failure(
                    "PostgreSQL programs",
                    format!("{name} is missing beside postgres; {requirement}"),
                ))
            }
        };
        let postgres = program("postgres")?;
        let output = Command::new(&postgres)
            .arg("--version")
            .output()
            .map_err(|error| failure("PostgreSQL version", format!("I/O {:?}", error.kind())))?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !version.contains("(PostgreSQL) 18.") {
            return Err(failure(
                "PostgreSQL version",
                format!("found {version:?}; {requirement}"),
            ));
        }
        Ok(Self {
            postgres,
            initdb: program("initdb")?,
            pg_ctl: program("pg_ctl")?,
            createdb: program("createdb")?,
            dropdb: program("dropdb")?,
            version,
        })
    }
}

/// One running cluster owned by this process.
pub(super) struct Cluster {
    programs: Programs,
    directory: PathBuf,
    port: u16,
    owner_password: SecretString,
    child: Mutex<Option<Child>>,
    _owner: File,
}

static SHARED: tokio::sync::Mutex<Weak<Cluster>> = tokio::sync::Mutex::const_new(Weak::new());

impl Cluster {
    /// The cluster this process runs, started on first use. It stops when the last
    /// fixture that holds it is dropped.
    pub(super) async fn shared() -> Result<Arc<Self>, Failure> {
        let mut shared = SHARED.lock().await;
        if let Some(cluster) = shared.upgrade() {
            return Ok(cluster);
        }
        let cluster = Arc::new(Self::start().await?);
        *shared = Arc::downgrade(&cluster);
        Ok(cluster)
    }

    async fn start() -> Result<Self, Failure> {
        let programs = Programs::find()?;
        let root = fixture_root()?;
        sweep_abandoned_clusters(&programs, &root);
        let template = ensure_template(&programs, &root).await?;
        // The directory takes its swept `c-` name only once this process holds its lock.
        let name = &Uuid::new_v4().simple().to_string()[..12];
        let staging = root.join(format!("n-{name}"));
        let directory = root.join(format!("c-{name}"));
        fs::create_dir_all(staging.join("s"))
            .map_err(|error| failure("cluster directory", format!("I/O {:?}", error.kind())))?;
        let owner = File::create(staging.join("owner.lock"))
            .map_err(|error| failure("cluster lock", format!("I/O {:?}", error.kind())))?;
        owner
            .lock()
            .map_err(|error| failure("cluster lock", format!("I/O {:?}", error.kind())))?;
        fs::rename(&staging, &directory)
            .map_err(|error| failure("cluster directory", format!("I/O {:?}", error.kind())))?;
        let mut cluster = Self {
            programs,
            directory,
            port: 0,
            owner_password: random_secret(),
            child: Mutex::new(None),
            _owner: owner,
        };
        copy_directory(&template.join("data"), &cluster.data())
            .map_err(|error| failure("template copy", format!("I/O {:?}", error.kind())))?;
        restrict(&cluster.data())?;
        let (child, port) = launch(&cluster.programs, &cluster.data(), &cluster.socket()).await?;
        *cluster.child.lock().expect("cluster child lock") = Some(child);
        cluster.port = port;
        let mut connection = superuser(&cluster.socket(), port).await?;
        set_owner_password(&mut connection, &cluster.owner_password).await?;
        let _ = connection.close().await;
        Ok(cluster)
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn owner_password(&self) -> &SecretString {
        &self.owner_password
    }

    fn data(&self) -> PathBuf {
        self.directory.join("data")
    }

    fn socket(&self) -> PathBuf {
        self.directory.join("s")
    }

    /// Create `database` as a copy of the migrated template.
    pub(super) async fn create_database(&self, database: &str) -> Result<(), Failure> {
        let mut command = tokio::process::Command::new(&self.programs.createdb);
        command
            .args(["--template", TEMPLATE_DATABASE, "--owner", OWNER])
            .args(self.superuser_arguments())
            .arg(database)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(COMMAND, command.output())
            .await
            .map_err(|_| failure("database creation", format!("exceeded {COMMAND:?}")))?
            .map_err(|error| failure("database creation", format!("I/O {:?}", error.kind())))?;
        if !output.status.success() {
            return Err(failure(
                "database creation",
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }

    /// Drop `database`, ending its sessions.
    pub(super) fn drop_database(&self, database: &str) -> Result<(), Failure> {
        let mut command = Command::new(&self.programs.dropdb);
        command
            .args(["--force", "--if-exists"])
            .args(self.superuser_arguments())
            .arg(database);
        run_bounded(command, "database removal")
    }

    fn superuser_arguments(&self) -> Vec<String> {
        vec![
            "--host".into(),
            self.socket().display().to_string(),
            "--port".into(),
            self.port.to_string(),
            "--username".into(),
            SUPERUSER.into(),
        ]
    }

    /// The owner's connection settings for `database`, with a small pool.
    pub(super) fn owner_config(&self, database: &str) -> StoreConfig {
        StoreConfig::builder(
            format!(
                "postgres://{OWNER}@127.0.0.1:{}/{database}?sslmode=disable",
                self.port
            ),
            self.owner_password.clone(),
        )
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        // A test reaps a dead instance at once and expects it gone; tests of the grace
        // set their own.
        .instance_grace(InstanceGrace::ZERO)
        .build()
        .expect("fixture owner configuration")
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        stop(&self.programs, &self.data());
        if let Some(mut child) = self.child.get_mut().ok().and_then(Option::take) {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// `$TMPDIR/vvpg`: short, so socket paths stay within the 103-byte limit.
fn fixture_root() -> Result<PathBuf, Failure> {
    let root = std::env::temp_dir().join("vvpg");
    fs::create_dir_all(&root)
        .map_err(|error| failure("fixture directory", format!("I/O {:?}", error.kind())))?;
    Ok(root)
}

/// Stop and remove clusters whose owning process no longer holds their lock.
fn sweep_abandoned_clusters(programs: &Programs, root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let directory = entry.path();
        let named = directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("c-"));
        let Ok(lock) = File::open(directory.join("owner.lock")) else {
            continue;
        };
        if !named || lock.try_lock().is_err() {
            continue;
        }
        stop(programs, &directory.join("data"));
        let _ = fs::remove_dir_all(&directory);
    }
}

/// The migrated template data directory for this PostgreSQL version and these kernel
/// migrations, built once under a file lock.
async fn ensure_template(programs: &Programs, root: &Path) -> Result<PathBuf, Failure> {
    let mut key = Sha256::new();
    key.update(TEMPLATE_FORMAT.as_bytes());
    key.update(programs.version.as_bytes());
    for migration in MIGRATOR.iter() {
        key.update(migration.version.to_be_bytes());
        key.update(&*migration.checksum);
    }
    let key = hex::encode(&key.finalize()[..8]);
    let template = root.join(format!("template-{key}"));
    let lock = File::create(root.join(format!("template-{key}.lock")))
        .map_err(|error| failure("template lock", format!("I/O {:?}", error.kind())))?;
    let lock = tokio::task::spawn_blocking(move || lock.lock().map(|()| lock))
        .await
        .map_err(|error| failure("template lock", error))?
        .map_err(|error| failure("template lock", format!("I/O {:?}", error.kind())))?;
    if template.join("ready").is_file() {
        return Ok(template);
    }
    let _ = fs::remove_dir_all(&template);
    let build = root.join(format!("b-{}", &Uuid::new_v4().simple().to_string()[..12]));
    let built = build_template(programs, &build).await;
    let renamed = built.and_then(|()| {
        fs::rename(&build, &template)
            .map_err(|error| failure("template build", format!("I/O {:?}", error.kind())))
    });
    if renamed.is_err() {
        let _ = fs::remove_dir_all(&build);
    }
    drop(lock);
    renamed.map(|()| template)
}

async fn build_template(programs: &Programs, build: &Path) -> Result<(), Failure> {
    let data = build.join("data");
    let socket = build.join("s");
    fs::create_dir_all(&socket)
        .map_err(|error| failure("template build", format!("I/O {:?}", error.kind())))?;
    let mut initdb = Command::new(&programs.initdb);
    initdb.arg("--pgdata").arg(&data).args([
        "--username",
        SUPERUSER,
        "--auth-local=trust",
        "--auth-host=scram-sha-256",
        "--encoding=UTF8",
        "--no-locale",
        "--no-instructions",
        "--no-sync",
        "--wal-segsize=1",
    ]);
    run_bounded_async(initdb, "initdb").await?;
    let (child, port) = launch(programs, &data, &socket).await?;
    let mut child = ChildGuard(Some(child));
    let migrated = async {
        let mut connection = superuser(&socket, port).await?;
        sqlx::raw_sql(include_str!("create_owner.sql"))
            .execute(&mut connection)
            .await
            .map_err(|error| failure("template owner", error))?;
        let password = random_secret();
        set_owner_password(&mut connection, &password).await?;
        let _ = connection.close().await;
        let mut createdb = Command::new(&programs.createdb);
        createdb
            .args(["--owner", OWNER, "--host"])
            .arg(&socket)
            .args(["--port", &port.to_string(), "--username", SUPERUSER])
            .arg(TEMPLATE_DATABASE);
        run_bounded_async(createdb, "template database").await?;
        let config = StoreConfig::builder(
            format!("postgres://{OWNER}@127.0.0.1:{port}/{TEMPLATE_DATABASE}?sslmode=disable"),
            password,
        )
        .max_connections(1)
        .build()
        .map_err(|error| failure("template migration", error))?;
        let store = PlatformStore::connect(config)
            .await
            .map_err(|error| failure("template migration", error))?;
        let migrated = tokio::time::timeout(COMMAND, store.migrate())
            .await
            .map_err(|_| failure("template migration", format!("exceeded {COMMAND:?}")))?
            .map_err(|error| failure("template migration", error));
        store.close().await;
        migrated
    }
    .await;
    let mut pg_ctl = Command::new(&programs.pg_ctl);
    pg_ctl
        .args(["stop", "--mode=fast", "--wait", "--timeout=60", "--pgdata"])
        .arg(&data);
    let stopped = run_bounded_async(pg_ctl, "template server stop").await;
    if let Some(mut process) = child.0.take() {
        let _ = process.wait();
    }
    migrated?;
    stopped?;
    let _ = fs::remove_dir_all(&socket);
    File::create(build.join("ready"))
        .map_err(|error| failure("template build", format!("I/O {:?}", error.kind())))?;
    Ok(())
}

/// Kills a server that a failed template build left running.
struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Start `postgres` on `data` on a free loopback port, moving to a fresh port when the
/// server exits during startup, and wait until the superuser can connect.
async fn launch(programs: &Programs, data: &Path, socket: &Path) -> Result<(Child, u16), Failure> {
    let log = data
        .parent()
        .expect("data directory has a parent")
        .join("postgres.log");
    let mut exits = Vec::new();
    for _ in 0..PORT_ATTEMPTS {
        let port = free_port()?;
        let output = File::create(&log)
            .map_err(|error| failure("server log", format!("I/O {:?}", error.kind())))?;
        let errors = output
            .try_clone()
            .map_err(|error| failure("server log", format!("I/O {:?}", error.kind())))?;
        let mut child = Command::new(&programs.postgres)
            .arg("-D")
            .arg(data)
            .arg("-k")
            .arg(socket)
            .args(["-p", &port.to_string()])
            .args([
                "-c",
                "listen_addresses=127.0.0.1",
                "-c",
                "fsync=off",
                "-c",
                "synchronous_commit=off",
                "-c",
                "full_page_writes=off",
                "-c",
                "shared_buffers=32MB",
                "-c",
                "max_connections=100",
                "-c",
                "log_min_error_statement=panic",
            ])
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(errors)
            .spawn()
            .map_err(|error| failure("server start", format!("I/O {:?}", error.kind())))?;
        let deadline = Instant::now() + STARTUP;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                exits.push(format!("port {port}: {status}"));
                break;
            }
            if socket.join(format!(".s.PGSQL.{port}")).exists() && accepting(socket, port).await {
                return Ok((child, port));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure(
                    "server start",
                    format!("not ready within {STARTUP:?}; log: {}", log_tail(&log)),
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    Err(failure(
        "server start",
        format!(
            "exited during startup ({}); log: {}",
            exits.join(", "),
            log_tail(&log)
        ),
    ))
}

/// Whether the server accepts a superuser session on its socket.
async fn accepting(socket: &Path, port: u16) -> bool {
    let options = superuser_options(socket, port);
    match tokio::time::timeout(Duration::from_secs(2), options.connect()).await {
        Ok(Ok(connection)) => {
            let _ = connection.close().await;
            true
        }
        _ => false,
    }
}

fn superuser_options(socket: &Path, port: u16) -> PgConnectOptions {
    PgConnectOptions::new_without_pgpass()
        .socket(socket)
        .port(port)
        .username(SUPERUSER)
        .database("postgres")
        .ssl_mode(PgSslMode::Disable)
}

async fn superuser(socket: &Path, port: u16) -> Result<PgConnection, Failure> {
    tokio::time::timeout(
        Duration::from_secs(10),
        superuser_options(socket, port).connect(),
    )
    .await
    .map_err(|_| failure("superuser session", "exceeded 10 seconds"))?
    .map_err(|error| failure("superuser session", error))
}

async fn set_owner_password(
    connection: &mut PgConnection,
    password: &SecretString,
) -> Result<(), Failure> {
    sqlx::raw_sql(include_str!("define_owner_password.sql"))
        .execute(&mut *connection)
        .await
        .map_err(|error| failure("owner password", error))?;
    sqlx::query(include_str!("set_owner_password.sql"))
        .bind(password.expose_secret())
        .execute(&mut *connection)
        .await
        .map_err(|_| failure("owner password", "ALTER ROLE failed"))?;
    Ok(())
}

/// Stop a cluster's server, if one runs on `data`, without waiting for sessions.
fn stop(programs: &Programs, data: &Path) {
    if !data.join("postmaster.pid").exists() {
        return;
    }
    let mut pg_ctl = Command::new(&programs.pg_ctl);
    pg_ctl
        .args([
            "stop",
            "--mode=immediate",
            "--wait",
            "--timeout=10",
            "--pgdata",
        ])
        .arg(data);
    let _ = run_bounded(pg_ctl, "server stop");
}

async fn run_bounded_async(command: Command, stage: &'static str) -> Result<(), Failure> {
    tokio::task::spawn_blocking(move || run_bounded(command, stage))
        .await
        .map_err(|error| failure(stage, error))?
}

/// Run a program to completion within [`COMMAND`], keeping its error output for the
/// diagnostic.
fn run_bounded(mut command: Command, stage: &'static str) -> Result<(), Failure> {
    let log = std::env::temp_dir().join(format!("vvpg-{}.log", Uuid::new_v4().simple()));
    let output =
        File::create(&log).map_err(|error| failure(stage, format!("I/O {:?}", error.kind())))?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(output)
        .spawn()
        .map_err(|error| failure(stage, format!("I/O {:?}", error.kind())))?;
    let deadline = Instant::now() + COMMAND;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(failure(stage, format!("exceeded {COMMAND:?}")));
            }
            Err(error) => break Err(failure(stage, format!("I/O {:?}", error.kind()))),
        }
    };
    let tail = log_tail(&log);
    let _ = fs::remove_file(&log);
    match status? {
        status if status.success() => Ok(()),
        status => Err(failure(stage, format!("{status}: {tail}"))),
    }
}

fn log_tail(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return "no log".to_owned();
    };
    let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    lines[lines.len().saturating_sub(12)..].join(" | ")
}

fn free_port() -> Result<u16, Failure> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| failure("port selection", format!("I/O {:?}", error.kind())))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| failure("port selection", format!("I/O {:?}", error.kind())))
}

fn copy_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        } else {
            return Err(std::io::Error::from(ErrorKind::Unsupported));
        }
    }
    Ok(())
}

/// PostgreSQL requires its data directory to be private to its owner.
fn restrict(data: &Path) -> Result<(), Failure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(data, fs::Permissions::from_mode(0o700))
            .map_err(|error| failure("data directory", format!("I/O {:?}", error.kind())))?;
    }
    Ok(())
}

pub(super) fn random_secret() -> SecretString {
    SecretString::from(format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ))
}
