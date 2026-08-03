const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");
const {
  mountWorkload,
  readJson,
  readScenario,
  sha256File,
  startWorkloadServer,
  verifyChromiumPin,
  verifyMeasurementPreflight,
} = require("./browser-workload-support");

const root = path.resolve(__dirname, "../../..");
const workloadsRoot = path.join(root, "testing/browser/workload-pack");
const artifactRoot = path.join(root, "testing/browser/artifacts/bindgen");
const workloadManifestPath = path.join(workloadsRoot, "manifest.json");
const runnerManifestPath = path.join(workloadsRoot, "runner-manifest.json");
const runnerObservationPath = path.resolve(
  process.env.RUNNER_OBSERVATION ??
    path.join(root, "testing/browser/artifacts/runner-observation.json"),
);
const interactionObservationPath = path.resolve(
  process.env.INTERACTION_OBSERVATION ??
    path.join(root, "testing/browser/artifacts/interaction-observation.json"),
);
const outputPath = path.resolve(
  process.env.RESOURCE_OBSERVATION ??
    path.join(root, "testing/browser/artifacts/resource-observation.json"),
);
const manifest = readJson(path.join(workloadsRoot, "resource-manifest.json"));
const workloadManifest = readJson(workloadManifestPath);
const runnerManifest = readJson(runnerManifestPath);
const runnerObservation = readJson(runnerObservationPath);
let actualRunnerObservation;

async function verifyPreflight() {
  actualRunnerObservation = await verifyMeasurementPreflight({
    root,
    workloadManifestPath,
    runnerManifestPath,
    contractManifest: manifest,
    runnerManifest,
    runnerObservation,
    runnerObservationPath,
    interactionObservationPath,
  });
}

async function settlePage(page) {
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  );
}

async function collectHeap(page, cdp) {
  await settlePage(page);
  await cdp.send("HeapProfiler.collectGarbage");
  await settlePage(page);
  const { metrics } = await cdp.send("Performance.getMetrics");
  const heap = metrics.find(({ name }) => name === "JSHeapUsedSize")?.value;
  if (!Number.isSafeInteger(heap) || heap < 0) {
    throw new Error(`Chromium did not report an integral JSHeapUsedSize: ${heap}`);
  }
  return heap;
}

async function wasmMemory(page) {
  const bytes = await page.evaluate(() => window.__dynamicFormsWasmMemory?.buffer.byteLength);
  if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes % 65536 !== 0) {
    throw new Error(`WASM linear memory is unavailable or invalid: ${bytes}`);
  }
  return bytes;
}

async function invoke(page, workload, phase) {
  await page.evaluate(async ({ workloadName, samplePhase }) => {
    const sentinel = document.querySelector("#workload-commit-sentinel");
    if (!sentinel) {
      throw new Error("workload commit sentinel is not mounted");
    }
    let invocation;
    const committed = new Promise((resolve) => {
      const observer = new MutationObserver(() => {
        if (
          invocation &&
          sentinel.getAttribute("data-workload-commit") === String(invocation.commit_token)
        ) {
          observer.disconnect();
          resolve();
        }
      });
      observer.observe(sentinel, {
        attributes: true,
        attributeFilter: ["data-workload-commit"],
      });
      invocation = window.__dynamicFormsWorkload.run_workload(
        workloadName,
        samplePhase,
      );
      if (sentinel.getAttribute("data-workload-commit") === String(invocation.commit_token)) {
        observer.disconnect();
        resolve();
      }
    });
    await committed;
  }, { workloadName: workload, samplePhase: phase });
}

function artifactObservations() {
  return manifest.artifacts.map(({ id, path: relative }) => {
    const file = path.join(root, relative);
    const bytes = fs.readFileSync(file);
    return {
      id,
      path: relative,
      sha256: crypto.createHash("sha256").update(bytes).digest("hex"),
      bytes: bytes.length,
    };
  });
}

function metricObservations(memory, artifacts) {
  const artifact = Object.fromEntries(artifacts.map((value) => [value.id, value]));
  const highWater = Math.max(
    memory.wasm_memory_before_bytes,
    ...memory.operations.map(({ wasm_memory_bytes: bytes }) => bytes),
  );
  return [
    ["wasm-linear-high-water", highWater],
    [
      "browser-heap-post-gc-delta",
      Math.max(
        0,
        memory.browser_heap_after_bytes - memory.browser_heap_before_bytes,
      ),
    ],
    ["brotli-wasm-total", artifact["production-brotli-wasm"].bytes],
    [
      "brotli-wasm-incremental",
      artifact["production-brotli-wasm"].bytes -
        artifact["empty-shell-brotli-wasm"].bytes,
    ],
    [
      "brotli-runtime-javascript-total",
      artifact["production-brotli-javascript"].bytes,
    ],
  ].map(([id, bytes]) => {
    if (!Number.isSafeInteger(bytes) || bytes < 0) {
      throw new Error(`resource metric ${id} is invalid: ${bytes}`);
    }
    return { id, bytes };
  });
}

(async () => {
  await verifyPreflight();
  const artifacts = artifactObservations();
  const { server, url } = await startWorkloadServer({
    artifactRoot,
    title: "Schemaform resource runner",
    exposeMemory: true,
  });
  let browser;
  try {
    browser = await chromium.launch({
      headless: true,
      args: ["--enable-precise-memory-info"],
    });
    verifyChromiumPin(browser, chromium, runnerManifest);
    const context = await browser.newContext();
    try {
      const page = await context.newPage();
      const cdp = await context.newCDPSession(page);
      await cdp.send("Performance.enable");
      await page.goto(`${url}/?scenario=${encodeURIComponent(manifest.memory_protocol.scenario)}`);
      await page.evaluate(() => window.__dynamicFormsWorkloadReady);
      await mountWorkload(
        page,
        readScenario(workloadsRoot, workloadManifest, manifest.memory_protocol.scenario),
      );
      const memory = {
        wasm_memory_before_bytes: await wasmMemory(page),
        browser_heap_before_bytes: await collectHeap(page, cdp),
        operations: [],
      };
      const counts = Object.fromEntries(
        manifest.memory_protocol.operation_cycle.map((workload) => [workload, 0]),
      );
      for (let sequence = 0; sequence < manifest.memory_protocol.operations; sequence += 1) {
        const workload =
          manifest.memory_protocol.operation_cycle[
            sequence % manifest.memory_protocol.operation_cycle.length
          ];
        const phases = manifest.memory_protocol.operation_phases[workload];
        const phase = counts[workload] % phases;
        counts[workload] += 1;
        await invoke(page, workload, phase);
        memory.operations.push({
          sequence,
          workload,
          phase,
          wasm_memory_bytes: await wasmMemory(page),
        });
      }
      memory.browser_heap_after_bytes = await collectHeap(page, cdp);
      const observation = {
        version: manifest.version,
        workflow_run_attempt: Number(process.env.QUALIFICATION_ATTEMPT ?? "1"),
        runner_observation: actualRunnerObservation,
        workload_manifest_sha256: sha256File(workloadManifestPath),
        environment_sanity_passed: true,
        discretionary_retries: 0,
        waivers: [],
        scenario: manifest.memory_protocol.scenario,
        ...memory,
        metrics: metricObservations(memory, artifacts),
        artifacts,
      };
      fs.mkdirSync(path.dirname(outputPath), { recursive: true });
      fs.writeFileSync(outputPath, `${JSON.stringify(observation, null, 2)}\n`);
    } finally {
      await context.close();
    }
  } finally {
    if (browser) {
      await browser.close();
    }
    await new Promise((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
