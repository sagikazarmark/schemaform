// Drives the demo's daisyUI-rendered pages in a real browser, in the light and
// dark themes, running axe-core at named checkpoints. The scenarios fall in two
// groups, kept apart so the second can move to the component's registry:
//
// - Adapter contract (`contract-*`): what `schemaform-dioxus` promises every
//   renderer package, exercised through a real one — finding visibility, the
//   edit buffer behind a parse blocker, resynchronisation after a rejected
//   write, blocked submission and summary focus, focus-to-target, presence
//   affordances, write-only widgets resting after every write, and item
//   identity, focus and announcements across array mutations. Stays here: this
//   is how schemaform is tested end to end against a consumer.
// - daisyUI presentation (`presentation-*`): what `schemaform_daisyui` itself
//   decides — which registry widget a kind renders as and how it behaves when
//   driven, the empty state, the chrome under right-to-left writing. Moves with
//   the component.
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

// How daisyUI draws a checkbox's mark right now: the computed style of the
// `::before` pseudo-element that is the check or the indeterminate dash. The
// mark is visible at `opacity` 1 and its shape is the `clipPath`.
async function markOf(checkbox) {
  return checkbox.evaluate((element) => {
    const before = getComputedStyle(element, "::before");
    return { opacity: before.opacity, clipPath: before.clipPath };
  });
}

// Waits for the collection's adapter-owned live region to announce `text`.
async function expectAnnouncement(page, label, text) {
  const status = collection(page, label).locator("[data-array-status]");
  await eventually(`"${text}" to be announced`, async () => (await status.textContent()) === text);
}

