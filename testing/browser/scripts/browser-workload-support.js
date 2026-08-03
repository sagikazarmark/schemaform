const fs = require("node:fs");
const crypto = require("node:crypto");
const http = require("node:http");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { isDeepStrictEqual } = require("node:util");
const { observeRunner } = require("./observe-browser-runner");

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function readScenario(workloadsRoot, workloadManifest, id) {
  const reference = workloadManifest.scenarios.find((scenario) => scenario.id === id);
  if (!reference) {
    throw new Error(`workload manifest does not define ${id}`);
  }
  const file = path.join(
    workloadsRoot,
    "objects/sha256",
    `${reference.object_sha256}.json`,
  );
  if (sha256File(file) !== reference.object_sha256) {
    throw new Error(`workload object digest mismatch for ${id}`);
  }
  return readJson(file);
}

async function verifyMeasurementPreflight({
  root,
  workloadManifestPath,
  runnerManifestPath,
  contractManifest,
  runnerManifest,
  runnerObservation,
  runnerObservationPath,
  interactionObservationPath,
}) {
  if (sha256File(workloadManifestPath) !== contractManifest.workload_manifest_sha256) {
    throw new Error("workload manifest digest mismatch; comparison aborted before sampling");
  }
  if (sha256File(runnerManifestPath) !== contractManifest.runner_manifest_sha256) {
    throw new Error("runner manifest digest mismatch; comparison aborted before sampling");
  }
  if (!isDeepStrictEqual(runnerManifest, runnerObservation)) {
    throw new Error("runner observation mismatch; comparison aborted before sampling");
  }
  const actualRunnerObservation = await observeRunner();
  if (!isDeepStrictEqual(runnerManifest, actualRunnerObservation)) {
    throw new Error("observed runner mismatch; comparison aborted before sampling");
  }
  if (require("playwright/package.json").version !== runnerManifest.browsers.playwright) {
    throw new Error("Playwright version does not match the runner manifest");
  }
  execFileSync(
    "cargo",
    [
      "run",
      "--locked",
      "-p",
      "browser-workload-pack",
      "--",
      "verify-runner",
      runnerObservationPath,
    ],
    { cwd: root, stdio: "inherit" },
  );
  execFileSync(
    "cargo",
    [
      "run",
      "--locked",
      "-p",
      "browser-workload-pack",
      "--",
      "verify-interactions",
      interactionObservationPath,
    ],
    { cwd: root, stdio: "inherit" },
  );
  return actualRunnerObservation;
}

function verifyChromiumPin(browser, chromium, runnerManifest) {
  const expected = runnerManifest.browsers.chromium;
  if (browser.version() !== expected.version) {
    throw new Error(`Chromium ${browser.version()}, expected ${expected.version}`);
  }
  const executable = chromium.executablePath().replaceAll("\\", "/");
  if (!executable.includes(`-${expected.revision}/`)) {
    throw new Error(
      `Chromium executable ${executable} is not pinned revision ${expected.revision}`,
    );
  }
}

function startWorkloadServer({ artifactRoot, title, exposeMemory = false }) {
  const wasmPath = path.join(artifactRoot, "browser_workload_runner_bg.wasm");
  const files = new Map([
    [
      "/browser_workload_runner.js",
      [path.join(artifactRoot, "browser_workload_runner.js"), "text/javascript"],
    ],
    ["/browser_workload_runner_bg.wasm", [wasmPath, "application/wasm"]],
  ]);
  const html = Buffer.from(`<!doctype html>
<html><head><meta charset="utf-8"><title>${title}</title></head>
<body><div id="main"></div><script type="module">
import init, * as workload from "/browser_workload_runner.js";
window.__dynamicFormsWorkloadReady = (async () => {
  const instance = await init();
  ${exposeMemory ? "window.__dynamicFormsWasmMemory = instance.memory;" : ""}
  window.__dynamicFormsWorkload = workload;
  return true;
})();
</script></body></html>\n`);
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url, "http://127.0.0.1").pathname;
    if (pathname === "/") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(html);
      return;
    }
    const file = files.get(pathname);
    if (!file) {
      response.writeHead(404);
      response.end();
      return;
    }
    response.writeHead(200, { "content-type": file[1] });
    fs.createReadStream(file[0]).pipe(response);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      resolve({ server, url: `http://127.0.0.1:${address.port}` });
    });
  });
}

async function readyPage(context, url, scenario) {
  const page = await context.newPage();
  await page.goto(`${url}/?scenario=${encodeURIComponent(scenario)}`);
  await page.evaluate(() => window.__dynamicFormsWorkloadReady);
  return page;
}

async function mountWorkload(page, scenario) {
  await page.evaluate(async (serializedScenario) => {
    window.__dynamicFormsWorkload.prepare_mount_workload(serializedScenario);
    const committed = new Promise((resolve) => {
      const observer = new MutationObserver(() => {
        if (document.querySelector("#workload-commit-sentinel")) {
          observer.disconnect();
          resolve();
        }
      });
      observer.observe(document.documentElement, { childList: true, subtree: true });
    });
    window.__dynamicFormsWorkload.mount_workload();
    await committed;
  }, JSON.stringify(scenario));
}

module.exports = {
  mountWorkload,
  readJson,
  readScenario,
  readyPage,
  sha256File,
  startWorkloadServer,
  verifyChromiumPin,
  verifyMeasurementPreflight,
};
