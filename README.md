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
cargo install --git https://github.com/drbh/jikan.git jk
```

2. Start the Jikan daemon:

```bash
jikand
# or run in the background
jikand daemon
# and to stop the daemon
jikand stop
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
jk init
# Server response: Registered 1 workflows in namespace 'daily-brief'.
```

5. List your workflows:

```bash
jk list daily-brief
# Server response: Workflows:
# - daily-brief (namespace: daily-brief)
```

6. See the next scheduled run:

```bash
jk next daily-brief daily-brief
# Server response: Workflow 'daily-brief' in namespace 'daily-brief' will run in 1239 seconds.
# Absolute time: Sun, 25 Aug 2024 14:07:00 -0400
```

7. Run the workflow manually:

```bash
jk run daily-brief daily-brief
# Server response: Workflow 'daily-brief' in namespace 'daily-brief' ran successfully.
```

after running the workflow, you should see the output in the `daily_brief.md` file:

```bash
cat ./daily_brief.md
# Good morning! Here's your daily brief:
# Sunday, August 25, 2024
# ## Today's Weather
# jfk

#      \  /       Partly cloudy
#    _ /"".-.     +23(25) °C
#      \_(   ).   ↑ 19 km/h
#      /(___(__)  16 km
#                 0.0 mm
```

🙌 awesome, you just registered and ran your first Jikan workflow!

## Core Concepts

- **Workflows**: Defined tasks that Jikan executes based on triggers.
- **Namespaces**: Logical groupings for organizing workflows.
- **Jobs**: Individual units of work within a workflow.
- **Triggers**: Events that initiate workflow execution (e.g., schedules, manual runs).

## Basic Usage

1. Start the Jikan daemon:

```bash
jk --help
```

2. Use the `jk` tool to interact with Jikan:

```bash
jk command [arguments]
```

## Workflow Configuration

Create YAML files to define your workflows. Example:

```yaml
name: my-workflow
run-name: some-run-name
on:
  schedule:
    - cron: "0 0 0 * * * *" # Run daily at midnight

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

- List workflows:

```bash
jk list [namespace]
```

- Get workflow details:

```bash
jk get [namespace] [name]
```

- Run a workflow:

```bash
jk run [namespace] [name]
```

- List namespaces:

```bash
jk list_namespaces
```

### Environment Variables

- Set an environment variable:

```bash
jk set_env [namespace] [name] [key] [value]
```

- Get an environment variable:

```bash
jk get_env [namespace] [name] [key]
```

- List environment variables:

```bash
jk list_env [namespace] [name]
```

- Checking the next scheduled run:

```bash
jk next [namespace] [name]
```

- View last Run:

```bash
jk last [namespace] [name]
```

## Troubleshooting

- If Jikan fails to start, check the log file at `~/.jikan/jikan.log`.
- Ensure all required dependencies are installed and up-to-date.
- Verify that your workflow YAML files are correctly formatted.

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](CONTRIBUTING.md) for more information on how to get started.

## License

Jikan is released under the [MIT License](LICENSE).
