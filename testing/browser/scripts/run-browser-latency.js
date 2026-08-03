const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");
const {
  mountWorkload,
  readJson,
  readScenario,
  readyPage,
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
  process.env.LATENCY_OBSERVATION ??
    path.join(root, "testing/browser/artifacts/latency-observation.json"),
);
const workloadManifest = readJson(workloadManifestPath);
const manifest = readJson(path.join(workloadsRoot, "latency-manifest.json"));
const runnerManifest = readJson(runnerManifestPath);
const runnerObservation = readJson(runnerObservationPath);
const interactionObservation = readJson(interactionObservationPath);
const wasmPath = path.join(artifactRoot, "browser_workload_runner_bg.wasm");
let actualRunnerObservation;

function nearestRank(samples, percentile) {
  const ordered = [...samples].sort((left, right) => left - right);
  return ordered[Math.ceil((ordered.length * percentile) / 100) - 1];
}

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

async function coldSample(browser, url, metric, contextIndex) {
  const context = await browser.newContext();
  try {
    const page = await readyPage(context, url, metric.scenario);
    const scenario = readScenario(workloadsRoot, workloadManifest, metric.scenario);
    const duration = await page.evaluate(async ({ workload, serializedScenario }) => {
      const runner = window.__dynamicFormsWorkload;
      if (workload === "compilation") {
        const invocation = runner.compile_workload(serializedScenario);
        return invocation.finished_at_ms - invocation.started_at_ms;
      }
      runner.prepare_mount_workload(serializedScenario);
      const committed = new Promise((resolve) => {
        const observer = new MutationObserver(() => {
          const sentinel = document.querySelector("#workload-commit-sentinel");
          if (sentinel) {
            observer.disconnect();
            resolve(performance.now());
          }
        });
        observer.observe(document.documentElement, { childList: true, subtree: true });
      });
      const started = runner.mount_workload();
      const committedAt = await committed;
      return committedAt - started;
    }, { workload: metric.workload, serializedScenario: JSON.stringify(scenario) });
    return {
      context: contextIndex,
      fresh_context: true,
      sample: { sequence: 0, phase: 0, duration_ms: duration },
    };
  } finally {
    await context.close();
  }
}

async function invokeHot(page, workload, phase, recipe) {
  if (workload === "edit") {
    return page.evaluate(async ({ binding, value }) => {
      const sentinel = document.querySelector("#workload-commit-sentinel");
      const input = [...document.querySelectorAll("input")].find(
        (candidate) => candidate.name === binding,
      );
      if (!sentinel || !input) {
        throw new Error("edit workload DOM boundary is not mounted");
      }
      const previousRevision = sentinel.getAttribute("data-workload-state-revision");
      let committedAt;
      let resolveCommit;
      const committed = new Promise((resolve) => {
        resolveCommit = resolve;
      });
      const observer = new MutationObserver(() => {
        if (
          sentinel.getAttribute("data-workload-state-revision") !== previousRevision
        ) {
          committedAt = performance.now();
          observer.disconnect();
          resolveCommit();
        }
      });
      observer.observe(sentinel, {
        attributes: true,
        attributeFilter: ["data-workload-state-revision"],
      });
      const valueSetter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      ).set;
      valueSetter.call(input, value);
      let startedAt;
      input.addEventListener(
        "input",
        () => {
          startedAt = performance.now();
        },
        { capture: true, once: true },
      );
      input.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          data: value,
          inputType: "insertText",
        }),
      );
      if (sentinel.getAttribute("data-workload-state-revision") !== previousRevision) {
        committedAt = performance.now();
        observer.disconnect();
        resolveCommit();
      }
      await committed;
      return committedAt - startedAt;
    }, { binding: recipe.binding, value: recipe.alternating_values[phase] });
  }
  return page.evaluate(async ({ workloadName, samplePhase }) => {
    const sentinel = document.querySelector("#workload-commit-sentinel");
    if (!sentinel) {
      throw new Error("workload commit sentinel is not mounted");
    }
    let committedAt;
    let resolveCommit;
    const committed = new Promise((resolve) => {
      resolveCommit = resolve;
    });
    const observer = new MutationObserver(() => {
      if (
        sentinel.getAttribute("data-workload-commit") === String(invocation.commit_token)
      ) {
        committedAt = performance.now();
        observer.disconnect();
        resolveCommit();
      }
    });
    observer.observe(sentinel, {
      attributes: true,
      attributeFilter: ["data-workload-commit"],
    });
    const invocation = window.__dynamicFormsWorkload.run_workload(
      workloadName,
      samplePhase,
    );
    if (sentinel.getAttribute("data-workload-commit") === String(invocation.commit_token)) {
      committedAt = performance.now();
      observer.disconnect();
      resolveCommit();
    }
    await committed;
    return committedAt - invocation.started_at_ms;
  }, { workloadName: workload, samplePhase: phase });
}

