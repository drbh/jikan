use anyhow::{Context as _, Result};
use chrono::{DateTime, Datelike, Local, Timelike};
use cron::Schedule as CronSchedule;
use daemonize::Daemonize;
use dirs::home_dir;
use minijinja::{context, Environment};
use parking_lot::RwLock;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
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
pub struct JobStatus {
    pub namespace: String,
    pub name: String,
    pub status: String,
    pub message: String,
    pub timestamp: usize,
}

pub struct StatusBroadcaster {
    senders: Arc<Mutex<Vec<Sender<JobStatus>>>>,
}

impl Default for StatusBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBroadcaster {
    #[must_use]
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> Receiver<JobStatus> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut senders) = self.senders.lock() {
            senders.push(tx);
        }
        rx
    }

    pub fn send(&self, status: &JobStatus) -> Result<()> {
        if let Ok(senders) = self.senders.lock() {
            senders.iter().for_each(|sender| {
                if let Err(e) = sender.send(status.clone()) {
                    warn!("Failed to send status: {}", e);
                }
            });
        }
        Ok(())
    }
}

impl std::fmt::Debug for StatusBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusBroadcaster").finish()
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    broadcaster: Arc<StatusBroadcaster>,
    subscribers: Arc<RwLock<HashMap<String, Sender<JobStatus>>>>,
}

