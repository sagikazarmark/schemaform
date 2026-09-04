// Drives the demo's daisyUI-rendered pages in a real browser: runs axe-core at
// named checkpoints in the light and dark themes, verifies finding-summary
// focus-to-target and presence repair on the daisyUI-rendered controls of the
// daisyUI form, and add, insert, move, and remove with their focus and
// announcements on the daisyUI-rendered arrays of the arrays page.
//
//   node daisyui.mjs --site <built demo directory>   # serves the bundle itself
//   node daisyui.mjs --url http://127.0.0.1:8080     # against a running server
//
// Exit code 1 when any checkpoint reports a violation, any behavioural
// assertion fails, or the page throws. Artifacts (axe reports, screenshots,
// browser log) are written under --out (default: ./artifacts).

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { createRequire } from "node:module";
import { parseArgs } from "node:util";
import { chromium } from "playwright";

const require = createRequire(import.meta.url);
const axePath = require.resolve("axe-core/axe.min.js");

const THEMES = ["light", "dark"];
const DEFAULT_TIMEOUT_MS = 15_000;
// The landmark the example page renders its live form into.
const AXE_ROOT = '[role="region"][aria-label="Example demo"]';

// --- Static server ----------------------------------------------------------

const CONTENT_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
};

// Serves `root` the way the deployed worker does: files by path, and the
// single-page application's `index.html` for every path that is not a file.
function serveSite(root) {
  const site = path.resolve(root);
  const index = path.join(site, "index.html");
  if (!fs.existsSync(index)) {
    throw new Error(`${site} does not contain index.html`);
  }
  const server = http.createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
    let file = path.normalize(path.join(site, pathname));
    if (file !== site && !file.startsWith(site + path.sep)) {
      response.writeHead(403).end();
      return;
    }
    if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      file = index;
    }
    response.writeHead(200, {
      "content-type": CONTENT_TYPES[path.extname(file)] ?? "application/octet-stream",
      "cache-control": "no-store",
    });
    fs.createReadStream(file).pipe(response);
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({
        url: `http://127.0.0.1:${port}`,
        close: () => new Promise((done) => server.close(done)),
      });
    });
  });
}

// --- Assertions -------------------------------------------------------------

class AssertionFailure extends Error {}

function assert(condition, message) {
  if (!condition) {
    throw new AssertionFailure(message);
  }
}

// Polls `probe` until it returns a truthy value; the page settles
// asynchronously after every core transition, so nothing is asserted
// synchronously.
async function eventually(description, probe, timeout = DEFAULT_TIMEOUT_MS) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    last = await probe();
    if (last) {
      return last;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new AssertionFailure(`timed out waiting for ${description} (last: ${JSON.stringify(last)})`);
}

// --- Page helpers -----------------------------------------------------------

// The live form on the page. The example's source pane stays mounted next to
// it, so every locator is scoped to the form rather than the document.
function form(page) {
  return page.locator("form.schemaform");
}

function tab(page, name) {
  return form(page).getByRole("tab", { name, exact: true });
}

function control(page, pointer) {
  return form(page).locator(`input[name="${pointer}"]`);
}

// An affordance rendered as a button, found by its accessible name (the
// affordance ids depend on bind order, the names do not): a presence or append
// affordance by its localized label, an item affordance by its positional
// accessible name, which the collection renderer puts in `aria-label`.
function affordance(page, name) {
  return form(page).getByRole("button", { name, exact: true });
}

// The daisyUI collection for the array titled `label`: a fieldset named by its
// legend. Item cards are groups too, but named `{item} {position}`.
function collection(page, label) {
  return form(page).getByRole("group", { name: label, exact: true });
}

// Waits for the collection's adapter-owned live region to announce `text`.
async function expectAnnouncement(page, label, text) {
  const status = collection(page, label).locator("[data-array-status]");
  await eventually(`"${text}" to be announced`, async () => (await status.textContent()) === text);
}

