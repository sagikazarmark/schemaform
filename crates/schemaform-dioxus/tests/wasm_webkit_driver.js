const { webkit } = require("playwright");

const url = process.env.WASM_BINDGEN_TEST_URL ?? "http://127.0.0.1:8000";

(async () => {
  const browser = await webkit.launch({ headless: true });
  const page = await browser.newPage();
  await page.goto(url);
  await page.waitForFunction(
    () => document.body.textContent.includes("test result: ok"),
    undefined,
    { timeout: 120_000 },
  );
  await browser.close();
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
