use chrono::DateTime;
use chrono::Datelike;
use chrono::Local;
use chrono::Timelike;
use cron::Schedule as CronSchedule;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpStream as TokioTcpStream;
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use rustpython_vm as vm;

const WORKFLOWS: TableDefinition<&str, &str> = TableDefinition::new("workflows");

#[derive(Debug, Deserialize, Serialize, Clone)]
struct JikanConfig {
    name: String,
    #[serde(rename = "run-name")]
    run_name: String,
    on: On,
    jobs: Jobs,
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
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
enum RunOnBackend {
    PythonInternal,
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

#[derive(Clone, Debug)]
struct WorkflowJob {
    config: JikanConfig,
}

impl WorkflowJob {
    fn new(config: JikanConfig) -> Self {
        Self { config }
    }
}

impl JobTrait for WorkflowJob {
    fn cron(&self) -> &str {
        &self.config.on.schedule[0].cron
    }

    #[instrument(skip(self), fields(workflow_name = %self.config.name))]
    fn run(&mut self) -> Result<(), String> {
        info!("Running workflow");

        for (job_name, job) in &self.config.jobs.job_map {
            info!(job_name = %job_name, "Starting job");

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
                        RunOnBackend::PythonInternal => {
                            self.run_python_internal(&run);
                        }
                        RunOnBackend::Bash => {
                            self.run_bash_external(&run);
                        }
                        RunOnBackend::PythonExternal(binary_path) => {
                            self.run_python_external(binary_path, &run);
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
    fn run_python_internal(&self, code: &str) {
        // TODO: improve embedded Python VM (maybe move behind a feature flag)
        vm::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            match vm
                .compile(code, vm::compiler::Mode::Exec, "<embedded>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err, Some(code)))
            {
                Ok(code_obj) => match vm.run_code_obj(code_obj, scope) {
                    Ok(_) => {}
                    Err(err) => {
                        error!("{:?}", err);
                    }
                },
                Err(err) => {
                    error!("{:?}", err);
                }
            }
        });
    }

    fn run_bash_external(&self, command: &str) {
        info!("Executing bash command: {}", command);
        let output = Command::new("/bin/bash")
            .arg("-c")
            .arg(command)
            .env_clear() // clear environment vars
            .current_dir("/")
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

    fn run_python_external(&self, binary_path: &Option<String>, script: &str) {
        let binding = "python".to_string();
        let python_binary = binary_path.as_ref().unwrap_or(&binding);
        info!("Executing python script: {}", script);
        // TODO: improve working directory handling
        let directory = "."; // Set root as working directory
        let output = std::process::Command::new(python_binary)
            .arg("-c")
            .arg(script)
            .env_clear() // clear environment vars
            .current_dir(directory)
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

use parking_lot::RwLock;

#[derive(Clone)]
struct Scheduler {
    jobs: Arc<RwLock<Vec<Box<dyn JobTrait>>>>,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn add(&self, job: Box<dyn JobTrait>) {
        let mut jobs = self.jobs.write();
        info!(job = %job.downcast_ref_to_workflow_job().unwrap().config.name, "Adding job");
        jobs.push(job);
    }

    fn remove(&self, name: &str) {
        let mut jobs = self.jobs.write();
        jobs.retain(|job| {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                workflow_job.config.name != name
            } else {
                true
            }
        });
        // show the remaining jobs
        info!(remaining_jobs = jobs.len(), "Remaining jobs");
    }

    #[instrument(skip(self))]
    fn run(&self) {
        let mut jobs = self.jobs.write();
        debug!(starting_jobs = jobs.len(), "Starting to check jobs");

        jobs.retain_mut(|job| {
            if let Some(workflow_job) = job.clone().downcast_ref_to_workflow_job() {
                let _s = workflow_job.cron();

                let s = if _s == "now" {
                    let now = Local::now();
                    let in_near_future = now + chrono::Duration::seconds(1);
                    time_to_cron(in_near_future)
                } else {
                    _s.to_string()
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
                                Ok(_) => {
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

        debug!(remaining_jobs = jobs.len(), "Finished checking jobs");
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

#[tokio::main]
#[instrument]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Jikan application");

    let db = Arc::new(Mutex::new(Database::create("jikan.redb").map_err(|e| {
        error!(error = %e, "Failed to create database");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?));

    let scheduler: Arc<Scheduler> = Arc::new(Scheduler::new());

    match hydrate_scheduler(&db, scheduler.clone()) {
        Ok(_) => {}
        Err(e) => {
            warn!(error = %e, "Failed to hydrate scheduler")
        }
    }

    start_server_and_scheduler(db, scheduler).await
}

#[instrument(skip(db, scheduler))]
fn hydrate_scheduler(db: &Arc<Mutex<Database>>, scheduler: Arc<Scheduler>) -> std::io::Result<()> {
    let db_guard = db.lock().unwrap();
    let read_txn = db_guard.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    // attempt to open the table, but don't fail if it doesn't exist
    let table = match read_txn.open_table(WORKFLOWS) {
        Ok(table) => table,
        Err(e) => {
            if e.to_string().contains("Table not found") {
                warn!("Workflows table not found. Starting with an empty scheduler.");
                info!("Finished hydrating scheduler (0 workflows)");
                return Ok(());
            } else {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        }
    };

    let mut hydrated_jobs = 0;
    for result in table.iter().map_err(|e| {
        error!(error = %e, "Failed to iterate over workflows");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        let (name, yaml_content) = result.map_err(|e| {
            error!(error = %e, "Failed to read workflow entry");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        match serde_yaml::from_str(yaml_content.value()) {
            Ok(config) => {
                let workflow = WorkflowJob::new(config);
                scheduler.add(Box::new(workflow));
                hydrated_jobs += 1;
                info!(workflow = name.value(), "Hydrated workflow");
            }
            Err(e) => {
                error!(error = %e, workflow = name.value(), "Failed to parse workflow config");
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

    // start TCP server
    let listener = TcpListener::bind("127.0.0.1:8080")?;
    info!("Server listening on port 8080");

    loop {
        let (stream, addr) = listener.accept()?;
        info!(client_addr = %addr, "New client connected");
        let db = Arc::clone(&db);
        let scheduler = Arc::clone(&scheduler);
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, db, scheduler).await {
                error!(client_addr = %addr, error = %e, "Error handling client");
            }
        });
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
        ["ADD", name] => add_workflow(name, &db, scheduler).await?,
        ["LIST"] => list_workflows(&db).await?,
        ["GET", name] => get_workflow(name, &db).await?,
        ["DELETE", name] => delete_workflow(name, &db, scheduler).await?,
        _ => {
            warn!(command = %line.trim(), "Invalid command received");

            "Invalid command. Use ADD, LIST, GET, or DELETE.\n".to_string()
        }
    };

    writer.write_all(response.as_bytes()).await?;
    writer.flush().await?;

    info!("Response sent to client");

    Ok(())
}

#[instrument(skip(db, scheduler))]
async fn add_workflow(
    name: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Scheduler>,
) -> std::io::Result<String> {
    let yaml_content = tokio::fs::read_to_string(format!("workflows/{}.yaml", name)).await?;
    let config: JikanConfig = serde_yaml::from_str(&yaml_content)
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
            .insert(name, yaml_content.as_str())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    write_txn
        .commit()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    scheduler.add(Box::new(WorkflowJob::new(config)));

    Ok(format!("Workflow '{}' added successfully.\n", name))
}

#[instrument(skip(db))]
async fn list_workflows(db: &Arc<Mutex<Database>>) -> std::io::Result<String> {
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
        let (name, _) = result.map_err(|e| {
            error!(error = %e, "Failed to read workflow entry");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        response.push_str(&format!("- {}\n", name.value()));
    }

    info!("Workflows listed successfully");
    Ok(response)
}

#[instrument(skip(db))]
async fn get_workflow(name: &str, db: &Arc<Mutex<Database>>) -> std::io::Result<String> {
    let db = db.lock().unwrap();
    let read_txn = db.begin_read().map_err(|e| {
        error!(error = %e, "Failed to begin read transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;
    let table = read_txn.open_table(WORKFLOWS).map_err(|e| {
        error!(error = %e, "Failed to open workflows table");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    if let Some(workflow) = table.get(name).map_err(|e| {
        error!(error = %e, workflow = name, "Failed to get workflow");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })? {
        info!(workflow = name, "Workflow retrieved successfully");
        Ok(format!("Workflow '{}':\n{}", name, workflow.value()))
    } else {
        warn!(workflow = name, "Workflow not found");
        Ok(format!("Workflow '{}' not found.\n", name))
    }
}

#[instrument(skip(db, scheduler))]
async fn delete_workflow(
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
        table.remove(name).map_err(|e| {
            error!(error = %e, workflow = name, "Failed to remove workflow");
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
    }
    write_txn.commit().map_err(|e| {
        error!(error = %e, "Failed to commit transaction");
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })?;

    // remove the job from the scheduler
    scheduler.remove(name);
    drop(scheduler);

    info!(workflow = name, "Workflow deleted successfully");
    Ok(format!("Workflow '{}' deleted successfully.\n", name))
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