// The presence affordances the core currently allows for a control, by label:
// "Set {label}", "Set {label} to null", "Remove {label}", each offered or not.
async function expectAffordances(page, label, { set, setNull, remove }) {
  const expected = [
    [`Set ${label}`, set],
    [`Set ${label} to null`, setNull],
    [`Remove ${label}`, remove],
  ];
  for (const [text, offered] of expected) {
    await eventually(
      `"${text}" to be ${offered ? "offered" : "withdrawn"}`,
      async () => ((await affordance(page, text).count()) > 0) === offered,
    );
  }
}

async function selectTab(page, name) {
  await tab(page, name).click();
  await eventually(
    `the ${name} tab to be selected`,
    async () => (await tab(page, name).getAttribute("aria-selected")) === "true",
  );
}

// The focused element, reduced to what the scenarios assert on: a control's
// binding (`name`), the finding summary, an affordance button's accessible
// name (`label`), and the binding of the first control in the item card the
// element sits in (`card`).
async function activeElement(page) {
  return page.evaluate(() => {
    const active = document.activeElement;
    if (!active) {
      return null;
    }
    return {
      tag: active.tagName.toLowerCase(),
      name: active.getAttribute("name"),
      summary: active.hasAttribute("data-finding-summary"),
      label: active.getAttribute("aria-label"),
      card:
        active
          .closest('[data-schemaform-daisyui="collection-item"]')
          ?.querySelector("input, select")
          ?.getAttribute("name") ?? null,
    };
  });
}

async function openPage(page, baseUrl, route, theme) {
  await page.goto(`${baseUrl}${route}`);
  await page.locator('[data-demo-hydrated="true"]').waitFor({ timeout: 60_000 });
  await form(page).waitFor();
  await eventually(
    `the ${theme} theme to be applied`,
    async () => (await page.evaluate(() => document.documentElement.dataset.theme)) === theme,
  );
}

// --- One theme's run --------------------------------------------------------

// The scenarios in one theme: a fresh browser context, the checkpoints they
// passed through, and everything the browser said along the way. Each scenario
// runs to completion or failure on its own, so a failure on one page does not
// hide the other page's checkpoints.
class ThemeRun {
  constructor({ browser, baseUrl, theme, out }) {
    this.browser = browser;
    this.baseUrl = baseUrl;
    this.theme = theme;
    this.dir = path.join(out, theme);
    this.checkpoints = [];
    this.browserLog = [];
    this.pageErrors = [];
    this.failures = [];
  }

  async open() {
    fs.mkdirSync(this.dir, { recursive: true });
    this.context = await this.browser.newContext({ viewport: { width: 1280, height: 900 } });
    await this.context.addInitScript({ path: axePath });
    await this.context.addInitScript((theme) => {
      window.localStorage.setItem("demo-theme", theme);
    }, this.theme);
    this.page = await this.context.newPage();
    this.page.on("console", (message) =>
      this.browserLog.push({ type: message.type(), text: message.text() }),
    );
    this.page.on("pageerror", (error) => this.pageErrors.push(error.stack ?? error.message));
  }

  // Runs axe over the example region (the daisyUI-rendered form with its reset
  // button and status line) and records the result under `id`.
  //
  // The region, not the document, is the subject: the gallery shell around it
  // is shared by every page and is not what a renderer check is about.
  async checkpoint(id) {
    assert(!this.checkpoints.some((entry) => entry.id === id), `duplicate checkpoint ${id}`);
    // Park the pointer and let CSS transitions finish, so axe measures resting
    // colours rather than a half-faded hover left behind by the last click.
    await this.page.mouse.move(0, 0);
    await this.page.waitForFunction(
      () => document.getAnimations().every((animation) => !(animation instanceof CSSTransition)),
      undefined,
      { timeout: 5_000 },
    );
    const result = await this.page.evaluate(async (selector) => {
      const root = document.querySelector(selector);
      if (!root) {
        throw new Error(`axe root ${selector} is not on the page`);
      }
      return window.axe.run(root, { resultTypes: ["violations"] });
    }, AXE_ROOT);
    await this.page.screenshot({ path: path.join(this.dir, `${id}.png`), fullPage: true });
    fs.writeFileSync(path.join(this.dir, `${id}.axe.json`), `${JSON.stringify(result, null, 2)}\n`);
    const violations = result.violations.map((violation) => ({
      rule: violation.id,
      impact: violation.impact ?? "unknown",
      help: violation.help,
      targets: violation.nodes.flatMap(({ target }) => target.map(String)),
    }));
    this.checkpoints.push({ id, violations });
    const status = violations.length === 0 ? "ok" : `${violations.length} violation(s)`;
    process.stdout.write(`  ${this.theme}/${id}: ${status}\n`);
    for (const violation of violations) {
      process.stdout.write(
        `    ${violation.rule} (${violation.impact}): ${violation.help}\n      ${violation.targets.join("\n      ")}\n`,
      );
    }
  }

