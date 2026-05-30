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
const publicDocs = [
  ["docs/project/decision-record.html", "Zeta 构建决策记录"],
  ["docs/project/language-design-process.html", "Zeta 语言设计过程"],
  ["docs/project/stage-roadmap.html", "Zeta 阶段路线图"],
  ["docs/user/language-features.html", "Zeta 语言特性学习"],
  ["docs/user/install.html", "Zeta 本地安装"],
];

(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const consoleErrors = [];

  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  });
  page.on("pageerror", (err) => consoleErrors.push(err.message));

  await page.goto(new URL("?example=functions#playground", baseUrl).toString(), { waitUntil: "networkidle" });
  const loadedExample = await page.locator(".source-pane textarea").inputValue();

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

  await page.locator("button", { hasText: "布尔逻辑" }).click();
  await page.locator("button", { hasText: "AST" }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.includes("Unary op=not"));

  const docChecks = [];
  for (const [path, title] of publicDocs) {
    const response = await page.goto(new URL(path, baseUrl).toString(), { waitUntil: "networkidle" });
    const h1 = await page.locator("h1").first().innerText();
    const navHome = await page.locator(".doc-nav a", { hasText: "官网首页" }).count();
    const navDocs = await page.locator(".doc-nav a", { hasText: "文档中心" }).count();
    const navLinkColor = await page.locator(".doc-nav a").first().evaluate((node) => getComputedStyle(node).color);
    docChecks.push({
      path,
      status: response ? response.status() : 0,
      h1,
      navHome,
      navDocs,
      navLinkColor,
      ok: Boolean(response?.ok()) && h1 === title && navHome > 0 && navDocs > 0 && navLinkColor === "rgb(17, 17, 17)",
    });
  }

  const result = {
    ok:
      consoleErrors.length === 0 &&
      Boolean(wasmName) &&
      loadedExample.includes("fn add") &&
      operatorCount > 0 &&
      boolCount > 0 &&
      docChecks.every((doc) => doc.ok),
    url: baseUrl,
    wasm: wasmName,
    loadedExample: loadedExample.includes("fn add"),
    replOperatorTokens: operatorCount,
    replBoolTokens: boolCount,
    docChecks,
    consoleErrors,
  };

  console.log(JSON.stringify(result, null, 2));
  await browser.close();
  if (!result.ok) process.exit(1);
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
