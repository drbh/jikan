use anyhow::{Context as _, Result};
use chrono::{DateTime, Datelike, Local, Timelike};
use cron::Schedule as CronSchedule;
use daemonize::Daemonize;
use parking_lot::RwLock;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{
    fs::File,
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::runtime::Runtime;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    net::TcpStream as TokioTcpStream,
};
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use walkdir::WalkDir;

const WORKFLOWS: TableDefinition<(&str, &str), &str> = TableDefinition::new("workflows");
const NAMESPACES: TableDefinition<&str, &str> = TableDefinition::new("namespaces");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WorkflowYaml {
    name: String,
    #[serde(rename = "run-name")]
    run_name: Option<String>,
    on: On,
    jobs: Jobs,
    #[serde(rename = "env")]
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct On {
    schedule: Vec<Schedule>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Schedule {
    cron: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Jobs {
    #[serde(flatten)]
    job_map: HashMap<String, Job>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Job {
    #[serde(rename = "runs-on")]
    runs_on: RunOnBackend,
    steps: Vec<Step>,
    #[serde(rename = "if")]
    condition: Option<String>,
    #[serde(rename = "env")]
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
enum RunOnBackend {
    PythonExternal(Option<String>),
    Bash,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Step {
    run: Option<String>,
    name: Option<String>,
    uses: Option<String>,
}

trait JobTrait: Send + Sync {
    fn cron(&self) -> &str;
    fn run(&mut self) -> Result<(), String>;
    fn clone_box(&self) -> Box<dyn JobTrait>;
    fn downcast_ref_to_workflow_job(&self) -> Option<&WorkflowJob>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Context {
    env: HashMap<String, String>,
}

impl Context {
    fn new() -> Self {
        Context {
            env: HashMap::new(),
        }
    }

    fn set_env(&mut self, key: String, value: String) {
        self.env.insert(key, value);
    }

    fn get_env(&self, key: &str) -> Option<&String> {
        self.env.get(key)
    }

    fn list_env(&self) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
struct WorkflowJob {
    config: WorkflowYaml,
    context: Arc<RwLock<Context>>,
    namespace: Namespace,
}

impl WorkflowJob {
    fn new(config: WorkflowYaml, namespace: Namespace) -> Self {
        Self {
            config,
            context: Arc::new(RwLock::new(Context::new())),
            namespace,
        }
    }

    fn set_env(&self, key: String, value: String) {
        let mut context = self.context.write();
        context.set_env(key, value);
    }

    fn get_env(&self, key: &str) -> Option<String> {
        let context = self.context.read();
        context.get_env(key).cloned()
    }

    fn list_env(&self) -> Vec<(String, String)> {
        let context = self.context.read();
        context.list_env()
    }
}

impl JobTrait for WorkflowJob {
    fn cron(&self) -> &str {
        &self.config.on.schedule[0].cron
    }

    #[instrument(skip(self), fields(workflow_name = %self.config.name))]
    fn run(&mut self) -> Result<(), String> {
        info!("Running workflow");

        let working_dir = self.namespace.path.clone();
        info!(working_dir = %working_dir.display(), "Setting working directory");

        match &self.config.env {
            Some(env) => {
                for (key, value) in env {
                    info!(key = %key, value = %value, "Setting environment variable");
                    self.set_env(key.clone(), value.clone());
                }
            }
            None => {}
        }

        for (job_name, job) in &self.config.jobs.job_map {
            info!(job_name = %job_name, "Starting job");

            // evaluate the job condition
            if let Some(condition) = &job.condition {
                if !self.evaluate_condition(condition) {
                    info!(job_name = %job_name, condition = %condition, "Job condition not met, skipping");
                    continue;
                }
            }

            match &job.env {
                Some(env) => {
                    for (key, value) in env {
                        self.set_env(key.clone(), value.clone());
                    }
                }
                None => {}
            }

            // TODO: improve things optional fields on steps to enable more complex workflows
            for (index, step) in job.steps.iter().enumerate() {
                if let Some(run) = &step.run {
                    info!(step = index, command = run, "Executing step");

                    // if valid filepath read the contents as a string
                    let run = if std::path::Path::new(run).exists() {
                        std::fs::read_to_string(run).unwrap()
                    } else {
                        run.clone()
                    };

                    match &job.runs_on {
                        RunOnBackend::Bash => {
                            self.run_bash_external(&run, working_dir.clone());
                        }
                        RunOnBackend::PythonExternal(binary_path) => {
                            self.run_python_external(binary_path, &run, working_dir.clone());
                        }
                    }
                }

                if let Some(name) = &step.name {
                    debug!(step = index, name = name, "Step info");
                }

                if let Some(uses) = &step.uses {
                    info!(step = index, uses = uses, "Using action");
                    // handle the TODO:'uses' directive
                }
            }

            info!(job_name = %job_name, "Job completed");
        }

        info!("Workflow completed");
        Ok(())
    }

    fn clone_box(&self) -> Box<dyn JobTrait> {
        Box::new(self.clone())
    }

    fn downcast_ref_to_workflow_job(&self) -> Option<&WorkflowJob> {
        Some(self)
    }
}

impl WorkflowJob {
    fn evaluate_condition(&self, condition: &str) -> bool {
        // For simplicity, we'll just check if an environment variable is set
        // You can expand this to handle more complex conditions
        let parts: Vec<&str> = condition.split('=').collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let value = parts[1].trim().trim_matches('"');
            self.get_env(key).map_or(false, |v| v == value)
        } else {
            false
        }
    }

    fn run_bash_external(&self, command: &str, working_dir: PathBuf) {
        info!("Executing bash command: {}", command);
        let context = self.context.read();

        let output = Command::new("/bin/bash")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .env_clear() // clear environment vars
            .envs(&context.env)
            // .uid(65534) // use 'nobody' user ID for safety
            // .gid(65534) // use 'nobody' group ID for safety
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    info!("Command executed successfully");
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                } else {
                    error!("Command failed with exit code: {:?}", output.status.code());
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => {
                error!("Failed to execute command: {}", e);
            }
        }
    }

    fn run_python_external(
        &self,
        binary_path: &Option<String>,
        script: &str,
        working_dir: PathBuf,
    ) {
        let binding = "python".to_string();
        let python_binary = binary_path.as_ref().unwrap_or(&binding);
        let context = self.context.read();

        info!("Executing python script: {}", script);
        // TODO: improve working directory handling
        let directory = "."; // Set root as working directory
        let output = std::process::Command::new(python_binary)
            .arg("-c")
            .arg(script)
            .current_dir(working_dir)
            .env_clear() // clear environment vars
            .current_dir(directory)
            .envs(&context.env)
            // .uid(65534) // use 'nobody' user ID for safety
            // .gid(65534) // use 'nobody' group ID for safety
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    info!("Script executed successfully");
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                } else {
                    error!("Script failed with exit code: {:?}", output.status.code());
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => {
                error!("Failed to execute script: {}", e);
            }
        }
    }
}

impl Clone for Box<dyn JobTrait> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

type ScheduledJobs = Arc<RwLock<HashMap<String, Vec<Box<dyn JobTrait>>>>>;

#[derive(Clone)]
struct Scheduler {
    jobs: ScheduledJobs,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn add(&self, namespace: &str, job: Box<dyn JobTrait>) {
        let mut jobs = self.jobs.write();
        let namespace_jobs = jobs.entry(namespace.to_string()).or_default();
        let job_name = job
            .clone()
            .downcast_ref_to_workflow_job()
            .unwrap()
            .config
            .name
            .clone();
        let job_index = namespace_jobs
            .iter()
            .position(|j| j.downcast_ref_to_workflow_job().unwrap().config.name == job_name);

        if let Some(index) = job_index {
            namespace_jobs[index] = job.clone();
        } else {
            namespace_jobs.push(job.clone());
        }

        info!(namespace = %namespace, job = %job.downcast_ref_to_workflow_job().unwrap().config.name, "Adding job");
    }

    fn remove(&self, namespace: &str, name: &str) {
        let mut jobs = self.jobs.write();
        if let Some(namespace_jobs) = jobs.get_mut(namespace) {
            namespace_jobs.retain(|job| {
                if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                    workflow_job.config.name != name
                } else {
                    true
                }
            });
        }
        info!(namespace = %namespace, remaining_jobs = jobs.get(namespace).map_or(0, std::vec::Vec::len), "Remaining jobs");
    }

    fn run(&self) {
        let mut jobs = self.jobs.write();
        for (namespace, namespace_jobs) in jobs.iter_mut() {
            debug!(namespace = %namespace, starting_jobs = namespace_jobs.len(), "Starting to check jobs");
            namespace_jobs.retain_mut(|job| {
            if let Some(workflow_job) = job.clone().downcast_ref_to_workflow_job() {
                let s = workflow_job.cron();
                let s = if s == "now" {
                    let now = Local::now();
                    let in_near_future = now + chrono::Duration::seconds(1);
                    time_to_cron(in_near_future)
                } else {
                    s.to_string()
                };

                match CronSchedule::from_str(&s) {
                    Ok(schedule) => {
                        let now = Local::now();
                        let should_run = schedule.upcoming(Local).next().map_or(false, |t| {
                            let time_diff = t - now;
                            let num_seconds = time_diff.num_seconds();
                            debug!(job = %workflow_job.config.name, seconds = num_seconds, "Seconds until next job");
                            num_seconds <= 0
                        });
                        if should_run {
                            match job.run() {
                                Ok(()) => {
                                    info!(job = %workflow_job.config.name, "Job ran successfully");
                                }
                                Err(e) => {
                                    error!(job = %workflow_job.config.name, error = %e, "Job failed");
                                }
                        };
                    }
                    true
                    }
                    Err(e) => {
                        error!(job = %workflow_job.config.name, error = %e, "Invalid cron, removing job");
                        false
                    }
                }
            } else {
                warn!("Non-workflow job, skipping");
                true
            }
            // TODO: handle special case for "now", and remove if it was run
        });
            debug!(namespace = %namespace, remaining_jobs = namespace_jobs.len(), "Finished checking jobs");
        }
    }
}

fn run_forever(scheduler: Arc<Scheduler>) {
    tokio::spawn(async move {
        loop {
            scheduler.run();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn initialize() -> std::io::Result<(Arc<Mutex<Database>>, Arc<Scheduler>)> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting workflow engine");

    let database_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".jikan")
        .join("data");

    info!(database = %database_dir.display(), "Using database directory");
    std::fs::create_dir_all(&database_dir)?;

    let database_path = database_dir.join("jikan.redb");
    let db = Arc::new(Mutex::new(Database::create(database_path).map_err(
        |e| {
            error!(error = %e, "Failed to create database");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        },
    )?));

    let scheduler: Arc<Scheduler> = Arc::new(Scheduler::new());

    match hydrate_scheduler(&db, scheduler.clone()) {
        Ok(()) => {}
        Err(e) => {
            warn!(error = %e, "Failed to hydrate scheduler");
        }
    }

    Ok((db, scheduler))
}

fn daemonize() -> std::io::Result<()> {
    let pid_file = "/tmp/jikan.pid";
    let working_dir = "/tmp";

    std::fs::create_dir_all(working_dir)?;

    let stdout = File::create("/tmp/jikan.out")?;
    let stderr = File::create("/tmp/jikan.err")?;

    // Attempt to set permissions, but don't fail if it doesn't work
    let _ = std::fs::set_permissions(pid_file, std::fs::Permissions::from_mode(0o644));

    let daemonize = Daemonize::new()
        .pid_file(pid_file)
        .working_directory(working_dir)
        .stdout(stdout)
        .stderr(stderr);

    match daemonize.start() {
        Ok(()) => {
            info!("Successfully daemonized");
            Ok(())
        }
        Err(e) => {
            error!(error = %e, "Error daemonizing process");
            Err(std::io::Error::new(std::io::ErrorKind::Other, e))
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let run_in_background = args.get(1).map_or(false, |arg| arg == "daemon");

    let should_stop = args.get(1).map_or(false, |arg| arg == "stop");

    if should_stop {
        let pid_file = "/tmp/jikan.pid";
        let pid = std::fs::read_to_string(pid_file).map_err(|_e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to read PID file. Is the daemon running?",
            )
        })?;
        let pid = pid.trim().parse::<i32>().map_err(|_e| {
            std::io::Error::new(std::io::ErrorKind::Other, "Failed to parse PID file")
        })?;
        unsafe {
            // libc::kill(pid, libc::SIGTERM);
            // gracefully stop the daemon
            libc::kill(pid, libc::SIGINT);
        }
        // cleanup the pid file
        std::fs::remove_file(pid_file)?;
        return Ok(());
    }

    let (db, scheduler) = initialize()?;

    if run_in_background {
        daemonize()?;
    }

    // Create and start the Tokio runtime
    let runtime = Runtime::new()?;
    // we defer to the tokio runtime to the bottom of the main function rather
    // then with a macro to ensure that the daemonize function is called first
    runtime.block_on(async { start_server_and_scheduler(db, scheduler).await })
}

#[instrument(skip(db, scheduler))]
fn hydrate_scheduler(db: &Arc<Mutex<Database>>, scheduler: Arc<Scheduler>) -> Result<()> {
    let db_guard = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Failed to lock database: {}", e))?;
    let read_txn = db_guard
        .begin_read()
        .context("Failed to begin read transaction")?;

    let table = match read_txn.open_table(WORKFLOWS) {
        Ok(table) => table,
        Err(e) if e.to_string().contains("Table not found") => {
            warn!("Workflows table not found. Starting with an empty scheduler.");
            info!("Finished hydrating scheduler (0 workflows)");
            return Ok(());
        }
        Err(e) => return Err(e).context("Failed to open workflows table"),
    };

    let namespace_table = match read_txn.open_table(NAMESPACES) {
        Ok(table) => table,
        Err(e) if e.to_string().contains("Table not found") => {
            warn!("Namespaces table not found. Starting with an empty scheduler.");
            info!("Finished hydrating scheduler (0 workflows)");
            return Ok(());
        }
        Err(e) => return Err(e).context("Failed to open namespaces table"),
    };

    let mut hydrated_jobs = 0;

    for result in table.iter().context("Failed to iterate over workflows")? {
        let (key, yaml_content) = result.context("Failed to read workflow entry")?;
        let (namespace, name) = key.value();

        let namespace_entry = namespace_table
            .get(namespace)
            .with_context(|| format!("Failed to get namespace: {namespace}"))?
            .ok_or_else(|| anyhow::anyhow!("Namespace not found: {namespace}"))?;

        let namespace_obj: Namespace = serde_json::from_str(namespace_entry.value())
            .with_context(|| format!("Failed to parse namespace: {namespace}"))?;

        match serde_yaml::from_str(yaml_content.value()) {
            Ok(config) => {
                let workflow = WorkflowJob::new(config, namespace_obj);
                scheduler.add(namespace, Box::new(workflow));
                hydrated_jobs += 1;
                info!(workflow = name, "Hydrated workflow");
            }
            Err(e) => {
                error!(error = %e, workflow = name, "Failed to parse workflow config");
            }
        }
    }

    info!(
        hydrated_jobs = hydrated_jobs,
        "Finished hydrating scheduler"
    );
    Ok(())
}

#[instrument(skip(db, scheduler))]
async fn start_server_and_scheduler(
    db: Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<()> {
    // start the scheduler
    let scheduler_clone = Arc::clone(&scheduler);
    tokio::spawn(async move {
        run_forever(scheduler_clone);
    });

    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(addr.clone())?;
    info!(address = %addr, "Server started");

    {
        // ensure the NAMEPACES table and WORKFLOWS table exist (write dummy data; key, value)
        let db = db.lock().unwrap();
        let write_txn = db
            .begin_write()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        {
            write_txn
                .open_table(NAMESPACES)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            write_txn
                .open_table(WORKFLOWS)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }

        write_txn
            .commit()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }

    loop {
        let (stream, addr) = listener.accept()?;
        info!(client_addr = %addr, "New client connected");
        let db = Arc::clone(&db);
        let scheduler = Arc::clone(&scheduler);
        tokio::spawn(async move {
            info!(client_addr = %addr, "Handling client");
            if let Err(e) = handle_client(stream, db, scheduler).await {
                error!(client_addr = %addr, error = %e, "Error handling client");
            }
        });
    }
}

#[instrument(skip(db, scheduler))]
async fn delete_namespace(
    name: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    {
        let mut table = write_txn
            .open_table(NAMESPACES)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        table.remove(name).map_err(|e| {
            error!(error = %e, namespace = name, "Failed to remove namespace");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
    }
    write_txn
        .commit()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let mut scheduler = scheduler.jobs.write();
    scheduler.remove(name);

    Ok(format!("Namespace '{name}' deleted successfully.\n"))
}

#[instrument(skip(db))]
async fn add_namespace(
    name: &str,
    path: &str,
    db: &Arc<Mutex<Database>>,
) -> std::io::Result<String> {
    if path.starts_with('.') {
        return Ok("Invalid directory path. Please provide an absolute path.".to_string());
    }

    let namespace = Namespace {
        name: name.to_string(),
        path: PathBuf::from(path),
    };

    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    {
        let mut table = write_txn
            .open_table(NAMESPACES)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        table
            .insert(name, serde_json::to_string(&namespace).unwrap().as_str())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    write_txn
        .commit()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    Ok(format!("Namespace '{name}' added successfully.\n"))
}

#[instrument(skip(db))]
async fn list_namespaces(db: &Arc<Mutex<Database>>) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(NAMESPACES).map_err(|e| {
        error!(error = %e, "Failed to open namespaces table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    let mut response = String::from("Namespaces:\n");
    for result in table.iter().map_err(|e| {
        error!(error = %e, "Failed to iterate over namespaces");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        let (name, data) = result.map_err(|e| {
            error!(error = %e, "Failed to read namespace entry");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        let namespace: Namespace = serde_json::from_str(data.value()).unwrap();

        response.push_str(&format!(
            "- {} (path: {})\n",
            name.value(),
            namespace.path.display()
        ));
    }

    info!("Namespaces listed successfully");
    Ok(response)
}

#[instrument(skip(db, scheduler))]
async fn add_workflow(
    namespace: &str,
    name: &str,
    yaml_content: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let config: WorkflowYaml = serde_yaml::from_str(yaml_content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    {
        let mut table = write_txn
            .open_table(WORKFLOWS)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        table
            .insert((namespace, name), yaml_content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    write_txn
        .commit()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    let namespace_table = read_txn
        .open_table(NAMESPACES)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let namespace_entry = namespace_table
        .get(namespace)
        .map_err(|e| {
            error!(error = %e, namespace = namespace, "Failed to get namespace");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?
        .ok_or_else(|| {
            error!(namespace = namespace, "Namespace not found");
            std::io::Error::new(std::io::ErrorKind::NotFound, "Namespace not found")
        })?;

    let namespace_obj: Namespace = serde_json::from_str(namespace_entry.value()).map_err(|e| {
        error!(error = %e, namespace = namespace, "Failed to parse namespace");
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;

    scheduler.add(namespace, Box::new(WorkflowJob::new(config, namespace_obj)));

    Ok(format!(
        "Workflow '{name}' added successfully to namespace '{namespace}'.\n",
    ))
}

#[instrument(skip(db, scheduler))]
async fn register_workflows_from_dir(
    namespace: &str,
    dir_path: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let mut registered = 0;

    let namespace_path = PathBuf::from(dir_path);
    let workspace_path = namespace_path.join(".jikan").join("workflows");

    println!(
        "Registering workflows from directory: {}",
        workspace_path.display()
    );

    if workspace_path.is_relative() {
        return Ok("Invalid directory path. Please provide an absolute path.".to_string());
    }

    {
        // write the namespace to the database
        let db = db.lock().unwrap();
        let write_txn = db
            .begin_write()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        {
            let mut table = write_txn
                .open_table(NAMESPACES)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let new_namespace = Namespace {
                name: namespace.to_string(),
                path: namespace_path.clone(),
            };
            table
                .insert(
                    namespace,
                    serde_json::to_string(&new_namespace).unwrap().as_str(),
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }
        write_txn
            .commit()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }

    for entry in WalkDir::new(workspace_path.clone())
        .follow_links(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .map_or(false, |ext| ext == "yaml" || ext == "yml")
        {
            let file_name = entry.file_name().to_string_lossy();
            let workflow_name = file_name.trim_end_matches(".yaml").trim_end_matches(".yml");
            let yaml_content = tokio::fs::read_to_string(entry.path()).await?;

            if let Ok(_config) = serde_yaml::from_str::<WorkflowYaml>(&yaml_content) {
                add_workflow(
                    namespace,
                    workflow_name,
                    &yaml_content,
                    db,
                    Arc::clone(&scheduler),
                )
                .await?;
                registered += 1;
            } else {
                warn!(file = %entry.path().display(), "Failed to parse workflow config");
            }
        }
    }

    Ok(format!(
        "Registered {registered} workflows in namespace '{namespace}'.\n",
    ))
}

#[instrument(skip(db, scheduler))]
async fn run_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(WORKFLOWS).map_err(|e| {
        error!(error = %e, "Failed to open workflows table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    if let Some(_workflow) = table.get((namespace, name)).map_err(|e| {
        error!(error = %e, namespace = namespace, workflow = name, "Failed to get workflow");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        // find the job in the scheduler
        let jobs = scheduler.jobs.read();
        let job = jobs
            .get(namespace)
            .and_then(|namespace_jobs| {
                namespace_jobs.iter().find(|j| {
                    j.downcast_ref_to_workflow_job()
                        .map_or(false, |wj| wj.config.name == name)
                })
            })
            .cloned();

        if job.is_none() {
            warn!(
                namespace = namespace,
                workflow = name,
                "Workflow not found in scheduler"
            );
            return Ok(format!(
                "Workflow '{name}' not found in namespace '{namespace}'.\n",
            ));
        }

        let mut job = job.unwrap();

        match job.run() {
            Ok(()) => {
                info!(
                    namespace = namespace,
                    workflow = name,
                    "Workflow ran successfully"
                );
                Ok(format!(
                    "Workflow '{name}' in namespace '{namespace}' ran successfully.\n",
                ))
            }
            Err(e) => {
                error!(namespace = namespace, workflow = name, error = %e, "Workflow failed to run");
                Ok(format!(
                    "Workflow '{name}' in namespace '{namespace}' failed to run: {e}\n"
                ))
            }
        }
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Ok(format!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n",
        ))
    }
}

#[instrument(skip(db))]
async fn list_workflows(
    namespace: Option<&str>,
    db: &Arc<Mutex<Database>>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(WORKFLOWS).map_err(|e| {
        error!(error = %e, "Failed to open workflows table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    let mut response = String::from("Workflows:\n");
    for result in table.iter().map_err(|e| {
        error!(error = %e, "Failed to iterate over workflows");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        let (key, _value) = result.map_err(|e| {
            error!(error = %e, "Failed to read workflow entry");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;

        let (ns, name) = key.value();

        if namespace.is_none() || namespace == Some(ns) {
            response.push_str(&format!("- {name} (namespace: {ns})\n"));
        }
    }

    info!("Workflows listed successfully");
    Ok(response)
}

#[instrument(skip(db))]
async fn get_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(WORKFLOWS).map_err(|e| {
        error!(error = %e, "Failed to open workflows table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    if let Some(workflow) = table.get((namespace, name)).map_err(|e| {
        error!(error = %e, namespace = namespace, workflow = name, "Failed to get workflow");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        info!(
            namespace = namespace,
            workflow = name,
            "Workflow retrieved successfully"
        );
        Ok(format!(
            "Workflow '{name}' in namespace '{namespace}':\n{}",
            workflow.value()
        ))
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Ok(format!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n"
        ))
    }
}

#[instrument(skip(db, scheduler))]
async fn delete_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let write_txn = db.begin_write().map_err(|e| {
        error!(error = %e, "Failed to begin write transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    {
        let mut table = write_txn.open_table(WORKFLOWS).map_err(|e| {
            error!(error = %e, "Failed to open workflows table");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        table.remove((namespace, name)).map_err(|e| {
            error!(error = %e, namespace = namespace, workflow = name, "Failed to remove workflow");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
    }
    write_txn.commit().map_err(|e| {
        error!(error = %e, "Failed to commit transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    scheduler.remove(namespace, name);

    info!(
        namespace = namespace,
        workflow = name,
        "Workflow deleted successfully"
    );
    Ok(format!(
        "Workflow '{name}' deleted successfully from namespace '{namespace}'.\n"
    ))
}

#[instrument]
async fn get_next_run_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(WORKFLOWS).map_err(|e| {
        error!(error = %e, "Failed to open workflows table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    let namespace_table = read_txn.open_table(NAMESPACES).map_err(|e| {
        error!(error = %e, "Failed to open namespaces table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    if let Some(workflow) = table.get((namespace, name)).map_err(|e| {
        error!(error = %e, namespace = namespace, workflow = name, "Failed to get workflow");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        let config: WorkflowYaml = serde_yaml::from_str(workflow.value())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let namespace_entry = namespace_table
            .get(namespace)
            .map_err(|e| {
                error!(error = %e, namespace = namespace, "Failed to get namespace");
                std::io::Error::new(std::io::ErrorKind::Other, e)
            })?
            .ok_or_else(|| {
                error!(namespace = namespace, "Namespace not found");
                std::io::Error::new(std::io::ErrorKind::NotFound, "Namespace not found")
            })?;

        let namespace_obj: Namespace =
            serde_json::from_str(namespace_entry.value()).map_err(|e| {
                error!(error = %e, namespace = namespace, "Failed to parse namespace");
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;

        let job = WorkflowJob::new(config, namespace_obj);
        let cron = job.cron();
        let s = if cron == "now" {
            let now = Local::now();
            let in_near_future = now + chrono::Duration::seconds(1);
            time_to_cron(in_near_future)
        } else {
            cron.to_string()
        };

        match CronSchedule::from_str(&s) {
            Ok(schedule) => {
                let now = Local::now();
                let (next_time, next_run) =
                    schedule
                        .upcoming(Local)
                        .next()
                        .map_or((now, "never".to_string()), |t| {
                            let time_diff = t - now;
                            let num_seconds = time_diff.num_seconds();
                            (t, format!("{num_seconds} seconds"))
                        });

                info!(
                    namespace = namespace,
                    workflow = name,
                    next_run = &next_run,
                    "Next run calculated"
                );
                Ok(format!(
                    "Workflow '{name}' in namespace '{namespace}' will run in {next_run}.\nAbsolute time: {}\n",
                    next_time.to_rfc2822()
                ))
            }
            Err(e) => {
                error!(error = %e, namespace = namespace, workflow = name, "Invalid cron expression");
                Ok(format!(
                    "Workflow '{name}' in namespace '{namespace}' has an invalid cron expression.\n"
                ))
            }
        }
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Ok(format!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n",
        ))
    }
}

fn set_env(
    namespace: &str,
    name: &str,
    key: &str,
    value: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Scheduler>,
) -> String {
    let jobs = scheduler.jobs.read();
    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                workflow_job.set_env(key.to_string(), value.to_string());
                format!(
                    "Environment variable '{key}' set for workflow '{name}' in namespace '{namespace}'.\n",
                )
            } else {
                format!("Workflow '{name}' in namespace '{namespace}' is not a WorkflowJob.\n",)
            }
        } else {
            format!("Workflow '{name}' not found in namespace '{namespace}'.\n",)
        }
    } else {
        format!("Namespace '{namespace}' not found.\n",)
    }
}

fn get_env(
    namespace: &str,
    name: &str,
    key: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Scheduler>,
) -> String {
    let jobs = scheduler.jobs.read();
    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                if let Some(value) = workflow_job.get_env(key) {
                    format!("{key}={value}\n")
                } else {
                    format!("Environment variable '{key}' not found for workflow '{name}' in namespace '{namespace}'.\n")
                }
            } else {
                format!("Workflow '{name}' in namespace '{namespace}' is not a WorkflowJob.\n")
            }
        } else {
            format!("Workflow '{name}' not found in namespace '{namespace}'.\n")
        }
    } else {
        format!("Namespace '{namespace}' not found.\n")
    }
}

fn list_env(
    namespace: &str,
    name: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Scheduler>,
) -> String {
    let jobs = scheduler.jobs.read();
    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                let env_vars = workflow_job.list_env();
                let mut response = format!(
                    "Environment variables for workflow '{name}' in namespace '{namespace}':\n"
                );
                for (key, value) in env_vars {
                    response.push_str(&format!("{key}={value}\n"));
                }
                response
            } else {
                format!("Workflow '{name}' in namespace '{namespace}' is not a W©orkflowJob.\n")
            }
        } else {
            format!("Workflow '{name}' not found in namespace '{namespace}'.\n")
        }
    } else {
        format!("Namespace '{namespace}' not found.\n")
    }
}

#[instrument(skip(stream, db, scheduler), fields(client_addr = %stream.peer_addr().unwrap()))]
async fn handle_client(
    stream: TcpStream,
    db: Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<()> {
    let mut stream = TokioTcpStream::from_std(stream)?;
    let (reader, mut writer) = stream.split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    debug!(command = %line.trim(), "Received command");

    let response = match line.split_whitespace().collect::<Vec<&str>>().as_slice() {
        ["ADD_NAMESPACE", name, path] => add_namespace(name, path, &db).await?,
        ["DELETE_NAMESPACE", name] => delete_namespace(name, &db, scheduler).await?,
        ["LIST_NAMESPACES"] => list_namespaces(&db).await?,
        ["REGISTER_DIR", namespace, dir_path] => {
            register_workflows_from_dir(namespace, dir_path, &db, scheduler).await?
        }
        ["ADD", namespace, name, body] => {
            add_workflow(namespace, name, body, &db, scheduler).await?
        }
        ["LIST", namespace] => list_workflows(Some(namespace), &db).await?,
        ["GET", namespace, name] => get_workflow(namespace, name, &db).await?,
        ["NEXT", namespace, name] => get_next_run_workflow(namespace, name, &db).await?,
        ["DELETE", namespace, name] => delete_workflow(namespace, name, &db, scheduler).await?,
        ["RUN", namespace, name] => run_workflow(namespace, name, &db, scheduler).await?,
        ["SET_ENV", namespace, name, key, value] => {
            set_env(namespace, name, key, value, &db, &scheduler)
        }
        ["GET_ENV", namespace, name, key] => get_env(namespace, name, key, &db, &scheduler),
        ["LIST_ENV", namespace, name] => list_env(namespace, name, &db, &scheduler),
        _ => {
            warn!(command = %line.trim(), "Invalid command received");
            "Invalid command. Use ADD_NAMESPACE, LIST_NAMESPACES, REGISTER_DIR, ADD, LIST, GET, or DELETE.\n".to_string()
        }
    };

    writer.write_all(response.as_bytes()).await?;
    writer.flush().await?;

    info!("Response sent to client");

    Ok(())
}

fn time_to_cron(time: DateTime<Local>) -> String {
    format!(
        "{sec} {min} {hour} {dom} {mon} {dow} {year}",
        sec = time.second(),
        min = time.minute(),
        hour = time.hour(),
        dom = time.day(),
        mon = time.month(),
        dow = "*",  // dow = time.weekday().number_from_monday(),
        year = "*"  // year = time.year()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[tokio::test]
    async fn test_string_to_cron() {
        // let expression = "0   30   9,12,15     1,15       May-Aug  Mon,Wed,Fri  2018/2";
        let expression = "0 0 1 1 * * *";
        let x = CronSchedule::from_str(expression);
        assert!(x.is_ok());
    }

    #[tokio::test]
    async fn test_time_to_cron() {
        let time = Local.with_ymd_and_hms(2021, 5, 1, 0, 0, 0).unwrap();
        let cron = time_to_cron(time);
        assert_eq!(cron, "0 0 0 1 5 * *");
    }
}
