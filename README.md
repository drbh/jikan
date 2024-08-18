# 時間 (jikan)

> NOTE: This project is a work in progress and is not yet ready for use.

jikan is a workflow execution daemon inspired by GitHub Actions, offering more features than cron while being simpler than traditional workflow engines and CI/CD pipelines.


## Example usage

first define a simple workflow file, or use the example in `workflows/workflow-now.yaml`.

a workflow could look like this, where the `on` section defines when the workflow should run, and the `jobs` section defines the jobs that should run.

- currently there are a limited set of actions that can be run, but more will be added in the future.

```yaml
name: workflow-now
run-name: some-run-name
on:
  schedule:
    # second minute hour day month * *
    - cron: "0 50 18 18 8 * *"

jobs:
  some-job:
    runs-on: bash
    steps:
      - run: echo "This is a test"
  python-job:
    runs-on:
      python-external: .venv/bin/python
    steps:
      - run: func.py
```

once a workflow is defined, we can add it to the daemon to be executed. adding, removing, and listing workflows can be done with a simple TCP call to the running daemon.

it's recommended to interact using the `jikanctl` command line tool (20 line bash script) that is included in the repository.

```bash
./jikanctl ADD workflow-now
# Server response: Workflow 'workflow-now' added successfully.

./jikanctl LIST
# Server response: Workflows:
# - workflow-now

./jikanctl DELETE workflow-now
# Server response: Workflow 'workflow-now' deleted successfully.
```

### Running jikan

```bash
cargo run
# 2024-08-18T18:16:35.681904Z  INFO jikan: Starting Jikan application
# 2024-08-18T18:16:35.738946Z  INFO hydrate_scheduler: jikan: Finished hydrating scheduler hydrated_jobs=0
# 2024-08-18T18:16:35.739127Z  INFO start_server_and_scheduler: jikan: Server listening on port 8080
```

## Ideas for future features

- db and global workflows moved to global location `~/.jikan`
- should support concept of repo/directory based workflows (like GitHub Actions)
- inject global environment variables into all jobs and specialized keywords
- more informative api (upcoming runs, etc)
- add concept of logging and output capture/historical runs
