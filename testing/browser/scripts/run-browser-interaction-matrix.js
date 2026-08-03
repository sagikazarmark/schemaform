const fs = require("node:fs");
const crypto = require("node:crypto");
const path = require("node:path");
const { chromium, firefox, webkit } = require("playwright");
const axePath = require.resolve("axe-core/axe.min.js");

const root = path.resolve(__dirname, "../../..");
const interactionManifest = JSON.parse(
  fs.readFileSync(
    path.join(root, "testing/browser/workload-pack/interaction-manifest.json"),
  ),
);
const runnerManifest = JSON.parse(
  fs.readFileSync(
    path.join(root, "testing/browser/workload-pack/runner-manifest.json"),
  ),
);
const url = process.env.WASM_BINDGEN_TEST_URL ?? "http://127.0.0.1:8000";
const output = path.resolve(
  process.env.INTERACTION_OBSERVATION ??
    path.join(root, "testing/browser/artifacts/interaction-observation.json"),
);
const browserTypes = { chromium, firefox, webkit };
const requestedCell = process.env.INTERACTION_CELL;
const requiredTraces = [
  ...new Set([
    ...interactionManifest.scenarios.flatMap(({ traces }) => traces),
    ...interactionManifest.accessibility.checkpoints.map(({ trace }) => trace),
  ]),
].sort();

function verifyPins(name, browserType, browser) {
  const expected = runnerManifest.browsers[name];
  const actualVersion = browser.version();
  if (actualVersion !== expected.version) {
    throw new Error(`${name} version ${actualVersion}, expected ${expected.version}`);
  }
  const executable = browserType.executablePath().replaceAll("\\", "/");
  if (!executable.includes(`-${expected.revision}/`)) {
    throw new Error(
      `${name} executable ${executable} is not pinned revision ${expected.revision}`,
    );
  }
}

