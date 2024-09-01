const core = {
  // https://github.com/actions/toolkit/blob/main/packages/core/src/core.ts#L126
  getInput: (name) => {
    // convert dash to underscore and uppercase
    name = `INPUT_${name.replaceAll("-", "_").toUpperCase()}`;
    return process.env[name];
  },
  // https://github.com/actions/toolkit/blob/main/packages/core/src/core.ts#L192
  setOutput: (name, value) => {
    console.log(`::set-output ${name}=${value}`);
  },
  setFailed: (message) => {
    console.error(message);
    process.exit(1);
  },
};

const github = {
  context: {
    payload: JSON.parse(process.env.GITHUB_EVENT_PAYLOAD),
  },
};

try {
  const nameToGreet = core.getInput("who-to-greet");
  console.log(`Hello ${nameToGreet}!`);
  const time = new Date().toTimeString();
  core.setOutput("time", time);
  // Get the JSON webhook payload for the event that triggered the workflow
  const payload = JSON.stringify(github.context.payload, undefined, 2);
  console.log(`The event payload: ${payload}`);
  //   throw new Error("This is an error");
} catch (error) {
  core.setFailed(error.message);
}
