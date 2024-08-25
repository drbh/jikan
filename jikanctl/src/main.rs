use clap::{Arg, Command};
use std::process::Command as ProcessCommand;

fn main() -> std::io::Result<()> {
    let location_when_cli_is_run = std::env::current_dir()?;

    let matches = Command::new("jikanctl")
        .version("1.0")
        .author("Your Name")
        .about("Control interface for Jikan workflow execution daemon")
        .subcommand(
            Command::new("ADD")
                .about("Add a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("body").required(true)),
        )
        .subcommand(
            Command::new("LIST")
                .about("List workflows")
                .arg(Arg::new("namespace").required(false)),
        )
        .subcommand(
            Command::new("GET")
                .about("Get workflow details")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("DELETE")
                .about("Delete a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("RUN")
                .about("Run a workflow")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("ADD_NAMESPACE")
                .about("Add a namespace")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("path").required(true)),
        )
        .subcommand(Command::new("LIST_NAMESPACES").about("List namespaces"))
        .subcommand(
            Command::new("DELETE_NAMESPACE")
                .about("Delete a namespace")
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("SET_ENV")
                .about("Set an environment variable")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true))
                .arg(Arg::new("value").required(true)),
        )
        .subcommand(
            Command::new("GET_ENV")
                .about("Get an environment variable")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("key").required(true)),
        )
        .subcommand(
            Command::new("LIST_ENV")
                .about("List environment variables")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(
            Command::new("REGISTER_DIR")
                .about("Register directory-based workflows")
                .arg(Arg::new("namespace").required(true))
                .arg(Arg::new("dir_path").required(true)),
        )
        .subcommand(
            Command::new("NEXT")
                .about("Check the next scheduled run")
                .arg(Arg::new("namespace").required(false))
                .arg(Arg::new("name").required(true)),
        )
        .subcommand(Command::new("init").about("Initialize a new Jikan project"))
        .get_matches();

    let command = match matches.subcommand() {
        Some(("ADD", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            let body = sub_m.get_one::<String>("body").unwrap();
            format!("ADD {} {} {}", namespace, name, body)
        }
        Some(("LIST", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            format!("LIST {}", namespace)
        }
        Some(("GET", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("GET {} {}", namespace, name)
        }
        Some(("DELETE", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("DELETE {} {}", namespace, name)
        }
        Some(("RUN", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("RUN {} {}", namespace, name)
        }
        Some(("ADD_NAMESPACE", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            let path = sub_m.get_one::<String>("path").unwrap();

            let path = if path.starts_with("/") {
                path.clone()
            } else {
                location_when_cli_is_run
                    .join(path)
                    .to_str()
                    .unwrap()
                    .to_string()
            };

            format!("ADD_NAMESPACE {} {}", name, path)
        }
        Some(("LIST_NAMESPACES", _)) => "LIST_NAMESPACES".to_string(),
        Some(("DELETE_NAMESPACE", sub_m)) => {
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("DELETE_NAMESPACE {}", name)
        }
        Some(("SET_ENV", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            let key = sub_m.get_one::<String>("key").unwrap();
            let value = sub_m.get_one::<String>("value").unwrap();
            format!("SET_ENV {} {} {} {}", namespace, name, key, value)
        }
        Some(("GET_ENV", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            let key = sub_m.get_one::<String>("key").unwrap();
            format!("GET_ENV {} {} {}", namespace, name, key)
        }
        Some(("LIST_ENV", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("LIST_ENV {} {}", namespace, name)
        }
        Some(("REGISTER_DIR", sub_m)) => {
            let namespace = sub_m.get_one::<String>("namespace").unwrap();
            let dir_path = sub_m.get_one::<String>("dir_path").unwrap();

            let dir_path = if dir_path.starts_with("/") {
                dir_path.clone()
            } else {
                location_when_cli_is_run
                    .join(dir_path)
                    .to_str()
                    .unwrap()
                    .to_string()
            };

            format!("REGISTER_DIR {} {}", namespace, dir_path)
        }
        Some(("NEXT", sub_m)) => {
            let namespace = sub_m
                .get_one::<String>("namespace")
                .map(String::as_str)
                .unwrap_or("default");
            let name = sub_m.get_one::<String>("name").unwrap();
            format!("NEXT {} {}", namespace, name)
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
        .arg(format!("echo '{}' | nc -w 1 localhost 8080", command))
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