async function runCell(browser, cell) {
  const scale = cell.zoom_percent / 100;
  const context = await browser.newContext({
    viewport: { width: cell.viewport_width_css_pixels, height: 900 },
  });
  await context.addInitScript({ path: axePath });
  await context.addInitScript(({ zoom }) => {
    const roots = new WeakSet();
    window.__dynamicFormsInteractionZoom = {
      zoom,
      mountedRoots: 0,
      verifiedRoots: 0,
      invalidRoots: 0,
    };
    const accessibility = [];
    const rawAccessibility = [];
    const checkpointIds = new Set();
    window.__dynamicFormsAccessibilityResults = accessibility;
    window.__dynamicFormsRawAccessibilityResults = rawAccessibility;
    window.__dynamicFormsAccessibilityCheckpoint = async (id, trace, root) => {
      if (checkpointIds.has(id)) {
        throw new Error(`duplicate accessibility checkpoint ${id}`);
      }
      checkpointIds.add(id);
      root.setAttribute("data-schemaform-accessibility-checkpoint", id);
      try {
        const [result, ariaSnapshot] = await Promise.all([
          window.axe.run(root, { resultTypes: ["violations"] }),
          window.__dynamicFormsCaptureAccessibility(id),
        ]);
        accessibility.push({
          id,
          trace,
          aria_snapshot: ariaSnapshot,
          violations: result.violations.map((violation) => ({
            rule_id: violation.id,
            impact: violation.impact ?? "unknown",
            nodes: violation.nodes.length,
            targets: violation.nodes
              .flatMap(({ target }) => target.map(String))
              .sort(),
          })),
        });
        rawAccessibility.push({ id, trace, aria_snapshot: ariaSnapshot, report: result });
      } finally {
        root.removeAttribute("data-schemaform-accessibility-checkpoint");
      }
    };
    const applyZoom = () => {
      for (const root of document.querySelectorAll("body > div")) {
        if (!roots.has(root)) {
          roots.add(root);
          root.style.zoom = String(zoom);
          window.__dynamicFormsInteractionZoom.mountedRoots += 1;
        }
      }
    };
    const remove = Element.prototype.remove;
    Element.prototype.remove = function () {
      if (roots.has(this)) {
        const computedZoom = Number.parseFloat(getComputedStyle(this).zoom);
        if (computedZoom === zoom) {
          window.__dynamicFormsInteractionZoom.verifiedRoots += 1;
        } else {
          window.__dynamicFormsInteractionZoom.invalidRoots += 1;
        }
      }
      return remove.call(this);
    };
    applyZoom();
    new MutationObserver(applyZoom).observe(document, {
      childList: true,
      subtree: true,
    });
  }, { zoom: scale });

  await context.tracing.start({ screenshots: true, snapshots: true, sources: true });
  const page = await context.newPage();
  const browserLog = [];
  page.on("console", (message) =>
    browserLog.push({ type: message.type(), text: message.text() }),
  );
  page.on("pageerror", (error) =>
    browserLog.push({ type: "pageerror", text: error.stack ?? error.message }),
  );
  await page.exposeFunction("__dynamicFormsCaptureAccessibility", async (id) =>
    page
      .locator(`[data-schemaform-accessibility-checkpoint="${id}"]`)
      .ariaSnapshot(),
  );
  const artifactDirectory = path.join(
    root,
    "testing/browser/artifacts/interactions",
    cell.id,
  );
  fs.mkdirSync(artifactDirectory, { recursive: true });
  const artifactFiles = [
    ["trace", path.join(artifactDirectory, "trace.zip")],
    ["screenshot", path.join(artifactDirectory, "screenshot.png")],
    ["accessibility-report", path.join(artifactDirectory, "accessibility.json")],
    ["browser-log", path.join(artifactDirectory, "browser.log")],
  ];
  let text = "";
  let traceStopped = false;
  try {
    await page.goto(`${url}?interaction-cell=${encodeURIComponent(cell.id)}`);
    await page.waitForFunction(
      () =>
        document.body.textContent.includes("test result: ok") ||
        document.body.textContent.includes("test result: FAILED"),
      undefined,
      { timeout: 300_000 },
    );
    text = await page.locator("body").innerText();
    const dimensions = await page.evaluate(() => ({
      innerWidth: window.innerWidth,
      ...window.__dynamicFormsInteractionZoom,
      accessibility: window.__dynamicFormsAccessibilityResults,
      rawAccessibility: window.__dynamicFormsRawAccessibilityResults,
    }));
    const passingTests = new Set(
      text
        .split("\n")
        .map((line) => line.trim().match(/^test ([a-z0-9_]+) \.\.\. ok$/)?.[1])
        .filter(Boolean),
    );
    const traces = requiredTraces.filter((trace) => passingTests.has(trace));
    const passedCount = Number.parseInt(
      text.match(/test result: ok\. ([0-9]+) passed;/)?.[1] ?? "-1",
      10,
    );
    const passed =
      text.includes("test result: ok") &&
      dimensions.zoom === scale &&
      dimensions.mountedRoots === passedCount &&
      dimensions.verifiedRoots === passedCount &&
      dimensions.invalidRoots === 0 &&
      traces.length === requiredTraces.length;
    if (!passed) {
      const lines = text.split("\n");
      const diagnosticIndexes = lines
        .map((line, index) =>
          /FAILED|panicked|failures:|test result:/.test(line) ? index : -1,
        )
        .filter((index) => index >= 0);
      const diagnostics = [
        ...new Set(
          diagnosticIndexes.flatMap((index) =>
            lines.slice(Math.max(0, index - 2), index + 3),
          ),
        ),
      ];
      console.error(`${cell.id} failed`, dimensions, diagnostics.join("\n"));
    }
    await page.screenshot({ path: artifactFiles[1][1], fullPage: true });
    fs.writeFileSync(
      artifactFiles[2][1],
      `${JSON.stringify(dimensions.rawAccessibility, null, 2)}\n`,
    );
    fs.writeFileSync(artifactFiles[3][1], `${JSON.stringify(browserLog, null, 2)}\n`);
    await context.tracing.stop({ path: artifactFiles[0][1] });
    traceStopped = true;
    const artifacts = artifactFiles.map(([kind, file]) => {
      const bytes = fs.readFileSync(file);
      return {
        kind,
        path: path.relative(root, file).split(path.sep).join("/"),
        sha256: crypto.createHash("sha256").update(bytes).digest("hex"),
        bytes: bytes.length,
      };
    });
    return {
      browser: cell.browser,
      viewport_width_css_pixels: cell.viewport_width_css_pixels,
      zoom_percent: cell.zoom_percent,
      effective_viewport_width_css_pixels: dimensions.innerWidth,
      status: passed ? "passed" : "failed",
      traces,
      accessibility: dimensions.accessibility,
      artifacts,
    };
  } finally {
    if (!traceStopped) {
      await page.screenshot({ path: artifactFiles[1][1], fullPage: true }).catch(() => {});
      const rawAccessibility = await page
        .evaluate(() => window.__dynamicFormsRawAccessibilityResults ?? [])
        .catch(() => []);
      fs.writeFileSync(
        artifactFiles[2][1],
        `${JSON.stringify(rawAccessibility, null, 2)}\n`,
      );
      fs.writeFileSync(
        artifactFiles[3][1],
        `${JSON.stringify(browserLog, null, 2)}\n`,
      );
      await context.tracing.stop({ path: artifactFiles[0][1] }).catch(() => {});
    }
    await context.close();
  }
}

(async () => {
  const playwrightVersion = require("playwright/package.json").version;
  if (playwrightVersion !== runnerManifest.browsers.playwright) {
    throw new Error(
      `Playwright version ${playwrightVersion}, expected ${runnerManifest.browsers.playwright}`,
    );
  }
  const axeVersion = require("axe-core/package.json").version;
  if (axeVersion !== interactionManifest.accessibility.version) {
    throw new Error(
      `axe-core version ${axeVersion}, expected ${interactionManifest.accessibility.version}`,
    );
  }

  const observation = {
    version: interactionManifest.version,
    workflow_run_attempt: Number(process.env.QUALIFICATION_ATTEMPT ?? "1"),
    cells: [],
  };
  for (const name of interactionManifest.browsers) {
    const browserType = browserTypes[name];
    const browser = await browserType.launch({ headless: true });
    try {
      verifyPins(name, browserType, browser);
      for (const cell of interactionManifest.cells.filter(
        ({ browser: cellBrowser, id }) =>
          cellBrowser === name && (!requestedCell || id === requestedCell),
      )) {
        process.stdout.write(`Running ${cell.id}\n`);
        observation.cells.push(await runCell(browser, cell));
      }
    } finally {
      await browser.close();
    }
  }

  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(observation, null, 2)}\n`);
  if (observation.cells.some(({ status }) => status !== "passed")) {
    process.exitCode = 1;
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
