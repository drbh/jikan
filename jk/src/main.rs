use clap::{Arg, Command};
use std::process::Command as ProcessCommand;

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
        .arg(Arg::new("json").short('j').long("json"))
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
        .get_matches();

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
        println!("{}", response.trim());
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        eprintln!("Error: {}", error.trim());
    }

    Ok(())
}
