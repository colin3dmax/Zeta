const playwrightRequire = process.env.ZETA_PLAYWRIGHT_REQUIRE || "playwright";

let chromium;
try {
  ({ chromium } = require(playwrightRequire));
} catch (error) {
  console.error(
    [
      "Unable to load Playwright.",
      "Install it where Node can resolve `playwright`, or set ZETA_PLAYWRIGHT_REQUIRE to a playwright package path.",
      `Tried: ${playwrightRequire}`,
      error.message,
    ].join("\n")
  );
  process.exit(2);
}

const baseUrl = process.env.ZETA_LIVE_URL || "https://zeta.jennieapp.com/";

(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const consoleErrors = [];

  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  });
  page.on("pageerror", (err) => consoleErrors.push(err.message));

  await page.goto(baseUrl, { waitUntil: "networkidle" });

  await page.locator('a[href="#repl"]').click();
  const replInput = page.locator(".terminal-input-row input");
  await replInput.fill("true && !false");
  await page.waitForTimeout(100);
  const operatorCount = await page.locator(".terminal-input-highlight .tok-operator").count();
  const boolCount = await page.locator(".terminal-input-highlight .tok-bool").count();
  await replInput.press("Enter");
  await page.waitForFunction(() => document.body.innerText.includes("true"));

  await page.getByRole("link", { name: "Playground", exact: true }).click();
  await page.locator("button", { hasText: "Run" }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");

  const wasmName = await page.evaluate(() => {
    const entry = performance
      .getEntriesByType("resource")
      .find((item) => /zeta-[a-f0-9]+\.wasm/.test(item.name));
    return entry ? entry.name.match(/zeta-[a-f0-9]+\.wasm/)?.[0] ?? null : null;
  });

  await page.locator("button", { hasText: "AST" }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.includes("Unary op=not"));

  const result = {
    ok: consoleErrors.length === 0 && Boolean(wasmName) && operatorCount > 0 && boolCount > 0,
    url: baseUrl,
    wasm: wasmName,
    replOperatorTokens: operatorCount,
    replBoolTokens: boolCount,
    consoleErrors,
  };

  console.log(JSON.stringify(result, null, 2));
  await browser.close();
  if (!result.ok) process.exit(1);
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
