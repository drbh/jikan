import os
import json
import sys
from datetime import datetime

def get_input(name):
    # convert dash to underscore and uppercase
    name = f"INPUT_{name.replace('-', '_').upper()}"
    return os.getenv(name)


def set_output(name, value):
    print(f"::set-output {name}={value}")


def set_failed(message):
    print(message, file=sys.stderr)
    sys.exit(1)


github = {"context": {"payload": json.loads(os.getenv("GITHUB_EVENT_PAYLOAD"))}}

try:
    name_to_greet = get_input("who-to-greet")
    print(f"Hello {name_to_greet}!")
    time = datetime.now().isoformat()
    set_output("time", time)
    # Get the JSON webhook payload for the event that triggered the workflow
    payload = json.dumps(github["context"]["payload"], indent=2)
    print(f"The event payload: {payload}")
except Exception as e:
    set_failed(e)