  // The daisyUI form: registry widgets through the control renderer, arrays
  // and shell through the structure bundle, the summary through the presenter.
  async daisyuiScenario() {
    const { page, baseUrl } = this;

    await openPage(page, baseUrl, "/daisyui", this.theme);
    await this.checkpoint("profile");

    // A two-character display name violates minLength; the daisyUI Input
    // reports the finding once the control has been left, and the summary
    // lists it at the same moment.
    const name = control(page, "/name");
    await name.fill("Ad");
    await name.press("Tab");
    await eventually(
      "the display name to be marked invalid",
      async () => (await name.getAttribute("aria-invalid")) === "true",
    );
    const findings = form(page).locator("[data-finding-summary] [data-finding]");
    await eventually("the finding summary to list the finding", async () => (await findings.count()) === 1);
    await this.checkpoint("profile-invalid-name");

    await selectTab(page, "Billing");
    await this.checkpoint("billing");

    // Submitting from another tab is blocked, and the blocked submit moves
    // focus to the summary.
    await form(page).locator('button[type="submit"]').click();
    await eventually("the finding summary to take focus", async () => (await activeElement(page))?.summary);
    assert((await findings.count()) === 1, `expected one summary finding, found ${await findings.count()}`);
    await this.checkpoint("blocked-submit");

    // Focus-to-target: the finding's button reveals the Profile tab and
    // focuses the invalid control.
    await findings.first().getByRole("button").click();
    await eventually(
      "the Profile tab to be revealed",
      async () => (await tab(page, "Profile").getAttribute("aria-selected")) === "true",
    );
    await eventually("the display name control to receive focus", async () => {
      const active = await activeElement(page);
      return active?.tag === "input" && active.name === "/name";
    });
    await this.checkpoint("focus-to-target");

    // Presence repair on the nullable nickname, null at baseline. The core
    // decides which presence operations the node allows right now, and the
    // daisyUI renderer places exactly those as buttons: null offers set and
    // remove; a string offers set-null and remove; a missing value offers set
    // and set-null. (A null value displays as its canonical text, `null`, as
    // it does in the built-in; the affordances are what change.)
    const nickname = control(page, "/nickname");
    await expectAffordances(page, "Nickname", { set: true, setNull: false, remove: true });
    await affordance(page, "Set Nickname").click();
    await expectAffordances(page, "Nickname", { set: false, setNull: true, remove: true });
    assert((await nickname.inputValue()) === "", "a freshly set nickname should be the empty string");
    await nickname.fill("Countess");
    await eventually("the nickname edit to apply", async () => (await nickname.inputValue()) === "Countess");
    await this.checkpoint("presence-set");

    await affordance(page, "Set Nickname to null").click();
    await expectAffordances(page, "Nickname", { set: true, setNull: false, remove: true });
    await this.checkpoint("presence-set-null");

    await affordance(page, "Remove Nickname").click();
    await expectAffordances(page, "Nickname", { set: true, setNull: true, remove: false });
    await this.checkpoint("presence-remove");

    await affordance(page, "Set Nickname").click();
    await expectAffordances(page, "Nickname", { set: false, setNull: true, remove: true });
    assert((await nickname.inputValue()) === "", "a nickname set after removal should be the empty string");

    await selectTab(page, "Security");
    await this.checkpoint("security");

    await selectTab(page, "Team");
    await this.checkpoint("team");

    // The right-to-left variant is the same form with its chrome mirrored.
    await openPage(page, baseUrl, "/daisyui/rtl", this.theme);
    assert((await form(page).locator("xpath=..").getAttribute("dir")) === "rtl", "the RTL variant should set dir=rtl");
    await this.checkpoint("rtl-profile");
  }

