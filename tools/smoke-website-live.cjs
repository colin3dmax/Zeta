const { existsSync } = require("node:fs");
const { join } = require("node:path");
const { execFileSync } = require("node:child_process");

const localPlaywright = join(__dirname, "..", "website", "node_modules", "playwright");
const playwrightRequire = process.env.ZETA_PLAYWRIGHT_REQUIRE
  || (existsSync(localPlaywright) ? localPlaywright : "playwright");

let chromium;
try {
  ({ chromium } = require(playwrightRequire));
} catch (error) {
  try {
    const globalRoot = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
    ({ chromium } = require(join(globalRoot, "playwright")));
  } catch (globalError) {
    console.error(
      [
        "Unable to load Playwright.",
        "Run `npm install` in `website/`, install Playwright globally with npm, or set ZETA_PLAYWRIGHT_REQUIRE to a playwright package path.",
        `Tried: ${playwrightRequire}`,
        error.message,
        globalError.message,
      ].join("\n")
    );
    process.exit(2);
  }
}

const baseUrl = process.env.ZETA_LIVE_URL || "https://zeta.jennieapp.com/";
const publicDocs = [
  { path: "docs/user/getting-started.html", title: "Zeta 用户快速开始", layout: "shell" },
  { path: "docs/project/decision-record.html", title: "Zeta 构建决策记录", layout: "legacy" },
  { path: "docs/project/language-design-process.html", title: "Zeta 语言设计过程", layout: "legacy" },
  { path: "docs/project/stage-roadmap.html", title: "Zeta 阶段路线图", layout: "legacy" },
  { path: "docs/user/language-features.html", title: "Zeta 语言特性学习", layout: "shell" },
  { path: "docs/user/install.html", title: "Zeta 本地安装", layout: "shell" },
  { path: "docs/user/downloads.html", title: "Zeta 下载", layout: "shell" },
  { path: "docs/user/vscode.html", title: "Zeta VS Code 插件使用说明", layout: "shell" },
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
  await page.locator(".toolbar.examples").getByRole("button", { name: "控制流", exact: true }).click();
  const previewText = await page.locator(".editor-highlight").innerText();
  const escapedPreviewText = await page.locator(".editor-highlight").evaluate((node) => node.textContent || "");
  const letKeywordCount = await page.locator(".editor-highlight .tok-keyword").filter({ hasText: /^let$/ }).count();
  const ltOperatorCount = await page.locator(".editor-highlight .tok-operator").filter({ hasText: "<" }).count();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");

  const wasmName = await page.evaluate(() => {
    const entry = performance
      .getEntriesByType("resource")
      .find((item) => /zeta-[a-f0-9]+\.wasm/.test(item.name));
    return entry ? entry.name.match(/zeta-[a-f0-9]+\.wasm/)?.[0] ?? null : null;
  });

  await page.locator(".toolbar.examples").getByRole("button", { name: "布尔逻辑", exact: true }).click();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "AST", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.includes("Unary op=not"));
  await page.locator(".toolbar.examples").getByRole("button", { name: "Match", exact: true }).click();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.locator(".toolbar.examples").getByRole("button", { name: "Struct", exact: true }).click();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.locator(".toolbar.examples").getByRole("button", { name: "限定调用", exact: true }).click();
  const moduleModeGuide = await page.locator(".mode-guide").innerText();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行多文件", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "检查", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "ok");
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.locator(".toolbar.examples").getByRole("button", { name: "别名调用", exact: true }).click();
  const aliasPreview = await page.locator(".editor-highlight").innerText();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.locator(".toolbar.examples").getByRole("button", { name: "冲突诊断", exact: true }).click();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "检查", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.includes("RESOLVE_AMBIGUOUS_FUNCTION"));
  const outputWrapStyle = await page.locator(".output").evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      whiteSpace: style.whiteSpace,
      overflowWrap: style.overflowWrap,
      wordBreak: style.wordBreak,
    };
  });
  await page.locator(".toolbar.examples").getByRole("button", { name: "Enum", exact: true }).click();
  await page.locator(".playground-output .toolbar.compact").getByRole("button", { name: "运行", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".output")?.innerText.trim() === "42");
  await page.getByRole("button", { name: "Run All", exact: true }).click();
  await page.waitForFunction(() => document.body.innerText.includes("15/15 passed"));
  const featureTestsPassed = await page.locator(".feature-test-card strong.pass").count();

  const docChecks = [];
  for (const { path, title, layout } of publicDocs) {
    const response = await page.goto(new URL(path, baseUrl).toString(), { waitUntil: "networkidle" });
    const h1 = await page.locator("h1").first().innerText();
    const navSelector = layout === "shell" ? ".doc-topnav a" : ".doc-nav a";
    const navHome = await page.locator(navSelector, { hasText: "官网首页" }).count();
    const navDocs = await page.locator(navSelector, { hasText: "文档中心" }).count();
    const navLinkColor = await page.locator(navSelector).first().evaluate((node) => getComputedStyle(node).color);
    const shellLayout = layout === "shell"
      ? await page.evaluate(() => {
          const shell = document.querySelector(".doc-shell");
          const sidebar = document.querySelector(".doc-sidebar");
          const topbar = document.querySelector(".doc-topbar");
          if (!shell || !sidebar || !topbar) return null;
          return {
            shellDisplay: getComputedStyle(shell).display,
            sidebarPosition: getComputedStyle(sidebar).position,
            topbarPosition: getComputedStyle(topbar).position,
          };
        })
      : null;
    const layoutOk = layout === "shell"
      ? shellLayout?.shellDisplay === "grid" && shellLayout?.sidebarPosition === "sticky" && shellLayout?.topbarPosition === "sticky"
      : true;
    const sidebarExternalLinks = layout === "shell"
      ? await page.locator(".doc-sidebar a").evaluateAll((nodes) =>
          nodes
            .map((node) => node.getAttribute("href") || "")
            .filter((href) => !href.startsWith("#"))
        )
      : [];
    const sidebarOk = layout !== "shell" || sidebarExternalLinks.length === 0;
    const embeddedPlaygroundOk = path.endsWith("language-features.html")
      ? await page.evaluate(() => {
          const frame = document.querySelector("#feature-playground");
          const targets = Array.from(document.querySelectorAll('a.button[href*="example="]'))
            .map((node) => node.getAttribute("target"));
          if (!frame || targets.length === 0) return false;
          return frame.getAttribute("name") === "feature-playground"
            && frame.clientHeight >= 720
            && targets.every((target) => target === "feature-playground");
        })
      : true;
    docChecks.push({
      path,
      status: response ? response.status() : 0,
      h1,
      layout,
      navHome,
      navDocs,
      navLinkColor,
      shellLayout,
      sidebarExternalLinks,
      embeddedPlaygroundOk,
      ok: Boolean(response?.ok()) && h1 === title && navHome > 0 && navDocs > 0 && navLinkColor === "rgb(17, 17, 17)" && layoutOk && sidebarOk && embeddedPlaygroundOk,
    });
  }

  const result = {
    ok:
      consoleErrors.length === 0 &&
      Boolean(wasmName) &&
      loadedExample.includes("fn add") &&
      operatorCount > 0 &&
      boolCount > 0 &&
      previewText.includes("while count < 3") &&
      !escapedPreviewText.includes("&lt;") &&
      letKeywordCount > 0 &&
      ltOperatorCount > 0 &&
      moduleModeGuide.includes("模块图") &&
      aliasPreview.includes("import demo.math as math") &&
      outputWrapStyle.whiteSpace === "pre-wrap" &&
      outputWrapStyle.overflowWrap === "anywhere" &&
      docChecks.every((doc) => doc.ok),
    url: baseUrl,
    wasm: wasmName,
    loadedExample: loadedExample.includes("fn add"),
    replOperatorTokens: operatorCount,
    replBoolTokens: boolCount,
    previewHasRawLessThan: previewText.includes("while count < 3"),
    previewLeaksHtmlEntity: escapedPreviewText.includes("&lt;"),
    previewLetKeywordTokens: letKeywordCount,
    previewLessThanOperatorTokens: ltOperatorCount,
    moduleModeGuide,
    aliasPreviewHasImportAs: aliasPreview.includes("import demo.math as math"),
    outputWrapStyle,
    featureTestsPassed,
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
