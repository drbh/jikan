# Jikan (時間): Simplified Workflow Execution

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
   cargo install --git https://github.com/drbh/jikan.git
   cargo install --path /Users/drbh/Projects/jikan/jikan
   cargo install --path /Users/drbh/Projects/jikan/jikanctl
   ```

2. Start the Jikan daemon:

   ```bash
   jikan
   # or run in the background
   jikan daemon
   ```

3. Create a simple workflow file (e.g., `workflows/hello-world.yaml`):

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

4. Add the workflow:

   ```bash
   jikanctl REGISTER_DIR test-repo .bak/test-repo
   jikanctl ADD_NAMESPACE test-repo .bak/test-repo
   ```

5. List your workflows:
   ```bash
   jikanctl LIST test-repo
   ```

6. See the next scheduled run:
   ```bash
   jikanctl NEXT test-repo example-workflow
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
   jikanctl COMMAND [ARGUMENTS]
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

## Command Reference

### Workflow Management

- Add a workflow:

  ```bash
  jikanctl ADD [namespace] [name] [body]
  ```

- List workflows:

  ```bash
  jikanctl LIST [namespace]
  ```

- Get workflow details:

  ```bash
  jikanctl GET [namespace] [name]
  ```

- Delete a workflow:

  ```bash
  jikanctl DELETE [namespace] [name]
  ```

- Run a workflow:
  ```bash
  jikanctl RUN [namespace] [name]
  ```

### Namespace Management

- Add a namespace:

  ```bash
  jikanctl ADD_NAMESPACE [name] [path]
  ```

- List namespaces:

  ```bash
  jikanctl LIST_NAMESPACES
  ```

- Delete a namespace:
  ```bash
  jikanctl DELETE_NAMESPACE [name]
  ```

### Environment Variables

- Set an environment variable:

  ```bash
  jikanctl SET_ENV [namespace] [name] [key] [value]
  ```

- Get an environment variable:

  ```bash
  jikanctl GET_ENV [namespace] [name] [key]
  ```

- List environment variables:
  ```bash
  jikanctl LIST_ENV [namespace] [name]
  ```

## Advanced Usage

- Registering directory-based workflows:

  ```bash
  jikanctl REGISTER_DIR [namespace] [dir_path]
  ```

- Checking the next scheduled run:
  ```bash
  jikanctl NEXT [namespace] [name]
  ```

## Troubleshooting

- If Jikan fails to start, check the log file at `~/.jikan/jikan.log`.
- Ensure all required dependencies are installed and up-to-date.
- Verify that your workflow YAML files are correctly formatted.

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](CONTRIBUTING.md) for more information on how to get started.

## License

Jikan is released under the [MIT License](LICENSE).