  // The arrays page: two homogeneous arrays rendered through the daisyUI
  // collection renderer. The renderer only places the affordances; item
  // identity, focus after each mutation, and the live-region announcements are
  // the adapter's, so the scenario asserts all three through the daisyUI chrome.
  async arraysScenario() {
    const { page, baseUrl } = this;

    await openPage(page, baseUrl, "/arrays", this.theme);
    assert((await collection(page, "Tags").count()) === 1, "the Tags collection should be a labelled group");
    assert((await collection(page, "Team members").count()) === 1, "the Team members collection should be a labelled group");
    await this.checkpoint("arrays");

    // Insert before the first tag: the new item is seeded from the schema
    // default, takes focus, and the insertion is announced.
    await affordance(page, "Insert Tags item before position 1").click();
    await eventually("the inserted tag to receive focus", async () => {
      const active = await activeElement(page);
      return active?.tag === "input" && active.name === "/tags/0";
    });
    assert((await control(page, "/tags/0").inputValue()) === "new-tag", "the inserted tag should carry the schema default");
    assert((await control(page, "/tags/1").inputValue()) === "rust", "the former first tag should follow the inserted one");
    await expectAnnouncement(page, "Tags", "Tags item inserted at position 1.");
    await this.checkpoint("arrays-inserted");

    // Move it down: the item keeps its value, focus stays on its move-down
    // button in the moved row, and the move is announced.
    await affordance(page, "Move Tags item at position 1 down").click();
    await eventually("the moved tag to sit at position 2", async () => (await control(page, "/tags/1").inputValue()) === "new-tag");
    await eventually(
      "focus to stay on the moved item's move-down button",
      async () => (await activeElement(page))?.label === "Move Tags item at position 2 down",
    );
    await expectAnnouncement(page, "Tags", "Tags item moved down to position 2.");

    // Remove it: focus moves to the next item's control.
    await affordance(page, "Remove Tags item at position 2").click();
    await eventually("the next tag to receive focus", async () => {
      const active = await activeElement(page);
      return active?.tag === "input" && active.name === "/tags/1";
    });
    assert((await control(page, "/tags/1").inputValue()) === "dioxus", "the next tag should be the one that followed the removed item");
    await expectAnnouncement(page, "Tags", "Tags item removed from position 2.");

    // Append a team member: a new card whose object item is seeded empty (its
    // members offer "Set …"), with focus in the card and the addition announced.
    await affordance(page, "Add Team members item").click();
    await eventually("the new team card to be rendered", async () => (await control(page, "/team/2/name").count()) === 1);
    await eventually("focus to land in the new team card", async () => (await activeElement(page))?.card === "/team/2/name");
    await eventually(
      "the empty member's name to offer its set affordance",
      async () => (await affordance(page, "Set Name").count()) === 1,
    );
    await expectAnnouncement(page, "Team members", "Team members item added at position 3.");
    await this.checkpoint("arrays-mutated");

    // Empty the tags: the collection shows its empty state in place of the
    // cards, the removal is announced, and append is still offered.
    await affordance(page, "Remove Tags item at position 1").click();
    await eventually("a single tag to remain", async () => (await control(page, "/tags/1").count()) === 0);
    await affordance(page, "Remove Tags item at position 1").click();
    const emptyState = collection(page, "Tags").locator('[data-schemaform-daisyui="collection-empty"]');
    await eventually("the empty state to appear", async () => (await emptyState.count()) === 1);
    assert((await emptyState.textContent()) === "Nothing here yet.", "the empty state should say so");
    assert((await control(page, "/tags/0").count()) === 0, "no tag card should remain");
    await expectAnnouncement(page, "Tags", "Tags item removed from position 1.");
    assert((await affordance(page, "Add Tags item").count()) === 1, "append should still be offered");
    await this.checkpoint("arrays-empty");

    // minItems: once one team member remains, the core withdraws removal and
    // the renderer no longer places a remove button for it.
    await affordance(page, "Remove Team members item at position 3").click();
    await eventually("two members to remain", async () => (await control(page, "/team/2/name").count()) === 0);
    await affordance(page, "Remove Team members item at position 2").click();
    await eventually("one member to remain", async () => (await control(page, "/team/1/name").count()) === 0);
    await eventually(
      "removal to be withdrawn at minItems",
      async () => (await form(page).getByRole("button", { name: /^Remove Team members item/ }).count()) === 0,
    );
    await this.checkpoint("arrays-min-items");
  }

