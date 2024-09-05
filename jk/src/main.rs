use clap::{Arg, Command};
use colored::Colorize;
use serde_json::Value;
use std::process::Command as ProcessCommand;

fn format_json(json_str: &str, color: bool) -> String {
    let parsed: Value = serde_json::from_str(json_str).unwrap_or(Value::Null);
    let formatted = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| json_str.to_string());

    if color {
        colorize_json(&formatted)
    } else {
        formatted
    }
}

fn colorize_json(json_str: &str) -> String {
    json_str
        .lines()
        .map(|line| {
            if line.contains(':') {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                let key = parts[0].trim().trim_matches('"');
                let value = parts.get(1).map_or("", |s| s.trim());

                format!("{}: {}", key.blue(), value.green())
            } else if line.trim().starts_with('{')
                || line.trim().starts_with('}')
                || line.trim().starts_with('[')
                || line.trim().starts_with(']')
            {
                line.normal().to_string()
            } else {
                line.green().to_string()
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

fn default_namespace() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p: std::path::PathBuf| {
            p.file_name()
                // get the directory name as a string
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|_|
            // ensure that this is a valid jikan directory
            std::path::PathBuf::from(".jikan").exists())
        .unwrap_or_default()
}

// TODO: revisit the formatting after daemon updates
#[allow(dead_code)]
fn add_namespace(name: &str) -> String {
    if name.contains('/') {
        name.to_string()
    } else {
        format!("{}/{}", default_namespace(), name)
    }
}

// split namespace and name into a tuple
fn split_namespace_and_name(name: &str) -> (String, String) {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (default_namespace(), name.to_string())
    }
}

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let location_when_cli_is_run = std::env::current_dir()?;

    let matches = Command::new("jk")
        .version("1.0")
        .author("David Holtz (drbh)")
        .about("Control interface for Jikan workflow execution daemon")
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .help("Format and colorize output")
                .action(clap::ArgAction::SetTrue)
                .default_value("false"),
        )
        .subcommand(
            Command::new("list")
                .alias("ls")
                .about("List workflows")
                .arg(Arg::new("namespace").required(false))
                .arg(
                    Arg::new("all")
                        .short('a')
                        .long("all")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("get")
                .about("Get workflow details")
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("run")
                .about("Run a workflow")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("args").required(false)),
        )
        .subcommand(
            Command::new("exec")
                .about("Execute a workflow")
                .arg(Arg::new("file").required(true)),
        )
        .subcommand(Command::new("list_namespaces").about("List namespaces"))
        .subcommand(
            Command::new("set_env")
                .about("Set an environment variable")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true))
                .arg(Arg::new("value").required(true)),
        )
        .subcommand(
            Command::new("get_env")
                .about("Get an environment variable")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true)),
        )
        .subcommand(
            Command::new("list_env")
                .about("List environment variables")
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("next")
                .about("Check the next scheduled run")
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("last")
                .about("Check the last run")
                // .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(Command::new("init").about("Initialize a new Jikan project"))
        .subcommand(Command::new("sync").about("Reload workflows in the current namespace"))
        .subcommand(Command::new("unregister").about("Unregister the current namespace"))
        .subcommand(Command::new("daemon_info").about("Get daemon info"))
        .subcommand(Command::new("get_log_timeline").arg(Arg::new("name")))
        .subcommand(
            Command::new("get_log")
                .about("Get log for a job")
                .arg(Arg::new("run_id").required(true)),
        )
        .subcommand(
            Command::new("clone")
                .about("Clone a remote action")
                .arg(
                    Arg::new("url")
                        .required(true)
                        .help("URL of the repository to clone"),
                )
                .arg(
                    Arg::new("folder")
                        .required(true)
                        .help("Specific folder within the repository to clone"),
                )
                .arg(Arg::new("action_id").required(true)),
        )
        .get_matches();

    let format_output = matches.get_flag("format");

    // before we do anything, we need to check if the daemon is running
    let output = ProcessCommand::new("sh")
        .arg("-c")
        .arg("nc -z localhost 8080")
        .output()?;
    if !output.status.success() {
        eprintln!("Jikan daemon is not running. Please start the daemon before running commands.");
        return Ok(());
    }

    let command = match matches.subcommand() {
        Some(("list" | "ls", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or_else(default_namespace, std::string::ToString::to_string);

            let all = sub_m.get_flag("all");

            if all {
                "LIST".to_string()
            } else {
                format!("LIST {namespace}")
            }
        }
        Some(("get", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            format!("GET {namespace} {name}")
        }
        Some(("run", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            let args = sub_m.get_one::<String>("args").map_or("", String::as_str);
            format!("RUN {namespace} {name} {args}")
        }
        Some(("exec", sub_m)) => {
            let file = sub_m.get_one::<String>("file").unwrap();
            // get current directory and prepend to file
            let file = location_when_cli_is_run.join(file);
            format!("EXEC {}", file.to_str().unwrap())
        }
        Some(("list_namespaces", _)) => "LIST_NAMESPACES".to_string(),
        Some(("daemon_info", _sub_m)) => "DAEMON_INFO".to_string(),
        Some(("set_env", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            let key = sub_m.get_one::<String>("key").unwrap();
            let value = sub_m.get_one::<String>("value").unwrap();
            format!("SET_ENV {namespace} {name} {key} {value}")
        }
        Some(("get_env", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            let key = sub_m.get_one::<String>("key").unwrap();
            format!("GET_ENV {namespace} {name} {key}")
        }
        Some(("list_env", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            format!("LIST_ENV {namespace} {name}")
        }
        Some(("next", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            format!("NEXT {namespace} {name}")
        }
        Some(("last", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let (namespace, name) = split_namespace_and_name(name);
            format!("LAST {namespace} {name}")
        }
        Some(("init" | "sync", _sub_m)) => {
            let namespace = location_when_cli_is_run
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();

            // add directories if they don't exist
            let _ = std::fs::create_dir_all(".jikan/workflows");

            format!(
                "REGISTER_DIR {} {}",
                namespace,
                location_when_cli_is_run.to_str().unwrap()
            )
        }
        Some(("unregister", _sub_m)) => {
            let namespace = location_when_cli_is_run
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();

            format!(
                "UNREGISTER_DIR {} {}",
                namespace,
                location_when_cli_is_run.to_str().unwrap()
            )
        }
        Some(("get_log", sub_m)) => {
            let name = sub_m
                .get_one::<String>("name")
                .unwrap_or(&String::new())
                .clone();
            let (namespace, _name) = split_namespace_and_name(&name);
            let run_id = sub_m.get_one::<String>("run_id").unwrap();
            format!("GET_LOG {namespace} {run_id}")
        }
        Some(("get_log_timeline", sub_m)) => {
            let name = sub_m
                .get_one::<String>("name")
                .unwrap_or(&String::new())
                .clone();
            let (namespace, _name) = split_namespace_and_name(&name);
            format!("GET_LOG_TIMELINE {namespace}")
        }
        Some(("clone", sub_m)) => {
            let url = sub_m.get_one::<String>("url").unwrap();
            let folder = sub_m.get_one::<String>("folder").unwrap();
            let action_id = sub_m.get_one::<String>("action_id").unwrap();
            format!("CLONE {url} {folder} {action_id}")
        }
        _ => {
            println!("Invalid command. Use --help for usage information.");
            return Ok(());
        }
    };

    let output = ProcessCommand::new("sh")
        .arg("-c")
        .arg(format!("echo '{command}' | nc -w 1 localhost 8080"))
        .output()?;

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        let formatted_response = if format_output {
            format_json(&response, true)
        } else {
            response.to_string()
        };
        println!("{}", formatted_response.trim());
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", error.trim().red());
    }

    Ok(())
}
