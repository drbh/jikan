use clap::{Arg, Command};
use std::process::Command as ProcessCommand;

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let location_when_cli_is_run = std::env::current_dir()?;

    let matches = Command::new("jikanctl")
        .version("1.0")
        .author("Your Name")
        .about("Control interface for Jikan workflow execution daemon")
        .subcommand(
            Command::new("add")
                .about("Add a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("body").required(true)),
        )
        .subcommand(
            Command::new("list")
                .about("List workflows")
                .arg(Arg::new("namespace").required(false)),
        )
        .subcommand(
            Command::new("get")
                .about("Get workflow details")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("delete")
                .about("Delete a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("run")
                .about("Run a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("add_namespace")
                .about("Add a namespace")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("path").required(true)),
        )
        .subcommand(Command::new("LIST_NAMESPACES").about("List namespaces"))
        .subcommand(
            Command::new("delete_namespace")
                .about("Delete a namespace")
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("set_env")
                .about("Set an environment variable")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true))
                .arg(Arg::new("value").required(true)),
        )
        .subcommand(
            Command::new("get_env")
                .about("Get an environment variable")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true)),
        )
        .subcommand(
            Command::new("list_env")
                .about("List environment variables")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("register_dir")
                .about("Register directory-based workflows")
                .arg(Arg::new("namespace").required(true))
                .arg(Arg::new("dir_path").required(true)),
        )
        .subcommand(
            Command::new("next")
                .about("Check the next scheduled run")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(Command::new("init").about("Initialize a new Jikan project"))
        .get_matches();

    let command = match matches.subcommand() {
        Some(("add", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            let body = sub_m.get_one::<String>("body").unwrap();
            format!("ADD {namespace} {name} {body}")
        }
        Some(("list", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            format!("LIST {namespace}")
        }
        Some(("get", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("GET {namespace} {name}")
        }
        Some(("delete", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("DELETE {namespace} {name}")
        }
        Some(("run", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("RUN {namespace} {name}")
        }
        Some(("add_namespace", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let path = sub_m.get_one::<String>("path").unwrap();

            let path = if path.starts_with('/') {
                path.clone()
            } else {
                location_when_cli_is_run
                    .join(path)
                    .to_str()
                    .unwrap()
                    .to_string()
            };

            format!("ADD_NAMESPACE {name} {path}")
        }
        Some(("list_namespaces", _)) => "LIST_NAMESPACES".to_string(),
        Some(("DELETE_NAMESPACE", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("DELETE_NAMESPACE {name}")
        }
        Some(("set_env", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            let key = sub_m.get_one::<String>("key").unwrap();
            let value = sub_m.get_one::<String>("value").unwrap();
            format!("SET_ENV {namespace} {name} {key} {value}")
        }
        Some(("get_env", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            let key = sub_m.get_one::<String>("key").unwrap();
            format!("GET_ENV {namespace} {name} {key}")
        }
        Some(("list_env", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("LIST_ENV {namespace} {name}")
        }
        Some(("register_dir", sub_m)) => {
            let namespace = sub_m.get_one::<String>("namespace").unwrap();
            let dir_path = sub_m.get_one::<String>("dir_path").unwrap();

            let dir_path = if dir_path.starts_with('/') {
                dir_path.clone()
            } else {
                location_when_cli_is_run
                    .join(dir_path)
                    .to_str()
                    .unwrap()
                    .to_string()
            };

            format!("REGISTER_DIR {namespace} {dir_path}")
        }
        Some(("next", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map_or("default", String::as_str);
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("NEXT {namespace} {name}")
        }
        Some(("init", _sub_m)) => {
            let namespace = location_when_cli_is_run
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            format!(
                "REGISTER_DIR {} {}",
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
        println!("Server response: {}", response.trim());
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        eprintln!("Error: {}", error.trim());
    }

    Ok(())
}
