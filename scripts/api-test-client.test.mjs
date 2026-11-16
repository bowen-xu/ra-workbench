import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {test} from "node:test";
import {launchWorkbenchApiTestHost} from "../preview-host/api-test-client.mjs";

async function fixture(run, version = "0.8.0") {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "workbench-api-client-"));
  const executable = path.join(directory, "host.mjs");
  await fs.writeFile(executable, `#!/usr/bin/env node
import {createInterface} from "node:readline";
import fs from "node:fs";
const reply = value => process.stdout.write(JSON.stringify(value) + "\\n");
reply({jsonrpc:"2.0",method:"workbench.ready",params:{protocolVersion:1,version:${JSON.stringify(version)}}});
const input = createInterface({input:process.stdin});
let held;
input.on("line", line => {
  const request = JSON.parse(line);
  const result = value => reply({jsonrpc:"2.0",id:request.id,result:value});
  if (request.method === "held") {held = request;return;}
  if (request.method === "release") {
    result("second");reply({jsonrpc:"2.0",id:held.id,result:"first"});return;
  }
  if (request.method === "reject") {
    reply({jsonrpc:"2.0",id:request.id,error:{code:-32000,message:"original",data:{code:"fixture_rejected",requestId:"original-request",httpStatus:409}}});return;
  }
  if (request.method === "configuration") {
    result({credentialPath:process.env.RESEARCH_ASSISTANT_CREDENTIALS_PATH});return;
  }
  if (request.method === "malformed") {
    reply({jsonrpc:"2.0",id:request.id,error:null});return;
  }
  result({arguments:request.params,plugins:fs.readdirSync(process.env.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY)});
});
input.on("close", () => process.exit(0));
`, {mode: 0o755});
  try {await run({executable, dataDirectory: path.join(directory, "data"), expectedVersion: "0.8.0"});}
  finally {await fs.rm(directory, {recursive: true, force: true});}
}

test("the public API client preserves independent responses, native errors and zero-plugin selection", () => fixture(async options => {
  const host = await launchWorkbenchApiTestHost(options);
  try {
    const first = host.invoke("held");
    const second = host.invoke("release");
    assert.deepEqual(await Promise.all([first, second]), ["first", "second"]);
    await assert.rejects(host.invoke("reject"), cause => cause.code === "fixture_rejected"
      && cause.requestId === "original-request" && cause.httpStatus === 409);
    assert.deepEqual(await host.invoke("echo", {id: "synthetic"}), {arguments: {id: "synthetic"}, plugins: []});
    assert.deepEqual(await host.invoke("configuration"), {credentialPath: path.join(options.dataDirectory, "credentials.json")});
  } finally {
    const closing = host.close();
    assert.equal(host.close(), closing);
    await closing;
  }
}));

test("a malformed error response rejects the request and closes the failed host", () => fixture(async options => {
  const host = await launchWorkbenchApiTestHost(options);
  await assert.rejects(host.invoke("malformed"), /workbench_api_test_host_response_invalid/u);
  await assert.rejects(host.close(), /workbench_api_test_host_response_invalid/u);
}));

test("a public host consumer rejects an incompatible Workbench version", () => fixture(async options => {
  await assert.rejects(launchWorkbenchApiTestHost(options), /workbench_api_test_host_version_mismatch/u);
}, "0.9.0"));

test("an unencodable request leaves the host usable and closable", () => fixture(async options => {
  const host = await launchWorkbenchApiTestHost(options);
  try {
    const arguments_ = {};
    arguments_.self = arguments_;
    await assert.rejects(host.invoke("echo", arguments_), /circular/u);
    assert.deepEqual(await host.invoke("echo"), {arguments: {}, plugins: []});
  } finally {await host.close();}
}));