impl SubscriptionManager {
    #[must_use]
    pub fn new(broadcaster: Arc<StatusBroadcaster>) -> Self {
        Self {
            broadcaster,
            subscribers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_subscriber(&self, id: &str, workflows: Vec<String>) -> Receiver<JobStatus> {
        info!("Adding subscriber {}", id);
        let (tx, rx) = mpsc::channel();
        let mut subscribers = self.subscribers.write();
        subscribers.insert(id.to_string(), tx.clone());

        let broadcaster = self.broadcaster.clone();
        let subscribers_clone = self.subscribers.clone();
        let id_clone = id.to_string();

        std::thread::spawn(move || {
            let receiver = broadcaster.subscribe();
            while let Ok(status) = receiver.recv() {
                debug!("Received status for workflow: {}", status.name);
                if workflows.is_empty() || workflows.contains(&status.name) {
                    let subscribers = subscribers_clone.read();
                    if let Some(tx) = subscribers.get(&id_clone) {
                        if tx.send(status.clone()).is_err() {
                            warn!("Failed to send status to subscriber {}", id_clone);
                            break; // subscriber has disconnected
                        }
                    } else {
                        warn!("Subscriber {} not found", id_clone);
                        break; // subscriber has been removed
                    }
                }
            }
            // clean up the subscriber when the task ends
            let mut subscribers = subscribers_clone.write();
            subscribers.remove(&id_clone);
            info!("Subscriber {} removed", id_clone);
        });

        rx
    }

    pub fn remove_subscriber(&self, id: &str) {
        info!("Removing subscriber {}", id);
        let mut subscribers = self.subscribers.write();
        subscribers.remove(id);
    }

    pub fn send_event(&self, status: &JobStatus) -> Result<()> {
        match self.broadcaster.send(status) {
            Ok(()) => {
                debug!("Event sent successfully");
                Ok(())
            }
            Err(e) => {
                warn!("Failed to send event: {}", e);
                Err(anyhow::anyhow!("Failed to send event: {}", e))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunEventTypes {
    Schedule,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub event: RunEventTypes,
    pub data: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResults {
    pub success: bool,
    pub logfiles: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub struct DaemonInfo {
    started_at: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    name: String,
    path: PathBuf,
}

impl Namespace {
    fn new(name: String, path: PathBuf) -> Self {
        Self { name, path }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ActionYaml {
    name: String,
    description: String,
    inputs: HashMap<String, Input>,
    outputs: HashMap<String, Output>,
    runs: Runs,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Input {
    description: String,
    required: bool,
    default: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Output {
    description: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Runs {
    using: String,
    main: String,
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
    schedule: Option<Vec<Schedule>>,
    workflow_dispatch: Option<bool>,
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
    // NodeExternal(Option<String>),
    PythonExternal(Option<String>),
    Bash,
    Machine, // conceptually above the shell level
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Step {
    run: Option<String>,
    name: Option<String>,
    uses: Option<String>,
    with: Option<HashMap<String, String>>,
}

trait JobTrait: Send + Sync {
    fn crons(&self) -> Vec<String>;
    fn run(&mut self, event: RunEvent) -> Result<RunResults, String>;
    fn clone_box(&self) -> Box<dyn JobTrait>;
    fn downcast_ref_to_workflow_job(&self) -> Option<&WorkflowJob>;
}

// convert a JobTrait to a WorkflowJob with From and Into
impl From<Box<dyn JobTrait>> for WorkflowJob {
    fn from(job: Box<dyn JobTrait>) -> Self {
        job.downcast_ref_to_workflow_job().unwrap().clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Context {
    env: HashMap<String, String>,
    parameters: HashMap<String, String>,
}

impl Context {
    fn new() -> Self {
        Context {
            env: HashMap::new(),
            parameters: HashMap::new(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MachineConfig {
    operating_system: String,
    executable_path: String,
    username: String,
    run_binary_map: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct WorkflowJob {
    config: WorkflowYaml,
    machine_config: MachineConfig,
    context: Arc<RwLock<Context>>,
    namespace: Namespace,
    subscription_manager: Arc<SubscriptionManager>,
}

impl WorkflowJob {
    fn new(
        config: WorkflowYaml,
        machine_config: MachineConfig,
        namespace: Namespace,
        subscription_manager: Arc<SubscriptionManager>,
    ) -> Self {
        Self {
            config,
            machine_config,
            context: Arc::new(RwLock::new(Context::new())),
            namespace,
            subscription_manager,
        }
    }

    fn set_env<T>(&self, key: T, value: T)
    where
        T: Into<String>, // any type that can be converted to a string
    {
        let mut context = self.context.write();
        context.set_env(key.into(), value.into());
    }

    fn get_env<T>(&self, key: T) -> Option<String>
    where
        T: Into<String>,
    {
        let context = self.context.read();
        context.get_env(&key.into()).cloned()
    }

    fn list_env(&self) -> Vec<(String, String)> {
        let context = self.context.read();
        context.list_env()
    }
}

impl JobTrait for WorkflowJob {
    fn crons(&self) -> Vec<String> {
        self.config
            .on
            .schedule
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|s| s.cron.clone())
            .collect()
    }

    #[instrument(skip(self), fields(workflow_name = %self.config.name))]
    fn run(&mut self, event: RunEvent) -> Result<RunResults, String> {
        let job_id = Local::now().format("%Y%m%d%H%M%S").to_string();

        let mut logfiles = vec![];
        let mut success = false;
        info!("Running workflow");

        let job_status = JobStatus {
            namespace: self.namespace.name.clone(),
            name: self.config.name.clone(),
            status: "in_progress".to_string(),
            message: "Workflow started".to_string(),
            timestamp: usize::try_from(Local::now().timestamp()).unwrap_or(0),
        };

        if let Err(e) = self.subscription_manager.send_event(&job_status) {
            warn!("Failed to send start event: {}", e);
        }

        let working_dir = self.namespace.path.clone();
        info!(working_dir = %working_dir.display(), "Setting working directory");

        match &self.config.env {
            Some(env) => {
                for (key, value) in env {
                    info!(key = %key, value = %value, "Setting environment variable");
                    self.set_env(key, value);
                }
            }
            None => {}
        }

        for (job_name, job) in &self.config.jobs.job_map {
            info!(job_name = %job_name, "Starting job");

            let job_status = JobStatus {
                namespace: self.namespace.name.clone(),
                name: job_name.clone(),
                status: "in_progress".to_string(),
                message: "Job started".to_string(),
                timestamp: usize::try_from(Local::now().timestamp()).unwrap_or(0),
            };

            if let Err(e) = self.subscription_manager.send_event(&job_status) {
                warn!("Failed to send start event: {}", e);
            }

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
                        self.set_env(key, value);
                    }
                }
                None => {}
            }

            success = false;
            let mut job_logs = String::new();

            // TODO: improve things optional fields on steps to enable more complex workflows
            for (index, step) in job.steps.iter().enumerate() {
                if let Some(run) = &step.run {
                    info!(step = index, command = run, "Executing step");

                    let job_status = JobStatus {
                        namespace: self.namespace.name.clone(),
                        name: job_name.clone(),
                        status: "in_progress".to_string(),
                        message: format!("Step {index} started"),
                        timestamp: usize::try_from(Local::now().timestamp()).unwrap_or(0),
                    };

                    if let Err(e) = self.subscription_manager.send_event(&job_status) {
                        warn!("Failed to send start event: {}", e);
                    }

                    // if valid filepath read the contents as a string
                    let possible_path = working_dir.clone().join(run);

                    let run = if possible_path.exists() {
                        std::fs::read_to_string(possible_path).unwrap()
                    } else {
                        run.clone()
                    };

                    let (logs, succeeded) = match &job.runs_on {
                        RunOnBackend::Bash | RunOnBackend::Machine => {
                            self.run_bash_external(&run, working_dir.clone())
                        }
                        RunOnBackend::PythonExternal(binary_path) => {
                            self.run_python_external(binary_path, &run, working_dir.clone())
                        }
                    };
                    job_logs.push_str(&logs);
                    success = succeeded;
                }

                if let Some(name) = &step.name {
                    debug!(step = index, name = name, "Step info");
                }

                if let Some(uses) = &step.uses {
                    info!(step = index, uses = uses, "Using action");
                    // prefer the global action directory over the local one
                    let action_dir = home_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join(".jikan")
                        .join("actions")
                        .join(uses);

                    let action_yaml_path = action_dir.join("action.yaml");
                    let action_yaml = std::fs::read_to_string(action_yaml_path).unwrap();
                    let action_yaml: ActionYaml = serde_yaml::from_str(&action_yaml).unwrap();

                    // add all the inputs to the context as INPUT_
                    for (input_name, input) in &action_yaml.inputs {
                        // prefer the `with` value over the default value but use the default value if it exists
                        let input_value = match step.with {
                            Some(ref with) => with.get(input_name).cloned(),
                            None => None,
                        }
                        .unwrap_or_else(|| input.default.clone().unwrap_or_else(String::new));

                        let input_name =
                            format!("INPUT_{}", input_name.to_uppercase().replace('-', "_"));

                        info!(input_name = %input_name, input_value = %input_value, "Setting input");
                        self.set_env(&input_name, &input_value);
                    }

                    // get action_entry_point from the action yaml
                    let action_entry_point = action_yaml.runs.main.clone();

                    // now run node index.js in the action directory
                    let action_run = action_dir.join(action_entry_point);

                    // try to find the binary path for the action in rhe run_binary_map
                    let action_bin_path = self
                        .machine_config
                        .run_binary_map
                        .iter()
                        .find(|(key, _)| *key == &action_yaml.runs.using)
                        .map(|(_, value)| value);

                    if action_bin_path.is_none() {
                        error!("Binary not found for action");
                        continue;
                    }

                    let action_bin_path = action_bin_path.unwrap();

                    // set GITHUB_EVENT_PAYLOAD to a JSON string of the event data
                    let event_payload = serde_json::to_string(&event).unwrap();
                    self.set_env("GITHUB_EVENT_PAYLOAD", &event_payload);

                    let (logs, _succeeded) = self.run_bash_external(
                        &format!("{} {}", action_bin_path.clone(), action_run.display()),
                        working_dir.clone(),
                    );
                    let mut filtered_logs = String::new();

                    for line in logs.lines() {
                        if line.contains("::set-output") {
                            // remove the "::set-output " prefix and split on "="
                            let parts: Vec<&str> = line
                                .trim_start_matches("::set-output ")
                                .split('=')
                                .collect();
                            let key = parts[0];
                            let value = parts[1];

                            // ensure the output is listed in the action yaml
                            if action_yaml.outputs.contains_key(key) {
                                self.set_env(key, value);
                            } else {
                                warn!(key, value, "Output not defined in action yaml");
                            }
                        } else {
                            filtered_logs.push_str(line);
                            filtered_logs.push('\n');
                        }
                    }

                    // add filtered logs to the job logs
                    job_logs.push_str(&filtered_logs);
                }
            }

            // save all logs to a file to the central log directory in ~/.jikan/logs/{namespace}/{job_id}/{job_name}.log
            let log_dir = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".jikan")
                .join("logs")
                .join(self.namespace.name.clone())
                .join(job_id.clone());

            std::fs::create_dir_all(&log_dir).unwrap();
            let log_file = log_dir.join(format!("{job_name}.log"));
            logfiles.push(log_file.clone());
            std::fs::write(log_file, job_logs).unwrap();

            info!(job_name = %job_name, "Job completed");
        }

        let job_status = JobStatus {
            namespace: self.namespace.name.clone(),
            name: self.config.name.clone(),
            status: if success { "success" } else { "failure" }.to_string(),
            message: "Workflow completed".to_string(),
            timestamp: usize::try_from(Local::now().timestamp()).unwrap_or(0),
        };

        if let Err(e) = self.subscription_manager.send_event(&job_status) {
            warn!("Failed to send end event: {}", e);
        }

        info!("Workflow completed");
        Ok(RunResults { success, logfiles })
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
        let app_ctx = self.context.read();
        let env_hashmap = app_ctx.env.clone();
        let template_context = context! {
            env => env_hashmap,
        };

        let resolved = match Environment::new().render_str(condition, template_context) {
            Ok(result) => result.trim().to_lowercase() == "true",
            Err(e) => {
                error!("Failed to evaluate condition: {}", e);
                false
            }
        };
        info!(condition, resolved, "Condition evaluated");
        resolved
    }

    fn run_bash_external(&self, command: &str, working_dir: PathBuf) -> (String, bool) {
        info!("Executing bash command: {}", command);
        let context = self.context.read();
        let mut command_builder = Command::new("/bin/bash");

        let mut command_builder = command_builder
            .arg("-c")
            .arg(command)
            .env_clear()
            .envs(&context.env)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if !working_dir.to_string_lossy().is_empty() {
            command_builder = command_builder.current_dir(working_dir);
        }

        let output = command_builder.output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    info!("Command executed successfully");
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                    (String::from_utf8_lossy(&output.stdout).to_string(), true)
                } else {
                    error!("Command failed with exit code: {:?}", output.status.code());
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                    (String::from_utf8_lossy(&output.stderr).to_string(), true)
                }
            }
            Err(e) => {
                error!("Failed to execute command: {}", e);
                (e.to_string(), false)
            }
        }
    }

    fn run_python_external(
        &self,
        binary_path: &Option<String>,
        script: &str,
        working_dir: PathBuf,
    ) -> (String, bool) {
        let binding = "python".to_string();
        let python_binary = binary_path.as_ref().unwrap_or(&binding);
        let context = self.context.read();

        info!("Executing python script: {}", script);
        let output = std::process::Command::new(python_binary)
            .arg("-c")
            .arg(script)
            .current_dir(working_dir)
            .env_clear()
            .envs(&context.env)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    info!("Script executed successfully");
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                    (String::from_utf8_lossy(&output.stdout).to_string(), true)
                } else {
                    error!("Script failed with exit code: {:?}", output.status.code());
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                    (String::from_utf8_lossy(&output.stderr).to_string(), true)
                }
            }
            Err(e) => {
                error!("Failed to execute script: {}", e);
                (e.to_string(), false)
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

struct Scheduler {
    jobs: ScheduledJobs,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn add(&mut self, namespace: &str, job: Box<dyn JobTrait>) {
        let job_name = job
            .downcast_ref_to_workflow_job()
            .unwrap()
            .config
            .name
            .clone();

        let mut jobs = self.jobs.write();

        let namespace_jobs = jobs.entry(namespace.to_string()).or_default();

        let job_index = namespace_jobs
            .iter()
            .position(|j| j.downcast_ref_to_workflow_job().unwrap().config.name == job_name);

        if let Some(index) = job_index {
            namespace_jobs[index] = job;
        } else {
            namespace_jobs.push(job);
        }

        info!(namespace = %namespace, job = %job_name, "Adding job");
    }

    // TODO: allow removal via namespace wide changes
    #[allow(dead_code)]
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
            // TODO: revisit logic and if we should even be using retain_mut
            namespace_jobs.retain_mut(|job| {
                if let Some(workflow_job) = job.clone().downcast_ref_to_workflow_job() {
                    for s in &workflow_job.crons() {
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
                                    match job.run(RunEvent { event: RunEventTypes::Schedule,data: HashMap::new() }) {
                                        Ok(run_results) => {
                                            info!(run_results = ?run_results, job = %workflow_job.config.name, "Job ran successfully");
                                        }
                                        Err(e) => {
                                            error!(job = %workflow_job.config.name, error = %e, "Job failed");
                                        }
                                    };
                                }
                            }
                            Err(e) => {
                                error!(job = %workflow_job.config.name, error = %e, "Invalid cron, removing job");
                                return false; // remove the job from the list
                            }
                        }
                    }
                }
                true // default to keeping the job 
            });
            debug!(namespace = %namespace, remaining_jobs = namespace_jobs.len(), "Finished checking jobs");
        }
    }
}

fn run_forever(scheduler: Arc<Mutex<Scheduler>>) {
    tokio::spawn(async move {
        loop {
            // lock the scheduler only for the duration of the run() call
            // in the future, we may want to consider a lock-free data structure
            {
                match scheduler.lock() {
                    Ok(scheduler) => {
                        scheduler.run();
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to lock scheduler");
                    }
                }
            } // lock is released here
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

type AppData = (
    Arc<Mutex<Database>>,
    Arc<Mutex<Scheduler>>,
    Arc<SubscriptionManager>,
);

fn initialize(machine_config: MachineConfig) -> Result<AppData> {
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
    let db = Arc::new(Mutex::new(
        Database::create(database_path).context("Failed to create database")?,
    ));

    let scheduler = Arc::new(Mutex::new(Scheduler::new()));
    let broadcaster = Arc::new(StatusBroadcaster::new());
    let subscription_manager = Arc::new(SubscriptionManager::new(broadcaster.clone()));

    match hydrate_scheduler(machine_config, &db, &scheduler, &subscription_manager) {
        Ok(()) => {}
        Err(e) => {
            warn!(error = %e, "Failed to hydrate scheduler");
        }
    }

    Ok((db, scheduler, subscription_manager))
}

fn daemonize() -> Result<()> {
    let pid_file = "/tmp/jikan.pid";
    let working_dir = "/tmp";

    std::fs::create_dir_all(working_dir)?;

    let stdout = File::create("/tmp/jikan.out")?;
    let stderr = File::create("/tmp/jikan.err")?;

    // attempt to set permissions, but don't fail if it doesn't work
    let _ = std::fs::set_permissions(pid_file, std::fs::Permissions::from_mode(0o644));

    let daemonize = Daemonize::new()
        .pid_file(pid_file)
        .working_directory(working_dir)
        .stdout(stdout)
        .stderr(stderr);

    daemonize.start().context("Error daemonizing process")
}

fn main() -> Result<()> {
    let matches = clap::Command::new("jikand")
        .version("1.0")
        .author("David Holtz (drbh)")
        .about("The Jikan workflow engine daemon")
        .subcommand(clap::Command::new("daemon").about("Run the daemon in the background"))
        .subcommand(clap::Command::new("stop").about("Stop the running daemon"))
        .get_matches();

    // get native node binary path
    let node_binding = Command::new("which")
        .arg("node")
        .output()
        .context("Failed to run which")?;

    let node_binary = std::str::from_utf8(&node_binding.stdout)
        .unwrap()
        .trim()
        .to_string();

    // get native python binary path
    let python_binding = Command::new("which")
        .arg("python")
        .output()
        .context("Failed to run which")?;

    let python_binary = std::str::from_utf8(&python_binding.stdout)
        .unwrap()
        .trim()
        .to_string();

    let run_binary_map: HashMap<String, String> =
        [("node", node_binary), ("python", python_binary)]
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.to_string()))
            .collect();

    let binding = Command::new("uname")
        .arg("-s")
        .output()
        .context("Failed to run uname")?;

    // get the operating system from the uname command
    let operating_system = std::str::from_utf8(&binding.stdout);

    let operating_system = match operating_system {
        Ok("Darwin") => "macos",
        Ok("Linux") => "linux",
        _ => "unknown",
    };

    let executable_path = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // get the username from the USER environment variable
    let username = std::env::var("USER").context("Failed to get username")?;

    let machine_config = MachineConfig {
        operating_system: operating_system.to_string(),
        executable_path,
        username,
        run_binary_map,
    };

    let daemon_info = DaemonInfo {
        started_at: Local::now(),
    };

    if matches.subcommand_matches("stop").is_some() {
        let pid_file = "/tmp/jikan.pid";
        let pid = std::fs::read_to_string(pid_file)
            .context("Failed to read PID file. Is the daemon running?")?
            .trim()
            .parse::<i32>()
            .context("Failed to parse PID file. Is the daemon running?")?;
        unsafe {
            // gracefully stop the daemon
            libc::kill(pid, libc::SIGINT);
        }
        // cleanup the pid file
        std::fs::remove_file(pid_file)?;
        return Ok(());
    }

    let (db, scheduler, subscription_manager) = initialize(machine_config.clone())?;

    if matches.subcommand_matches("daemon").is_some() {
        daemonize()?;
    }

    // Create and start the Tokio runtime
    let runtime = Runtime::new()?;
    // we defer to the tokio runtime to the bottom of the main function rather
    // than with a macro to ensure that the daemonize function is called first
    runtime.block_on(async {
        start_server_and_scheduler(
            machine_config,
            db,
            scheduler,
            subscription_manager,
            daemon_info,
        )
        .await
    })
}

#[instrument(skip(db, scheduler))]
fn hydrate_scheduler(
    machine_config: MachineConfig,
    db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Mutex<Scheduler>>,
    subscription_manager: &Arc<SubscriptionManager>,
) -> Result<()> {
    let db_guard = db.lock().unwrap();
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

    let mut scheduler = scheduler.lock().unwrap();
    for result in table.iter().context("Failed to iterate over workflows")? {
        let (key, yaml_content) = result.context("Failed to read workflow entry")?;
        let (namespace, name) = key.value();

        let namespace_entry = namespace_table
            .get(namespace)
            .with_context(|| format!("Failed to get namespace: {namespace}"))?
            .ok_or_else(|| anyhow::anyhow!("Namespace not found: {namespace}"))?;

        let namespace_obj: Namespace =
            serde_json::from_str(namespace_entry.value()).context("Failed to parse namespace")?;

        match serde_yaml::from_str(yaml_content.value()) {
            Ok(config) => {
                let workflow = WorkflowJob::new(
                    config,
                    machine_config.clone(),
                    namespace_obj,
                    subscription_manager.clone(),
                );
                scheduler.add(namespace, Box::new(workflow));
                hydrated_jobs += 1;
                info!(namespace, name, "Hydrated workflow");
            }
            Err(e) => {
                error!(error = %e, workflow = name, "Failed to parse workflow config");
            }
        }
    }
    // drop the scheduler lock
    drop(scheduler);

    info!(
        hydrated_jobs = hydrated_jobs,
        "Finished hydrating scheduler"
    );
    Ok(())
}

#[instrument(skip(db, scheduler, subscription_manager))]
async fn start_server_and_scheduler(
    machine_config: MachineConfig,
    db: Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
    subscription_manager: Arc<SubscriptionManager>,
    daemon_info: DaemonInfo,
) -> Result<()> {
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
    info!(
        node = machine_config.run_binary_map.get("node"),
        python = machine_config.run_binary_map.get("python"),
        "Native binaries"
    );
    {
        // ensure the NAMESPACES table and WORKFLOWS table exist (write dummy data; key, value)
        let db = db.lock().unwrap();
        let write_txn = db
            .begin_write()
            .context("Failed to begin write transaction")?;
        {
            write_txn
                .open_table(NAMESPACES)
                .context("Failed to open namespaces table")?;

            write_txn
                .open_table(WORKFLOWS)
                .context("Failed to open workflows table")?;
        }

        write_txn.commit().context("Failed to commit transaction")?;
    }

    loop {
        let (stream, addr) = listener.accept()?;
        info!(client_addr = %addr, "New client connected");
        let db = Arc::clone(&db);
        let scheduler = Arc::clone(&scheduler);
        let subscription_manager = Arc::clone(&subscription_manager);
        let machine_clone = machine_config.clone();
        tokio::spawn(async move {
            info!(client_addr = %addr, "Handling client");
            if let Err(e) = handle_client(
                machine_clone,
                stream,
                db,
                scheduler,
                subscription_manager,
                daemon_info,
            )
            .await
            {
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
) -> Result<String> {
    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .context("Failed to begin write transaction")?;
    {
        let mut table = write_txn
            .open_table(NAMESPACES)
            .context("Failed to open namespaces table")?;
        table.remove(name).context("Failed to remove namespace")?;
    }
    write_txn.commit().context("Failed to commit transaction")?;

    let mut scheduler = scheduler.jobs.write();
    scheduler.remove(name);

    Ok(format!("Namespace '{name}' deleted successfully.\n"))
}

#[derive(Debug, Serialize)]
struct ListNamespaceNamespaceResponse {
    name: String,
    count: usize,
}

#[derive(Debug, Serialize)]
struct ListNamespaceResponse {
    namespaces: Vec<ListNamespaceNamespaceResponse>,
}

#[instrument(skip(db))]
async fn list_namespaces(db: &Arc<Mutex<Database>>) -> Result<ListNamespaceResponse> {
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(NAMESPACES)
        .context("Failed to open namespaces table")?;

    let mut list_namespace_response = ListNamespaceResponse { namespaces: vec![] };
    for result in table.iter().context("Failed to iterate over namespaces")? {
        let (name, data) = result.context("Failed to read namespace entry")?;
        let namespace: Namespace =
            serde_json::from_str(data.value()).context("Failed to parse namespace data")?;

        let namespace_path = namespace.path.clone();
        let workspace_path = namespace_path.join(".jikan").join("workflows");

        // TODO: add some notion of a namespace content sync status
        // otherwise the database may be out of sync with the filesystem
        let count = WalkDir::new(workspace_path.clone())
            .follow_links(true)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .map_or(false, |ext| ext == "yaml" || ext == "yml")
            })
            .count();

        list_namespace_response
            .namespaces
            .push(ListNamespaceNamespaceResponse {
                name: name.value().to_string(),
                count,
            });
    }

    info!("Namespaces listed successfully");
    Ok(list_namespace_response)
}

#[instrument(skip(db, scheduler))]
async fn add_workflow(
    machine_config: MachineConfig,
    namespace: &str,
    name: &str,
    yaml_content: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<String> {
    let config: WorkflowYaml =
        serde_yaml::from_str(yaml_content).context("Failed to parse workflow YAML")?;

    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .context("Failed to begin write transaction")?;
    {
        let mut table = write_txn
            .open_table(WORKFLOWS)
            .context("Failed to open workflows table")?;
        table
            .insert((namespace, name), yaml_content)
            .context("Failed to insert workflow")?;
    }
    write_txn.commit().context("Failed to commit transaction")?;

    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;

    let namespace_table = read_txn
        .open_table(NAMESPACES)
        .context("Failed to open namespaces table")?;

    let namespace_entry = namespace_table
        .get(namespace)
        .context("Failed to get namespace")?
        .ok_or_else(|| anyhow::anyhow!("Namespace not found: {}", namespace))?;

    let namespace_obj: Namespace =
        serde_json::from_str(namespace_entry.value()).context("Failed to parse namespace data")?;

    let mut scheduler = scheduler.lock().unwrap();
    let workflow = WorkflowJob::new(config, machine_config, namespace_obj, subscription_manager);
    scheduler.add(namespace, Box::new(workflow));
    drop(scheduler);

    Ok(format!(
        "Workflow '{name}' added successfully to namespace '{namespace}'.\n",
    ))
}

#[derive(Debug, Serialize)]
struct RegisterWorkflowsFromDirResponse {
    registered: usize,
}

#[instrument(skip(db, scheduler))]
async fn register_workflows_from_dir(
    machine_config: MachineConfig,
    namespace: &str,
    dir_path: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<RegisterWorkflowsFromDirResponse> {
    let mut registered = 0;

    let namespace_path = PathBuf::from(dir_path);
    let workspace_path = namespace_path.join(".jikan").join("workflows");

    // ensure the directory exists
    if !workspace_path.exists() {
        return Err(anyhow::anyhow!(
            "Directory not found: {}",
            workspace_path.display()
        ));
    }

    if workspace_path.is_relative() {
        return Err(anyhow::anyhow!(
            "Invalid directory path. Please provide an absolute path."
        ));
    }

    {
        // write the namespace to the database
        let db = db.lock().unwrap();
        let write_txn = db
            .begin_write()
            .context("Failed to begin write transaction")?;
        {
            let mut table = write_txn
                .open_table(NAMESPACES)
                .context("Failed to open namespaces table")?;
            let new_namespace = Namespace::new(namespace.to_string(), namespace_path.clone());
            table
                .insert(
                    namespace,
                    serde_json::to_string(&new_namespace).unwrap().as_str(),
                )
                .context("Failed to insert namespace")?;
        }
        write_txn.commit().context("Failed to commit transaction")?;
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
            match serde_yaml::from_str::<WorkflowYaml>(&yaml_content) {
                Ok(_config) => {
                    add_workflow(
                        machine_config.clone(),
                        namespace,
                        workflow_name,
                        &yaml_content,
                        db,
                        Arc::clone(&scheduler),
                        subscription_manager.clone(),
                    )
                    .await?;
                    registered += 1;
                }
                Err(e) => {
                    warn!(file = %entry.path().display(), error = %e, "Failed to parse workflow config");
                }
            }
        }
    }

    let response = RegisterWorkflowsFromDirResponse { registered };
    Ok(response)
}

#[derive(Debug, Serialize)]
struct UnregisterWorkflowsFromDirResponse {
    unregistered: usize,
}

#[instrument(skip(db, scheduler))]
async fn unregister_workflows_from_dir(
    namespace: &str,
    dir_path: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
) -> Result<UnregisterWorkflowsFromDirResponse> {
    let mut unregistered = 0;

    let namespace_path = PathBuf::from(dir_path);
    let workspace_path = namespace_path.join(".jikan").join("workflows");

    // ensure the directory exists
    if !workspace_path.exists() {
        return Err(anyhow::anyhow!(
            "Directory not found: {}",
            workspace_path.display()
        ));
    }

    if workspace_path.is_relative() {
        return Err(anyhow::anyhow!(
            "Invalid directory path. Please provide an absolute path."
        ));
    }

    {
        // remove the namespace from the database
        let db = db.lock().unwrap();
        let write_txn = db
            .begin_write()
            .context("Failed to begin write transaction")?;
        {
            let mut table = write_txn
                .open_table(NAMESPACES)
                .context("Failed to open namespaces table")?;
            table
                .remove(namespace)
                .context("Failed to remove namespace")?;
        }
        write_txn.commit().context("Failed to commit transaction")?;
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

            let db = db.lock().unwrap();
            let write_txn = db
                .begin_write()
                .context("Failed to begin write transaction")?;
            {
                let mut table = write_txn
                    .open_table(WORKFLOWS)
                    .context("Failed to open workflows table")?;
                table
                    .remove((namespace, workflow_name))
                    .context("Failed to remove workflow")?;
            }
            write_txn.commit().context("Failed to commit transaction")?;

            let scheduler = scheduler.lock().unwrap();
            scheduler.remove(namespace, workflow_name);
            unregistered += 1;
        }
    }

    let response = UnregisterWorkflowsFromDirResponse { unregistered };
    Ok(response)
}

#[derive(Debug, Serialize)]
struct RunWorkflowResponse {
    run_results: RunResults,
}

#[instrument(skip(db, scheduler))]
async fn run_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
    run_data_str: &str,
) -> Result<RunWorkflowResponse> {
    // first, check if the workflow exists in the database
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(WORKFLOWS)
        .context("Failed to open workflows table")?;

    // convert the run data to a HashMap<String, String> its in key=value,key=value format
    let run_data: HashMap<String, String> = run_data_str
        .split(',')
        .filter_map(|pair| {
            let mut split = pair.split('=');
            let key = split.next().unwrap_or("").to_string();
            let value = split.next().unwrap_or("").to_string();
            if key.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect();

    if let Some(_workflow) = table
        .get((namespace, name))
        .context("Failed to get workflow")?
    {
        // workflow exists in the database, now check the scheduler
        let jobs = scheduler.lock().unwrap();

        let jobs = jobs.jobs.read();

        if let Some(namespace_jobs) = jobs.get(namespace) {
            if let Some(job) = namespace_jobs.iter().find(|j| {
                j.downcast_ref_to_workflow_job()
                    .map_or(false, |wj| wj.config.name == name)
            }) {
                // found the job in the scheduler, run it
                let mut job = job.clone();
                drop(jobs); // release the read lock before running the job

                match job.run(RunEvent {
                    event: RunEventTypes::Manual,
                    data: run_data,
                }) {
                    Ok(run_results) => {
                        info!(
                            namespace = namespace,
                            workflow = name,
                            run_results = ?run_results,
                            "Workflow ran successfully"
                        );
                        let response = RunWorkflowResponse { run_results };
                        Ok(response)
                    }
                    Err(e) => {
                        error!(namespace = namespace, workflow = name, error = %e, "Workflow failed to run");
                        Err(anyhow::anyhow!(
                            "Workflow '{name}' in namespace '{namespace}' failed to run: {e}\n"
                        ))
                    }
                }
            } else {
                warn!(
                    namespace = namespace,
                    workflow = name,
                    "Workflow not found in scheduler"
                );
                Err(anyhow::anyhow!(
                    "Workflow '{name}' not found in namespace '{namespace}'.\n"
                ))
            }
        } else {
            warn!(
                namespace = namespace,
                workflow = name,
                "Namespace not found in scheduler"
            );
            Err(anyhow::anyhow!(
                "Namespace '{namespace}' not found in the scheduler. Unable to run workflow '{name}'.\n"
            ))
        }
    } else {
        warn!(
            namespace = namespace,
            workflow = name,
            "Workflow not found in database"
        );
        Err(anyhow::anyhow!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n"
        ))
    }
}

// exec_workflow is like run but is ephemeral and does not save the results and uses a fileinput
async fn exec_workflow(
    file_path: &str,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<RunWorkflowResponse> {
    // first read the workflow file
    let yaml_content = tokio::fs::read_to_string(file_path).await?;

    // parse the workflow yaml
    let config: WorkflowYaml =
        serde_yaml::from_str(&yaml_content).context("Failed to parse workflow YAML")?;

    let machine_config = MachineConfig {
        operating_system: "linux".to_string(),
        executable_path: "/usr/bin/jikan".to_string(),
        username: "jikan".to_string(),
        run_binary_map: HashMap::new(),
    };

    let namespace = Namespace::new("default".to_string(), PathBuf::new());

    let mut job = WorkflowJob::new(config, machine_config, namespace, subscription_manager);

    // run the job
    let run_results = job
        .run(RunEvent {
            event: RunEventTypes::Manual,
            data: HashMap::new(),
        })
        .map_err(|e| anyhow::anyhow!("Failed to run workflow: {e}"))?;

    Ok(RunWorkflowResponse { run_results })
}

#[derive(Debug, Serialize)]
struct ListWorkflowsResponse {
    namespace: String,
    name: String,
    status: String,
    crons: Vec<String>,
    next_runs: Vec<String>,
}

#[instrument(skip(db))]
async fn list_workflows(
    machine_config: MachineConfig,
    namespace: Option<&str>,
    db: &Arc<Mutex<Database>>,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<Vec<ListWorkflowsResponse>> {
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(WORKFLOWS)
        .context("Failed to open workflows table")?;

    let mut response = Vec::new();

    for result in table.iter().context("Failed to iterate over workflows")? {
        let (key, workflow) = result.context("Failed to read workflow entry")?;
        let (ns, name) = key.value();
        let config = serde_yaml::from_str::<WorkflowYaml>(workflow.value())
            .context("Failed to parse workflow YAML")?;

        let empty_namespace = Namespace::new(String::new(), PathBuf::new());
        let workflow = WorkflowJob::new(
            config,
            machine_config.clone(),
            empty_namespace,
            subscription_manager.clone(),
        );
        let crons = workflow.crons();

        if namespace.is_none() || namespace == Some(ns) {
            let next_runs = get_next_runs(crons.clone(), ns, name, 2)
                .iter()
                .map(|(cron_str, next_run, num_seconds)| {
                    format!(
                        "{} {} (in {} seconds)",
                        cron_str,
                        next_run.format("%Y-%m-%d %H:%M:%S"),
                        num_seconds
                    )
                })
                .collect();
            response.push(ListWorkflowsResponse {
                namespace: ns.to_string(),
                name: name.to_string(),
                status: "running".to_string(),
                crons,
                next_runs,
            });
        }
    }

    info!("Workflows listed successfully");

    Ok(response)
}

#[derive(Debug, Serialize)]
struct GetWorkflowResponse {
    namespace: String,
    name: String,
    yaml: String,
}

#[instrument(skip(db))]
async fn get_workflow(
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
) -> Result<GetWorkflowResponse> {
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(WORKFLOWS)
        .context("Failed to open workflows table")?;

    if let Some(workflow) = table
        .get((namespace, name))
        .context("Failed to get workflow")?
    {
        info!(
            namespace = namespace,
            workflow = name,
            "Workflow retrieved successfully"
        );
        let response = GetWorkflowResponse {
            namespace: namespace.to_string(),
            name: name.to_string(),
            yaml: workflow.value().to_string(),
        };
        Ok(response)
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Err(anyhow::anyhow!(
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
) -> Result<String> {
    let db = db.lock().unwrap();
    let write_txn = db
        .begin_write()
        .context("Failed to begin write transaction")?;
    {
        let mut table = write_txn
            .open_table(WORKFLOWS)
            .context("Failed to open workflows table")?;
        table
            .remove((namespace, name))
            .context("Failed to remove workflow")?;
    }
    write_txn.commit().context("Failed to commit transaction")?;
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

fn get_next_runs(
    crons: Vec<String>,
    namespace: &str,
    name: &str,
    n: usize,
) -> Vec<(String, DateTime<Local>, i64)> {
    let mut next_runs = Vec::new();
    let now = Local::now();

    for cron in crons {
        let s = if cron == "now" {
            let in_near_future = now + chrono::Duration::seconds(1);
            time_to_cron(in_near_future)
        } else {
            cron.to_string()
        };

        match CronSchedule::from_str(&s.clone()) {
            Ok(schedule) => {
                for next_time in schedule.upcoming(Local).take(n) {
                    let time_diff = next_time - now;
                    let num_seconds = time_diff.num_seconds();
                    next_runs.push((s.clone(), next_time, num_seconds));
                }
            }
            Err(e) => {
                error!(error = %e, namespace = namespace, workflow = name, cron = %s, "Invalid cron expression");
            }
        }
    }
    next_runs
}

#[derive(Debug, Serialize)]
struct GetNextRunResponse {
    namespace: String,
    name: String,
    next_runs: Vec<String>,
}

#[instrument]
async fn get_next_run_workflow(
    machine_config: MachineConfig,
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<GetNextRunResponse> {
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(WORKFLOWS)
        .context("Failed to open workflows table")?;

    let namespace_table = read_txn
        .open_table(NAMESPACES)
        .context("Failed to open namespaces table")?;

    if let Some(workflow) = table
        .get((namespace, name))
        .context("Failed to get workflow")?
    {
        let config: WorkflowYaml =
            serde_yaml::from_str(workflow.value()).context("Failed to parse workflow YAML")?;

        let namespace_entry = namespace_table
            .get(namespace)
            .context("Failed to get namespace")?
            .ok_or_else(|| anyhow::anyhow!("Namespace not found"))?;

        let namespace_obj: Namespace =
            serde_json::from_str(namespace_entry.value()).context("Failed to parse namespace")?;

        let job = WorkflowJob::new(config, machine_config, namespace_obj, subscription_manager);
        let crons = job.crons();

        let mut next_runs = get_next_runs(crons, namespace, name, 1);

        if next_runs.is_empty() {
            info!(
                namespace = namespace,
                workflow = name,
                "No valid cron expressions found"
            );
            Ok(GetNextRunResponse {
                namespace: namespace.to_string(),
                name: name.to_string(),
                next_runs: vec!["No valid cron expressions found".to_string()],
            })
        } else {
            next_runs.sort_by_key(|&(_, _, seconds)| seconds);
            let mut response = GetNextRunResponse {
                namespace: namespace.to_string(),
                name: name.to_string(),
                next_runs: vec![],
            };
            for (cron_str, next_time, seconds) in next_runs.clone() {
                response.next_runs.push(format!(
                    "{} {} (in {} seconds)",
                    cron_str,
                    next_time.to_rfc2822(),
                    seconds
                ));
            }

            info!(
                namespace = namespace,
                workflow = name,
                next_runs = ?next_runs,
                "Next runs calculated"
            );
            Ok(response)
        }
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Err(anyhow::anyhow!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n"
        ))
    }
}

#[derive(Debug, Serialize)]
struct LogFileResponse {
    file_name: String,
    tail: String,
}

#[derive(Debug, Serialize)]
struct LastRunResponse {
    namespace: String,
    name: String,
    last_run: String,
    logs: Vec<LogFileResponse>,
}

#[instrument]
async fn get_last_run_workflow(
    machine_config: MachineConfig,
    namespace: &str,
    name: &str,
    db: &Arc<Mutex<Database>>,
    subscription_manager: Arc<SubscriptionManager>,
) -> Result<LastRunResponse> {
    let db = db.lock().unwrap();
    let read_txn = db
        .begin_read()
        .context("Failed to begin read transaction")?;
    let table = read_txn
        .open_table(WORKFLOWS)
        .context("Failed to open workflows table")?;

    if let Some(workflow) = table
        .get((namespace, name))
        .context("Failed to get workflow")?
    {
        let config: WorkflowYaml =
            serde_yaml::from_str(workflow.value()).context("Failed to parse workflow YAML")?;

        let namespace_table = read_txn
            .open_table(NAMESPACES)
            .context("Failed to open namespaces table")?;

        let namespace_entry = namespace_table
            .get(namespace)
            .context("Failed to get namespace")?
            .ok_or_else(|| anyhow::anyhow!("Namespace not found"))?;

        let namespace_obj: Namespace =
            serde_json::from_str(namespace_entry.value()).context("Failed to parse namespace")?;

        let _job = WorkflowJob::new(
            config,
            machine_config,
            namespace_obj.clone(),
            subscription_manager.clone(),
        );

        // get the logs from the last run using the directory path
        let logs_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".jikan")
            .join("logs")
            .join(namespace);

        // get a list of all of the directories in the last run directory
        let logs = std::fs::read_dir(logs_path.clone()).unwrap();
        let mut last_run = String::new();

        for log in logs {
            let log = log.unwrap();
            let name = log.file_name().into_string().unwrap();
            if name > last_run {
                last_run = name;
            }
        }

        // tail the all of the log files in the directory
        let logs = std::fs::read_dir(logs_path.join(last_run.clone())).unwrap();
        let mut logs_vec = Vec::new();
        for log in logs {
            let log = log.unwrap();
            let log_path = log.path();
            let log_content = std::fs::read_to_string(log_path).unwrap();

            let resp = LogFileResponse {
                file_name: log.file_name().into_string().unwrap(),
                tail: log_content,
            };

            logs_vec.push(resp);
        }

        let response = LastRunResponse {
            namespace: namespace.to_string(),
            name: name.to_string(),
            last_run: last_run.clone(),
            logs: logs_vec,
        };

        info!(
            namespace = namespace,
            workflow = name,
            last_run = last_run,
            "Last run retrieved"
        );

        Ok(response)
    } else {
        warn!(namespace = namespace, workflow = name, "Workflow not found");
        Err(anyhow::anyhow!(
            "Workflow '{name}' not found in namespace '{namespace}'.\n"
        ))
    }
}

#[derive(Debug, Serialize)]
struct SetEnvResponse {
    message: String,
}

fn set_env(
    namespace: &str,
    name: &str,
    key: &str,
    value: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Mutex<Scheduler>>,
) -> Result<SetEnvResponse> {
    let jobs = scheduler.lock().unwrap();
    let jobs = jobs.jobs.write();

    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                workflow_job.set_env(key, value);
                Ok(SetEnvResponse {
                    message: format!(
                        "Environment variable '{key}' set to '{value}' for workflow '{name}' in namespace '{namespace}'.\n",
                    ),
                })
            } else {
                Err(anyhow::anyhow!(
                    "Workflow '{name}' in namespace '{namespace}' is not a WorkflowJob.\n"
                ))
            }
        } else {
            Err(anyhow::anyhow!(
                "Workflow '{name}' not found in namespace '{namespace}'.\n"
            ))
        }
    } else {
        Err(anyhow::anyhow!("Namespace '{namespace}' not found.\n"))
    }
}

#[derive(Debug, Serialize)]
struct GetEnvResponse {
    key: String,
    value: String,
}

fn get_env(
    namespace: &str,
    name: &str,
    key: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Mutex<Scheduler>>,
) -> Result<GetEnvResponse> {
    let jobs = scheduler.lock().unwrap();
    let jobs = jobs.jobs.read();
    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                Ok(GetEnvResponse {
                    key: key.to_string(),
                    value: workflow_job.get_env(key).unwrap_or_default(),
                })
            } else {
                Err(anyhow::anyhow!(
                    "Workflow '{name}' in namespace '{namespace}' is not a WorkflowJob.\n"
                ))
            }
        } else {
            Err(anyhow::anyhow!(
                "Workflow '{name}' not found in namespace '{namespace}'.\n"
            ))
        }
    } else {
        Err(anyhow::anyhow!("Namespace '{namespace}' not found.\n"))
    }
}

#[derive(Debug, Serialize)]
struct ListEnvResponse {
    key: String,
    value: String,
}

fn list_env(
    namespace: &str,
    name: &str,
    _db: &Arc<Mutex<Database>>,
    scheduler: &Arc<Mutex<Scheduler>>,
) -> Result<Vec<ListEnvResponse>> {
    let jobs = scheduler.lock().unwrap();
    let jobs = jobs.jobs.read();
    if let Some(namespace_jobs) = jobs.get(namespace) {
        if let Some(job) = namespace_jobs.iter().find(|j| {
            j.downcast_ref_to_workflow_job()
                .map_or(false, |wj| wj.config.name == name)
        }) {
            if let Some(workflow_job) = job.downcast_ref_to_workflow_job() {
                let env_vars = workflow_job.list_env();
                let mut response = Vec::new();
                for (key, value) in env_vars {
                    response.push(ListEnvResponse { key, value });
                }
                Ok(response)
            } else {
                Err(anyhow::anyhow!(
                    "Workflow '{name}' in namespace '{namespace}' is not a WorkflowJob.\n"
                ))
            }
        } else {
            Err(anyhow::anyhow!(
                "Workflow '{name}' not found in namespace '{namespace}'.\n"
            ))
        }
    } else {
        Err(anyhow::anyhow!("Namespace '{namespace}' not found.\n"))
    }
}

#[derive(Debug, Serialize)]
struct DaemonInfoResponse {
    started_at: String,
    uptime: String,
    version: String,
}

#[instrument]
async fn get_daemon_info(daemon_info: DaemonInfo) -> Result<DaemonInfoResponse> {
    let version = env!("CARGO_PKG_VERSION");
    let response = DaemonInfoResponse {
        started_at: daemon_info.started_at.to_rfc2822(),
        uptime: format!(
            "{}",
            Local::now()
                .signed_duration_since(daemon_info.started_at)
                .num_seconds()
        ),
        version: version.to_string(),
    };

    Ok(response)
}

#[derive(Debug, Serialize)]
struct DetailedLogFileResponse {
    file_name: String,
    content: String,
}

#[instrument]
async fn get_log_file(
    namespace: &str,
    run_id: &str,
    job_name: Option<&str>,
    db: &Arc<Mutex<Database>>,
) -> Result<Vec<DetailedLogFileResponse>> {
    let mut logs_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".jikan")
        .join("logs")
        .join(namespace)
        .join(run_id);

    if let Some(job_name) = job_name {
        logs_path = logs_path.join(job_name);
    }

    let mut response = Vec::new();

    // for each file in the directory, read the content and return it
    let logs = std::fs::read_dir(logs_path.clone()).unwrap();
    for log in logs {
        let log = log.unwrap();
        let log_path = log.path();
        let log_content = std::fs::read_to_string(log_path).unwrap();

        let resp = DetailedLogFileResponse {
            file_name: log.file_name().into_string().unwrap(),
            content: log_content,
        };

        response.push(resp);
    }

    Ok(response)
}

// get_log_timeline list all the the logs in the logs directory under the namespace
#[derive(Debug, Serialize)]
struct LogTimelineResponse {
    namespace: String,
    logs: Vec<String>,
}

#[instrument]
fn get_log_timeline(namespace: &str) -> Result<LogTimelineResponse> {
    let logs_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".jikan")
        .join("logs")
        .join(namespace);

    let logs = std::fs::read_dir(logs_path.clone()).unwrap();
    let mut logs_vec = Vec::new();

    for log in logs {
        let log = log.unwrap();
        let name = log.file_name().into_string().unwrap();
        logs_vec.push(name);
    }

    let response = LogTimelineResponse {
        namespace: namespace.to_string(),
        logs: logs_vec,
    };

    Ok(response)
}

use git2::Repository;

#[derive(Debug, Serialize)]
struct GitCloneResponse {
    url: String,
    target_path: String,
    local_path: String,
}

enum ActionCloneTarget {
    Path(String),
    GitUrl(String),
}

fn git_clone_action(
    url_or_path: &str,
    target_path: &str,
    action_id: &str,
) -> Result<GitCloneResponse> {
    let base_path = dirs::home_dir()
        .context("Failed to get home directory")?
        .join(".jikan/actions");

    let action_clone_target = if url_or_path.starts_with("http") {
        ActionCloneTarget::GitUrl(url_or_path.to_string())
    } else {
        ActionCloneTarget::Path(url_or_path.to_string())
    };

    let local_action_path = base_path.join(action_id);
    std::fs::create_dir_all(&local_action_path)?;

    // if its a git url, clone the repo to the local path if its a path, copy the contents to the local path
    let (tmp_dir, local_target_path) = match action_clone_target {
        ActionCloneTarget::Path(path) => {
            let target_path = PathBuf::from(path);
            (None, target_path)
        }
        ActionCloneTarget::GitUrl(url) => {
            let tmp_dir = tempfile::tempdir()?;
            let _ = Repository::clone(&url, tmp_dir.path())?;
            let target_path_within_repo = tmp_dir.path().join(PathBuf::from(target_path));
            (Some(tmp_dir), target_path_within_repo)
        }
    };

    std::fs::create_dir_all(&local_action_path)?;

    for entry in WalkDir::new(local_target_path.clone())
        .follow_links(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let entry_path = entry.path();
        let relative_path = entry_path.strip_prefix(local_target_path.clone())?;
        let target_path = local_action_path.join(relative_path);
        if entry_path.is_dir() {
            std::fs::create_dir_all(target_path)?;
        } else {
            std::fs::copy(entry_path, target_path)?;
        }
    }

    // if its a git url we need to close the temp dir
    if let Some(tmp_dir) = tmp_dir {
        tmp_dir.close()?;
    }

    Ok(GitCloneResponse {
        url: url_or_path.to_string(),
        target_path: target_path.to_string(),
        local_path: local_action_path.to_string_lossy().to_string(),
    })
}

fn serialize_response<T: Serialize>(response: T) -> Result<String> {
    serde_json::to_string(&response).context("Failed to serialize response")
}

#[instrument(skip(stream, db, scheduler, subscription_manager), fields(client_addr = %stream.peer_addr().unwrap()))]
async fn handle_client(
    machine_config: MachineConfig,
    stream: TcpStream,
    db: Arc<Mutex<Database>>,
    scheduler: Arc<Mutex<Scheduler>>,
    subscription_manager: Arc<SubscriptionManager>,
    daemon_info: DaemonInfo,
) -> Result<()> {
    let mut stream = TokioTcpStream::from_std(stream)?;
    let (reader, mut writer) = stream.split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    debug!(command = %line.trim(), "Received command");
    let split_line = line.split_whitespace().collect::<Vec<&str>>();

    // handle long-lived interactions
    let long_lived = match split_line.as_slice() {
        ["SUBSCRIBE_STATUS"] | ["SUBSCRIBE_STATUS", ..] => {
            let workflows_to_watch: Vec<String> =
                split_line[1..].iter().map(|&s| s.to_string()).collect();
            let subscriber_id = format!("{:?}", "david");
            let receiver =
                subscription_manager.add_subscriber(&subscriber_id, workflows_to_watch.clone());
            Some((receiver, subscriber_id))
        }
        _ => None,
    };
    let subscription_manager = Arc::clone(&subscription_manager);

    // long responses will keep the tcp connection open and send updates to the client
    if let Some((receiver, subscriber_id)) = long_lived {
        while let Ok(status) = receiver.recv() {
            let status_json = serde_json::to_string(&status).unwrap();
            if let Err(e) = writer.write_all(status_json.as_bytes()).await {
                error!("Failed to send status update: {}", e);
                break;
            }
            if let Err(e) = writer.write_all(b"\n").await {
                error!("Failed to send newline: {}", e);
                break;
            }
            if let Err(e) = writer.flush().await {
                error!("Failed to flush writer: {}", e);
                break;
            }
        }

        subscription_manager.remove_subscriber(&subscriber_id);
        return Ok(());
    }

    let response = match split_line.as_slice() {
        ["DAEMON_INFO"] => serialize_response(get_daemon_info(daemon_info).await?)?,
        ["LIST_NAMESPACES"] => serialize_response(list_namespaces(&db).await?)?,
        ["REGISTER_DIR", namespace, dir_path] => serialize_response(register_workflows_from_dir(machine_config, namespace, dir_path, &db, scheduler, subscription_manager).await?)?,
        ["UNREGISTER_DIR", namespace, dir_path] => serialize_response(unregister_workflows_from_dir(namespace, dir_path, &db, scheduler).await?)?,
        ["NEXT", namespace, name] => serialize_response(get_next_run_workflow(machine_config, namespace, name, &db, subscription_manager).await?)?,
        ["LAST", namespace, name] => serialize_response(get_last_run_workflow(machine_config, namespace, name, &db, subscription_manager).await?)?,
        ["LIST"] => serialize_response(list_workflows(machine_config, None, &db, subscription_manager).await?)?,
        ["LIST", namespace] => serialize_response(list_workflows(machine_config, Some(namespace), &db, subscription_manager).await?)?,
        ["GET", namespace, name] => serialize_response(get_workflow(namespace, name, &db).await?)?,
        ["RUN", namespace, name] => serialize_response(run_workflow(namespace, name, &db, scheduler, "").await?)?,
        ["RUN", namespace, name, run_data] => {
            serialize_response(run_workflow(namespace, name, &db, scheduler, run_data).await?)?
        }
        ["EXEC", filepath] => serialize_response(exec_workflow(filepath, subscription_manager).await?)?,
        ["SET_ENV", namespace, name, key, value] => {
            serialize_response(set_env(namespace, name, key, value, &db, &scheduler)?)?
        }
        ["GET_ENV", namespace, name, key] => serialize_response(get_env(namespace, name, key, &db, &scheduler)?)?,
        ["LIST_ENV", namespace, name] => serialize_response(list_env(namespace, name, &db, &scheduler)?)?,
        ["GET_LOG", namespace, run_id, job_name] => {
            serialize_response(get_log_file(namespace, run_id, Some(job_name), &db).await?)?
        }
        ["GET_LOG", namespace, run_id] => {
            serialize_response(get_log_file(namespace, run_id, None, &db).await?)?
        }
        ["GET_LOG_TIMELINE", namespace] => {
            serialize_response(get_log_timeline(namespace)?)?
        }
        ["CLONE", url, target_path, action_id] => {
            serialize_response(git_clone_action(url, target_path, action_id)?)?
        }
        _ => {
            warn!(command = %line.trim(), "Invalid command received");
            "Invalid command. Use ADD_NAMESPACE, LIST_NAMESPACES, REGISTER_DIR, ADD, LIST, GET, or DELETE.\n".to_string()
        }
    }.to_string();

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
