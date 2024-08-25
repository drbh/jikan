# Jikan (時間): Simplified Workflow Execution

> [!CAUTION]
> This project is in the early stages of development and is not yet stable. Please use with caution and expect breaking changes as 1.0.0 approaches.

Jikan is a lightweight workflow execution daemon inspired by GitHub Actions, offering more features than cron while being simpler than traditional workflow engines and CI/CD pipelines.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Core Concepts](#core-concepts)
3. [Installation](#installation)
4. [Basic Usage](#basic-usage)
5. [Workflow Configuration](#workflow-configuration)
6. [Command Reference](#command-reference)
7. [Advanced Usage](#advanced-usage)
8. [Troubleshooting](#troubleshooting)
9. [Contributing](#contributing)
10. [License](#license)

## Quick Start

1. Clone the repository:

```bash
# install the daemon
cargo install --git https://github.com/drbh/jikan.git jikan
# install the CLI
cargo install --git https://github.com/drbh/jikan.git jikanctl
```

2. Start the Jikan daemon:

```bash
jikan
# or run in the background
jikan daemon
# and to stop the daemon
jikan stop
```

3. Create a simple workflow file (e.g., `.jikan/workflows/hello-world.yaml`):

```yaml
name: hello-world
on:
  schedule:
    - cron: "0 * * * * * *" # Run every minute
jobs:
  say-hello:
    runs-on: bash
    steps:
      - run: echo "Hello, Jikan!"
```

> [!TIP]
> It's recommend to create a directory/repo (similar to github actions) to store your workflows. For example, the directory structure should be similar to the `example/daily-brief` directory:

```bash
.
├── .jikan
│   └── workflows
│       └── daily-brief.yaml
└── daily_brief.md
```

4. Add all the workflows in the directory:

```bash
cd example/daily-brief
jikanctl init
# Server response: Registered 1 workflows in namespace 'daily-brief'.
```

5. List your workflows:

```bash
jikanctl list daily-brief
# Server response: Workflows:
# - daily-brief (namespace: daily-brief)
```

6. See the next scheduled run:

```bash
jikanctl next daily-brief daily-brief
# Server response: Workflow 'daily-brief' in namespace 'daily-brief' will run in 1239 seconds.
# Absolute time: Sun, 25 Aug 2024 14:07:00 -0400
```

## Core Concepts

- **Workflows**: Defined tasks that Jikan executes based on triggers.
- **Namespaces**: Logical groupings for organizing workflows.
- **Jobs**: Individual units of work within a workflow.
- **Triggers**: Events that initiate workflow execution (e.g., schedules, manual runs).

## Basic Usage

1. Start the Jikan daemon:

```bash
jikanctl --help
```

2. Use the `jikanctl` tool to interact with Jikan:

```bash
jikanctl command [arguments]
```

## Workflow Configuration

Create YAML files to define your workflows. Example:

```yaml
name: my-workflow
run-name: some-run-name
on:
  schedule:
    - cron: "0 0 * * * * *" # Run daily at midnight

jobs:
  example-job:
    runs-on: bash
    steps:
      - run: echo "This is a test"
  python-job:
    runs-on:
      python-external: .venv/bin/python
    steps:
      - run: func.py
```

Learn more about syntax and how it compares to Github Actions in [SYNTAX.md](SYNTAX.md).

## Command Reference

### Workflow Management

- Add a workflow:

```bash
jikanctl add [namespace] [name] [body]
```

- List workflows:

```bash
jikanctl list [namespace]
```

- Get workflow details:

```bash
jikanctl get [namespace] [name]
```

- Delete a workflow:

```bash
jikanctl delete [namespace] [name]
```

- Run a workflow:

```bash
jikanctl run [namespace] [name]
```

### Namespace Management

- Add a namespace:

```bash
jikanctl add_namespace [name] [path]
```

- List namespaces:

```bash
jikanctl list_namespaces
```

- Delete a namespace:

```bash
jikanctl delete_namespace [name]
```

### Environment Variables

- Set an environment variable:

```bash
jikanctl set_env [namespace] [name] [key] [value]
```

- Get an environment variable:

```bash
jikanctl get_env [namespace] [name] [key]
```

- List environment variables:

```bash
jikanctl list_env [namespace] [name]
```

## Advanced Usage

- Registering directory-based workflows:

```bash
jikanctl register_dir [namespace] [dir_path]
```

- Checking the next scheduled run:

````bash
jikanctl next [namespace] [name]
```

## Troubleshooting

- If Jikan fails to start, check the log file at `~/.jikan/jikan.log`.
- Ensure all required dependencies are installed and up-to-date.
- Verify that your workflow YAML files are correctly formatted.

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](CONTRIBUTING.md) for more information on how to get started.

## License

Jikan is released under the [MIT License](LICENSE).
````