async function hotProcess(url, metric, scenario, recipe, processIndex) {
  const browser = await chromium.launch({ headless: true });
  try {
    verifyChromiumPin(browser, chromium, runnerManifest);
    const context = await browser.newContext();
    try {
      const page = await readyPage(context, url, metric.scenario);
      await mountWorkload(page, scenario);
      for (let warmup = 0; warmup < manifest.protocol.warmups_per_process; warmup += 1) {
        await invokeHot(page, metric.workload, warmup % metric.phases, recipe);
      }
      const samples = [];
      for (let sequence = 0; sequence < manifest.protocol.samples_per_process; sequence += 1) {
        const phase = sequence % metric.phases;
        samples.push({
          sequence,
          phase,
          duration_ms: await invokeHot(page, metric.workload, phase, recipe),
        });
      }
      return {
        process: processIndex,
        fresh_process: true,
        warmups_completed: manifest.protocol.warmups_per_process,
        samples,
      };
    } finally {
      await context.close();
    }
  } finally {
    await browser.close();
  }
}

async function runMetric(coldBrowser, url, metric) {
  process.stdout.write(`Running ${metric.id}\n`);
  if (metric.cold) {
    const contexts = [];
    for (let index = 0; index < manifest.protocol.cold_context_samples; index += 1) {
      contexts.push(await coldSample(coldBrowser, url, metric, index));
    }
    const samples = contexts.map(({ sample }) => sample.duration_ms);
    return {
      id: metric.id,
      p50_ms: nearestRank(samples, 50),
      p95_ms: nearestRank(samples, 95),
      p99_ms: nearestRank(samples, 99),
      runs: { kind: "cold", contexts },
    };
  }
  const scenario = readScenario(workloadsRoot, workloadManifest, metric.scenario);
  const recipe = scenario.workloads.find(({ workload }) => workload === metric.workload);
  if (!recipe) {
    throw new Error(`${metric.scenario} does not define ${metric.workload}`);
  }
  const processes = [];
  for (let index = 0; index < manifest.protocol.hot_processes; index += 1) {
    processes.push(await hotProcess(url, metric, scenario, recipe, index));
  }
  const samples = processes.flatMap((process) =>
    process.samples.map(({ duration_ms: duration }) => duration),
  );
  return {
    id: metric.id,
    p50_ms: nearestRank(samples, 50),
    p95_ms: nearestRank(samples, 95),
    p99_ms: nearestRank(samples, 99),
    runs: { kind: "hot", processes },
  };
}

(async () => {
  await verifyPreflight();
  if (!fs.existsSync(wasmPath)) {
    throw new Error(`production WASM artifact does not exist: ${wasmPath}`);
  }
  const { server, url } = await startWorkloadServer({
    artifactRoot,
    title: "Schemaform latency runner",
  });
  let coldBrowser;
  try {
    coldBrowser = await chromium.launch({ headless: true });
    verifyChromiumPin(coldBrowser, chromium, runnerManifest);
    const metrics = [];
    for (const metric of manifest.metrics) {
      metrics.push(await runMetric(coldBrowser, url, metric));
    }
    const observation = {
      version: manifest.version,
      workflow_run_attempt: Number(process.env.QUALIFICATION_ATTEMPT ?? "1"),
      runner_observation: actualRunnerObservation,
      workload_manifest_sha256: sha256File(workloadManifestPath),
      production_artifact_sha256: sha256File(wasmPath),
      environment_sanity_passed: true,
      discretionary_retries: 0,
      outliers_removed: 0,
      interaction: interactionObservation,
      metrics,
    };
    fs.mkdirSync(path.dirname(outputPath), { recursive: true });
    fs.writeFileSync(outputPath, `${JSON.stringify(observation, null, 2)}\n`);
  } finally {
    if (coldBrowser) {
      await coldBrowser.close();
    }
    await new Promise((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