// The presence affordances the core currently allows for a control, by label:
// "Set {label}", "Clear {label}" (set null), "Remove {label}", each offered or
// not.
async function expectAffordances(page, label, { set, setNull, remove }) {
  const expected = [
    [`Set ${label}`, set],
    [`Clear ${label}`, setNull],
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
// name (`label`), and the binding of the first control in the array item the
// element sits in (`item`, found through the adapter-owned row wrapper, so it
// does not depend on the renderer's item markup).
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
      item:
        active.closest("[data-array-item]")?.querySelector("input, select")?.getAttribute("name") ??
        null,
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
// passed through, and everything the browser said along the way.
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

  // ==========================================================================
  // Adapter contract — stays with schemaform.
  //
  // What these assert is what `schemaform-dioxus` promises every renderer
  // package, exercised here through a real one: finding visibility, the edit
  // buffer behind a parse blocker, resynchronisation after a rejected write,
  // blocked submission and summary focus, focus-to-target, presence
  // affordances, write-only widgets resting after every write, and item
  // identity, focus and announcements across array mutations. They fail when
  // schemaform breaks a seam, not when daisyUI changes its markup, so every
  // locator below is a control's binding, an affordance's accessible name, a
  // collection's label, or an adapter-owned attribute.
  // ==========================================================================

  // The daisyUI form: the adapter's contract through the registry widgets.
  async contractForm() {
    const { page, baseUrl } = this;

    await openPage(page, baseUrl, "/daisyui", this.theme);
    await this.checkpoint("profile");

    // An unparseable edit stays in the widget as typed: the core keeps it as
    // an edit buffer behind a parse blocker, the control is invalid, and the
    // error element the control references names the blocker.
    const age = control(page, "/age");
    await age.fill("abc");
    await eventually("the age to be marked invalid", async () => (await age.getAttribute("aria-invalid")) === "true");
    assert((await age.inputValue()) === "abc", "an unparseable edit is kept as typed");
    const ageErrors = form(page).locator(`[id="${await age.getAttribute("aria-errormessage")}"]`);
    await eventually(
      "the referenced error element to name the parse blocker",
      async () => (await ageErrors.textContent())?.includes("Enter a valid integer."),
    );
    await age.fill("40");
    await eventually("the age to be valid again", async () => (await age.getAttribute("aria-invalid")) === "false");

    // A write the core rejects — an edit buffer over its resource limit — is
    // resynchronised: the widget carrying the element id returns to the last
    // accepted value and the failure reaches the host's `on_error`, which the
    // demo logs to the console.
    await age.fill("9".repeat(512 * 1024 + 1));
    await eventually("the rejected write to be resynchronised", async () => (await age.inputValue()) === "40");
    await eventually(
      "the rejected write to be reported to the host",
      async () => this.browserLog.some((entry) => entry.type === "error" && entry.text.startsWith("form operation failed")),
    );

    // The accepted write reached form data: a submission shows it.
    await form(page).locator('button[type="submit"]').click();
    const submitted = page.locator('div[dir] > p[role="status"]');
    await eventually("the submission to be shown", async () => (await submitted.count()) === 1);
    assert((await submitted.textContent()).includes('"age": 40'), "the submission should carry the accepted write");
    await page.getByRole("button", { name: "Reset to baseline", exact: true }).click();
    await eventually("the baseline to be restored", async () => (await age.inputValue()) === "36");

    // A two-character display name violates minLength; the finding becomes
    // visible once the control has been left, on the control and in the
    // summary at the same moment.
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

    // Submitting from another tab is blocked, and the blocked submit moves
    // focus to the summary.
    await selectTab(page, "Billing");
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
    // renderer places exactly those: null offers set and remove; a string
    // offers set-null and remove; a missing value offers set and set-null.
    const nickname = control(page, "/nickname");
    await expectAffordances(page, "Nickname", { set: true, setNull: false, remove: true });
    await affordance(page, "Set Nickname").click();
    await expectAffordances(page, "Nickname", { set: false, setNull: true, remove: true });
    assert((await nickname.inputValue()) === "", "a freshly set nickname should be the empty string");
    await nickname.fill("Countess");
    await eventually("the nickname edit to apply", async () => (await nickname.inputValue()) === "Countess");
    await this.checkpoint("presence-set");

    await affordance(page, "Clear Nickname").click();
    await expectAffordances(page, "Nickname", { set: true, setNull: false, remove: true });
    await this.checkpoint("presence-set-null");

    await affordance(page, "Remove Nickname").click();
    await expectAffordances(page, "Nickname", { set: true, setNull: true, remove: false });
    await this.checkpoint("presence-remove");

    await affordance(page, "Set Nickname").click();
    await expectAffordances(page, "Nickname", { set: false, setNull: true, remove: true });
    assert((await nickname.inputValue()) === "", "a nickname set after removal should be the empty string");

    // Write-only widgets never echo their value: after every write the edit
    // hook puts the widget carrying the element id back on its placeholder,
    // and the write-only string is an empty password input.
    await selectTab(page, "Security");
    const twoFactor = form(page).locator('select[name="/two_factor"]');
    assert((await twoFactor.inputValue()) === "", "a write-only boolean rests on its placeholder");
    await twoFactor.selectOption("false");
    await eventually(
      "the write-only boolean to rest on its placeholder after the write",
      async () => (await twoFactor.inputValue()) === "",
    );
    const recovery = form(page).locator('select[name="/recovery_channel"]');
    assert((await recovery.inputValue()) === "", "a write-only choice rests on its placeholder");
    await recovery.selectOption({ label: "sms" });
    await eventually(
      "the write-only choice to rest on its placeholder after the write",
      async () => (await recovery.inputValue()) === "",
    );
    const token = control(page, "/access_token");
    assert((await token.getAttribute("type")) === "password", "a write-only string is a password input");
    assert((await token.inputValue()) === "", "a write-only string shows nothing");
    await this.checkpoint("security-write-only");
  }

  // The arrays page: item identity, focus after each mutation, the live-region
  // announcements, and affordance authorization, through the daisyUI
  // collection. The renderer only places the affordances; all four are the
  // adapter's.
  async contractArrays() {
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

    // Append a team member: a new item whose object is seeded empty (its
    // members offer "Set …"), with focus in the item and the addition announced.
    await affordance(page, "Add Team members item").click();
    await eventually("the new team item to be rendered", async () => (await control(page, "/team/2/name").count()) === 1);
    await eventually("focus to land in the new team item", async () => (await activeElement(page))?.item === "/team/2/name");
    await eventually(
      "the empty member's name to offer its set affordance",
      async () => (await affordance(page, "Set Name").count()) === 1,
    );
    await expectAnnouncement(page, "Team members", "Team members item added at position 3.");
    await this.checkpoint("arrays-mutated");

    // Empty the tags: the removal is announced and append is still offered.
    await affordance(page, "Remove Tags item at position 1").click();
    await eventually("a single tag to remain", async () => (await control(page, "/tags/1").count()) === 0);
    await affordance(page, "Remove Tags item at position 1").click();
    await eventually("no tag to remain", async () => (await control(page, "/tags/0").count()) === 0);
    await expectAnnouncement(page, "Tags", "Tags item removed from position 1.");
    assert((await affordance(page, "Add Tags item").count()) === 1, "append should still be offered");

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

  // ==========================================================================
  // daisyUI presentation — moves with the component.
  //
  // What these assert is what `schemaform_daisyui` itself decides: which
  // registry widget a control kind renders as and how that widget behaves
  // when driven (the native checkbox, the registry Checkbox showing null as
  // indeterminate, the radio group, the compound select), the empty state,
  // and the chrome under right-to-left writing. They fail when daisyUI changes
  // its presentation, not when schemaform changes a seam. The axe checkpoints
  // inside them are the demo's and stay; the registry brings its own harness.
  // ==========================================================================

  // The registry widgets on the daisyUI form.
  async presentationForm() {
    const { page, baseUrl } = this;

    await openPage(page, baseUrl, "/daisyui", this.theme);

    // The native checkbox writes the boolean through the edit hook. A rejected
    // write would resynchronise the checkbox; a stable unchecked state means
    // the core accepted it.
    const active = control(page, "/active");
    assert(await active.isChecked(), "the account starts active");
    await active.click();
    await eventually("the checkbox to be unchecked", async () => !(await active.isChecked()));

    // The nullable registry Checkbox shows null as indeterminate, drawn with
    // the same mark as a native indeterminate checkbox. A click checks it, as
    // activating an indeterminate checkbox does, which the core reflects by
    // offering the clear (set-null) affordance; clearing takes it back to
    // indeterminate, and a click checks it again. daisyUI transitions the mark
    // between shapes, so each shape is awaited rather than read at the flip.
    const newsletter = form(page).locator('[role="checkbox"][name="/newsletter"]');
    assert((await newsletter.getAttribute("aria-checked")) === "mixed", "null shows as indeterminate");
    const indeterminateMark = await eventually(
      "the indeterminate mark to be drawn",
      async () => {
        const mark = await markOf(newsletter);
        return mark.opacity === "1" ? mark : null;
      },
    );
    await newsletter.click();
    await eventually(
      "the newsletter checkbox to be checked",
      async () => (await newsletter.getAttribute("aria-checked")) === "true",
    );
    await affordance(page, "Clear Product newsletter").click();
    await eventually(
      "the newsletter checkbox to be indeterminate again",
      async () => (await newsletter.getAttribute("aria-checked")) === "mixed",
    );
    await newsletter.click();
    await eventually(
      "the newsletter checkbox to be checked once more",
      async () => (await newsletter.getAttribute("aria-checked")) === "true",
    );
    await eventually(
      "the checked mark to be drawn in a shape of its own",
      async () => {
        const mark = await markOf(newsletter);
        return mark.opacity === "1" && mark.clipPath !== indeterminateMark.clipPath ? mark : null;
      },
    );

    // Both writes reached form data: a submission shows them.
    await form(page).locator('button[type="submit"]').click();
    const submitted = page.locator('div[dir] > p[role="status"]');
    await eventually("the submission to be shown", async () => (await submitted.count()) === 1);
    const submittedText = await submitted.textContent();
    for (const expected of ['"active": false', '"newsletter": true']) {
      assert(submittedText.includes(expected), `the submission should contain ${expected}: ${submittedText}`);
    }
    await this.checkpoint("profile-widgets");

    await selectTab(page, "Billing");
    await this.checkpoint("billing");

    // The radio group: clicking an item selects its option and unchecks the
    // previous one.
    const yearly = form(page).getByRole("radio", { name: "yearly", exact: true });
    const monthly = form(page).getByRole("radio", { name: "monthly", exact: true });
    assert((await yearly.getAttribute("aria-checked")) === "true", "yearly is the baseline cycle");
    await monthly.click();
    await eventually("monthly to be checked", async () => (await monthly.getAttribute("aria-checked")) === "true");
    await eventually("yearly to be unchecked", async () => (await yearly.getAttribute("aria-checked")) === "false");

    // The compound select: the trigger opens a listbox, choosing an option
    // closes it, and the trigger shows the option's label.
    const region = form(page).locator('button[name="/region"]');
    assert((await region.textContent())?.includes("eu"), "eu is the baseline region");
    await region.click();
    await page.getByRole("option", { name: "us", exact: true }).click();
    await eventually("the trigger to show the chosen option", async () => (await region.textContent())?.includes("us"));
    await eventually("the listbox to close", async () => (await region.getAttribute("aria-expanded")) === "false");
    await this.checkpoint("billing-widgets");

    await selectTab(page, "Security");
    await this.checkpoint("security");

    await selectTab(page, "Team");
    await this.checkpoint("team");

    // The right-to-left variant is the same form with its chrome mirrored.
    await openPage(page, baseUrl, "/daisyui/rtl", this.theme);
    assert((await form(page).locator("xpath=..").getAttribute("dir")) === "rtl", "the RTL variant should set dir=rtl");
    await this.checkpoint("rtl-profile");
  }

  // The daisyUI collection's own chrome on the arrays page.
  async presentationArrays() {
    const { page, baseUrl } = this;

    await openPage(page, baseUrl, "/arrays", this.theme);

    // Each item is a card labelled by the item noun and its position.
    const firstCard = form(page).getByRole("group", { name: "Tags item 1", exact: true });
    assert((await firstCard.count()) === 1, "the first tag renders as a card named by noun and position");

    // Empty the tags: the collection shows its empty state in place of the
    // cards.
    await affordance(page, "Remove Tags item at position 1").click();
    await eventually("a single tag to remain", async () => (await control(page, "/tags/1").count()) === 0);
    await affordance(page, "Remove Tags item at position 1").click();
    const emptyState = collection(page, "Tags").locator('[data-schemaform-daisyui="collection-empty"]');
    await eventually("the empty state to appear", async () => (await emptyState.count()) === 1);
    assert((await emptyState.textContent()) === "Nothing here yet.", "the empty state should say so");
    assert((await control(page, "/tags/0").count()) === 0, "no tag card should remain");
    await this.checkpoint("arrays-empty");
  }


  async run() {
    process.stdout.write(`${this.theme} theme\n`);
    await this.open();
    try {
      // Each scenario opens its page afresh and runs to completion or failure
      // on its own, so a failure in one does not hide another's checkpoints.
      // The two groups are what stays with schemaform (`contract-*`) and what
      // moves with the component (`presentation-*`).
      for (const [name, scenario] of [
        ["contract-form", () => this.contractForm()],
        ["contract-arrays", () => this.contractArrays()],
        ["presentation-form", () => this.presentationForm()],
        ["presentation-arrays", () => this.presentationArrays()],
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