  async run() {
    process.stdout.write(`${this.theme} theme\n`);
    await this.open();
    try {
      for (const [name, scenario] of [
        ["daisyui", () => this.daisyuiScenario()],
        ["arrays", () => this.arraysScenario()],
      ]) {
        try {
          await scenario();
        } catch (error) {
          const failure = error instanceof AssertionFailure ? error.message : (error.stack ?? String(error));
          this.failures.push({ scenario: name, failure });
          process.stdout.write(`  ${this.theme}/${name}: FAILED: ${failure}\n`);
          await this.page.screenshot({ path: path.join(this.dir, `${name}-failure.png`), fullPage: true }).catch(() => {});
        }
      }
    } finally {
      fs.writeFileSync(
        path.join(this.dir, "browser.log"),
        `${JSON.stringify({ console: this.browserLog, pageErrors: this.pageErrors }, null, 2)}\n`,
      );
      await this.context.close();
    }
    return {
      theme: this.theme,
      failures: this.failures,
      pageErrors: this.pageErrors,
      checkpoints: this.checkpoints,
    };
  }
}

// --- Entry point ------------------------------------------------------------

async function main() {
  const { values } = parseArgs({
    options: {
      site: { type: "string" },
      url: { type: "string" },
      out: { type: "string", default: path.join(process.cwd(), "artifacts") },
    },
  });
  if (!values.site === !values.url) {
    throw new Error("pass exactly one of --site <directory> or --url <base url>");
  }
  const server = values.site ? await serveSite(values.site) : null;
  const baseUrl = (server?.url ?? values.url).replace(/\/$/, "");
  const out = path.resolve(values.out);
  for (const stale of [...THEMES, "report.json"]) {
    fs.rmSync(path.join(out, stale), { recursive: true, force: true });
  }
  fs.mkdirSync(out, { recursive: true });

  const browser = await chromium.launch({ headless: true });
  const results = [];
  try {
    process.stdout.write(
      `chromium ${browser.version()}, axe-core ${require("axe-core/package.json").version}, ${baseUrl}, axe root ${AXE_ROOT}\n`,
    );
    for (const theme of THEMES) {
      results.push(await new ThemeRun({ browser, baseUrl, theme, out }).run());
    }
  } finally {
    await browser.close();
    await server?.close();
  }

  fs.writeFileSync(path.join(out, "report.json"), `${JSON.stringify(results, null, 2)}\n`);

  const checkpoints = results.flatMap(({ checkpoints }) => checkpoints);
  const violations = checkpoints.reduce((sum, { violations }) => sum + violations.length, 0);
  const failures = results.flatMap(({ failures }) => failures);
  const pageErrors = results.flatMap(({ pageErrors }) => pageErrors);
  process.stdout.write(
    `\n${checkpoints.length} checkpoints across ${results.length} themes: ${violations} violation(s), ${failures.length} failed scenario(s), ${pageErrors.length} page error(s)\n`,
  );
  for (const error of pageErrors) {
    process.stdout.write(`page error: ${error}\n`);
  }
  if (violations > 0 || failures.length > 0 || pageErrors.length > 0) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
