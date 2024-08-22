# Jikan Workflow Syntax

This document outlines the syntax for creating Jikan workflows, including a comparison with GitHub Actions and detailed explanations of each feature.

## Comparison with GitHub Actions

| Feature               | Jikan                             | GitHub Actions                                   |
| --------------------- | --------------------------------- | ------------------------------------------------ |
| Workflow Files        | YAML (.yaml, .yml)                | YAML (.yaml, .yml)                               |
| Workflow Structure    | `name`, `on`, `jobs`              | `name`, `on`, `jobs`                             |
| Triggers              | `schedule` (cron)                 | `schedule`, `push`, `pull_request`, etc.         |
| Jobs                  | Defined under `jobs`              | Defined under `jobs`                             |
| Job Runners           | `runs-on` (bash, python-external) | `runs-on` (ubuntu-latest, windows-latest, etc.)  |
| Steps                 | Defined under `steps`             | Defined under `steps`                            |
| Environment Variables | Set using `jikanctl SET_ENV`      | Set in workflow file or repository settings      |
| Secrets               | Not explicitly supported          | Supported via repository or organization secrets |
| Matrix Builds         | Not supported                     | Supported                                        |
| Service Containers    | Not supported                     | Supported                                        |
| Artifacts             | Not supported                     | Supported                                        |

## Workflow Structure

A basic Jikan workflow file has the following structure:

```yaml
name: workflow-name
run-name: optional-run-name
on:
  schedule:
    - cron: "* * * * * * *"
jobs:
  job-name:
    runs-on: bash
    steps:
      - run: command-to-execute
```

### Name

The `name` field is required and specifies the name of your workflow.

Example:

```yaml
name: daily-backup
```

### Run Name (Optional)

_templating not supported at the moment_

The `run-name` field is optional and can be used to specify a custom name for workflow runs.

Example:

```yaml
run-name: Backup run on ${date}
```

### Triggers (`on`)

Currently, Jikan supports schedule-based triggers using cron syntax.

Example:

```yaml
on:
  schedule:
    - cron: "0 0 * * * * *" # Run daily at midnight
```

The cron syntax in Jikan uses 7 fields: second, minute, hour, day of month, month, day of week, year.

### Jobs

Jobs are the main units of work in a workflow. Each job runs in its own execution environment.

Example:

```yaml
jobs:
  backup-data:
    runs-on: bash
    steps:
      - run: ./backup-script.sh
```

### Job Runners (`runs-on`)

Jikan supports two types of runners:

1. `bash`: Runs commands in a bash shell
2. `python-external`: Runs Python scripts using a specified Python interpreter

Example:

```yaml
jobs:
  python-task:
    runs-on:
      python-external: .venv/bin/python
    steps:
      - run: process_data.py
```

### Steps

Steps are the individual tasks that make up a job. Each step runs in its own process.

Example:

```yaml
steps:
  - run: echo "Step 1: Preparing data"
  - run: ./process_data.sh
  - run: echo "Step 3: Finalizing"
```

## Environment Variables

Jikan supports environment variables at both the workflow and job levels. These can be used to configure the behavior of your workflows and steps.

### Workflow-level Environment Variables

You can define environment variables that will be available to all jobs in the workflow:

```yaml
name: example-workflow
run-name: Example Workflow
env:
  WORKFLOW_ENV: "test"
on:
  schedule:
    - cron: "0 * * * * * *" # every minute
jobs:
  # ... jobs defined here
```

### Job-level Environment Variables

You can also define environment variables specific to a job:

```yaml
jobs:
  example-job:
    runs-on: bash
    env:
      STEP_ENV_NAME: "drbh"
    steps:
      - run: echo $STEP_ENV_NAME
```

### Using Environment Variables

Environment variables can be accessed in your steps using the `$` prefix:

```yaml
steps:
  - run: echo $WORKFLOW_ENV
  - run: echo $STEP_ENV_NAME
```

### Setting Environment Variables via Command Line

In addition to setting environment variables in the workflow file, you can also set them using the `jikanctl` command-line tool:

```bash
jikanctl SET_ENV [namespace] [workflow-name] [key] [value]
```

These variables can then be used in your workflow steps.

## Conditionals

Jikan supports conditional execution of jobs using the `if` keyword. This allows you to control whether a job runs based on certain conditions.

### Job-level Conditionals

You can use the `if` keyword at the job level to conditionally run entire jobs:

```yaml
jobs:
  conditional-job:
    if: "ENV_VAR=some_value"
    runs-on: bash
    steps:
      - run: echo "This job only runs if ENV_VAR equals some_value"
```

In this example, the `conditional-job` will only run if the environment variable `ENV_VAR` is set to "some_value".

### Syntax for Conditionals

The `if` condition supports basic equality checks:

- `ENV_VAR=value`: Checks if an environment variable equals a specific value
- `ENV_VAR!=value`: Checks if an environment variable does not equal a specific value

### Limitations

- Currently, Jikan only supports simple equality checks in conditionals.
- More complex conditions (like greater than, less than, or combining multiple conditions) are not supported.
- Conditionals are only available at the job level, not for individual steps.

### Example Workflow with Conditionals

Here's an example of a workflow using conditionals and environment variables:

```yaml
name: example-workflowcs
run-name: Example Workflow
env:
  WORKFLOW_ENV: "test"
on:
  schedule:
    - cron: "0 * * * * * *" # every minute
jobs:
  conditional-job:
    if: "ENV_VAR=some_value"
    runs-on: bash
    env:
      STEP_ENV_NAME: "drbh"
    steps:
      - run: echo $STEP_ENV_NAME && osascript -e 'display notification "yoyo" with title "hello"'
```

## Advanced Features

### Multiple Jobs

You can define multiple jobs in a single workflow:

```yaml
jobs:
  job1:
    runs-on: bash
    steps:
      - run: echo "This is job 1"
  job2:
    runs-on: bash
    steps:
      - run: echo "This is job 2"
```

## Limitations

- Jikan does not currently support matrix builds, service containers, or artifacts.
- There's no built-in support for secrets management.

Learn more about how to use Jikan in the [README](README.md).
