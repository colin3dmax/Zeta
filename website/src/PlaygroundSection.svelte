<script>
  import { onMount } from "svelte";
  import { runZeta } from "./wasm-playground.js";

  export let initialExample = "overview";
  export let embedded = false;

  const keywords = new Set(["module", "import", "as", "export", "fn", "let", "mut", "return", "break", "continue", "if", "else", "while", "match", "struct", "enum"]);
  const types = new Set(["Int", "String", "Bool"]);
  const commands = [":help", ":api", ":topics", ":examples", ":doc", ":complete", ":quit"];
  const topics = ["getting-started", "tutorial", "api", "std", "playground", "module", "import", "as", "fn", "let", "mut", "if", "while", "break", "continue", "match", "struct", "enum", "Int", "String", "Bool"];

  const sample = `module demo.core;
import std.core;
import std.io;

export fn main() -> Int {
  let mut count: Int = 0;
  let mut total: Int = 0;
  while count < 10 {
    count = count + 1;
    if count == 3 {
      continue;
    }
    if count == 6 {
      break;
    }
    total = total + count;
  }
  if total == 12 {
    return 42;
  }
  return 0;
}`;

  const playgroundExamples = {
    overview: sample,
    bindings: `fn main() -> Int {
  let mut answer: Int = 40;
  answer = answer + 2;
  return answer;
}`,
    control: `fn main() -> Int {
  let mut count: Int = 0;
  let mut total: Int = 0;
  while count < 10 {
    count = count + 1;
    if count == 3 {
      continue;
    }
    if count == 6 {
      break;
    }
    total = total + count;
  }
  if total == 12 {
    return 42;
  }
  return 0;
}`,
    functions: `fn add(left: Int, right: Int) -> Int {
  return left + right;
}

fn main() -> Int {
  return add(40, 2);
}`,
    data: `module demo.data;

export struct User {
  name: String,
  age: Int,
}

enum ResultTag {
  Ok,
  Err,
}`,
    struct: `struct User {
  name: String,
  age: Int,
}

fn main() -> Int {
  let user: User = User { name: "Ada", age: 42 };
  return user.age;
}`,
    enum: `enum ResultTag {
  Ok,
  Err,
}

fn main() -> Int {
  let tag: ResultTag = ResultTag.Ok;
  match tag {
    ResultTag.Ok -> { return 42; },
    ResultTag.Err -> { return 0; },
  }
  return 0;
}`,
    match: `fn main() -> Int {
  let value: Int = 2;
  match value {
    1 -> { return 10; },
    2 -> { return 42; },
    _ -> { return 0; },
  }
  return 0;
}`,
    bool: `fn main() -> Bool {
  return true && !false;
}`,
    modules: `// file: main.zeta
module demo.app;
import demo.math;

fn main() -> Int {
  return answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 42;
}`,
    modulesQualified: `// file: main.zeta
module demo.app;
import demo.math;

fn main() -> Int {
  return demo.math.answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return helper();
}

fn helper() -> Int {
  return 42;
}`,
    modulesAlias: `// file: main.zeta
module demo.app;
import demo.math as math;

fn main() -> Int {
  return math.answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 42;
}`,
    modulesAmbiguous: `// file: main.zeta
module demo.app;
import demo.math;
import demo.more;

fn main() -> Int {
  return answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 40;
}

// file: more.zeta
module demo.more;

export fn answer() -> Int {
  return 2;
}`
  };

  const exampleButtons = [
    ["overview", "综合示例"],
    ["bindings", "绑定/赋值"],
    ["control", "控制流"],
    ["functions", "函数调用"],
    ["bool", "布尔逻辑"],
    ["struct", "Struct"],
    ["enum", "Enum"],
    ["match", "Match"],
    ["data", "数据声明"],
    ["modules", "模块图"],
    ["modulesQualified", "限定调用"],
    ["modulesAlias", "别名调用"],
    ["modulesAmbiguous", "冲突诊断"]
  ];

  const featureTests = [
    { name: "模块/import/export", mode: "check-module-graph", example: "modules", expected: "ok" },
    { name: "跨模块限定调用", mode: "run-module-graph", example: "modulesQualified", expected: "42" },
    { name: "import alias 调用", mode: "run-module-graph", example: "modulesAlias", expected: "42" },
    { name: "短名冲突诊断", mode: "check-module-graph", example: "modulesAmbiguous", expectedOk: false, expectedIncludes: "RESOLVE_AMBIGUOUS_FUNCTION" },
    { name: "Int 算术", mode: "run", source: "fn main() -> Int { return 40 + 2; }", expected: "42" },
    { name: "Bool 逻辑", mode: "run", example: "bool", expected: "true" },
    { name: "let mut / 赋值", mode: "run", example: "bindings", expected: "42" },
    { name: "函数调用", mode: "run", example: "functions", expected: "42" },
    { name: "if / while", mode: "run", example: "control", expected: "42" },
    { name: "struct 字面量 / 字段访问", mode: "run", example: "struct", expected: "42" },
    { name: "enum 变体", mode: "run", example: "enum", expected: "42" },
    { name: "match 分支", mode: "run", example: "match", expected: "42" },
    { name: "数据声明 AST", mode: "ast", example: "data", expectedIncludes: "Struct name=User" },
    { name: "标准 import 边界", mode: "check", source: "import std.core;\nimport std.io;\nfn main() -> Int { return 42; }", expected: "ok" },
    { name: "MIR 返回路径诊断", mode: "run", source: "fn main() -> Int {\n  let answer: Int = 42;\n}", expectedOk: false, expectedIncludes: "MIR_MISSING_RETURN" }
  ];

  let source = sample;
  let currentExample = "overview";
  let output = "选择 AST、检查、运行查看真实 Zeta 编译器前端结果。多文件示例会自动使用模块图。";
  let runningMode = "";
  let sourceScrollTop = 0;
  let sourceScrollLeft = 0;
  let sourceCompletionOpen = false;
  let sourceCompletionPrefix = "";
  let featureTestRunning = false;
  let featureTestOutput = "尚未运行。";
  let featureTestResults = [];
  let appliedInitial = "";

  $: playgroundModeHint = hasVirtualFiles(source)
    ? "当前源码包含多个 // file: 文件块：检查和运行会自动使用模块图，跨文件 import/export 可以一起解析。"
    : "当前源码按单文件执行：检查只验证当前文件，运行执行当前文件里的无参数 main。";
  $: sourceSuggestions = completions(sourceCompletionPrefix);
  $: if (initialExample && initialExample !== appliedInitial) {
    loadPlaygroundExample(initialExample, false);
    appliedInitial = initialExample;
  }

  async function runPlayground(mode) {
    const effectiveMode = playgroundModeForSource(mode, source);
    runningMode = effectiveMode;
    output = "running...";
    try {
      const result = await runZeta(effectiveMode, source);
      output = result.output;
    } catch (error) {
      output = `Playground failed: ${error.message}`;
    } finally {
      runningMode = "";
    }
  }

  function playgroundModeForSource(mode, value) {
    if (!hasVirtualFiles(value)) return mode;
    if (mode === "check") return "check-module-graph";
    if (mode === "run") return "run-module-graph";
    return mode;
  }

  function hasVirtualFiles(value) {
    return /^\/\/\s*file:\s+\S+/m.test(value);
  }

  async function runFeatureTests() {
    featureTestRunning = true;
    featureTestOutput = "running...";
    featureTestResults = [];
    const results = [];
    for (const test of featureTests) {
      const testSource = test.source ?? playgroundExamples[test.example];
      try {
        const result = await runZeta(test.mode, testSource);
        const expectedOk = test.expectedOk ?? true;
        const outputMatches = test.expectedIncludes ? result.output.includes(test.expectedIncludes) : result.output.trim() === test.expected;
        results.push({ ...test, passed: result.ok === expectedOk && outputMatches, output: result.output });
      } catch (error) {
        results.push({ ...test, passed: false, output: error.message });
      }
    }
    featureTestResults = results;
    featureTestOutput = `${results.filter((result) => result.passed).length}/${results.length} passed`;
    featureTestRunning = false;
  }

  function loadFeatureTest(test) {
    source = test.source ?? playgroundExamples[test.example] ?? sample;
    currentExample = test.example ?? "custom";
    output = `Feature test: ${test.name}\nMode: ${test.mode}\nExpected: ${test.expected ?? test.expectedIncludes}\nExpected ok: ${test.expectedOk ?? true}`;
  }

  function loadPlaygroundExample(name, resetOutput = true) {
    const normalized = playgroundExamples[name] ? name : "overview";
    source = playgroundExamples[normalized];
    currentExample = normalized;
    if (resetOutput) output = "选择 AST、检查、运行查看真实 Zeta 编译器前端结果。多文件示例会自动使用模块图。";
  }

  export function loadExternalExample(name) {
    loadPlaygroundExample(name);
  }

  function escapeHtml(value) {
    return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  }

  function highlightCode(value) {
    return value.replace(/(&&|\|\||==|!=|<=|>=|->|:[a-z-]+|"(?:[^"\\]|\\.)*"|\b[A-Za-z_][A-Za-z0-9_]*\b|\b\d+\b|[=!<>+*/:-])/g, (part) => {
      const escaped = escapeHtml(part);
      if (commands.includes(part)) return `<span class="tok-command">${escaped}</span>`;
      if (keywords.has(part)) return `<span class="tok-keyword">${escaped}</span>`;
      if (types.has(part)) return `<span class="tok-type">${escaped}</span>`;
      if (part === "true" || part === "false") return `<span class="tok-bool">${escaped}</span>`;
      if (/^"/.test(part)) return `<span class="tok-string">${escaped}</span>`;
      if (/^\d+$/.test(part)) return `<span class="tok-number">${escaped}</span>`;
      if (/^(&&|\|\||==|!=|<=|>=|->|[=!<>+*/:-])$/.test(part)) return `<span class="tok-operator">${escaped}</span>`;
      return escaped;
    });
  }

  function syncSourceScroll(event) {
    sourceScrollTop = event.currentTarget.scrollTop;
    sourceScrollLeft = event.currentTarget.scrollLeft;
  }

  function completionPrefix(value) {
    const match = value.match(/(:?[A-Za-z0-9_-]+)$/);
    return match ? match[1] : "";
  }

  function completions(prefix) {
    if (!prefix) return [];
    const words = [...commands, ...topics, ...keywords, ...types].sort();
    return [...new Set(words)].filter((word) => word.startsWith(prefix)).slice(0, 8);
  }

  function showSourceCompletion() {
    sourceCompletionPrefix = completionPrefix(source.slice(0, document.activeElement?.selectionStart ?? source.length));
    sourceCompletionOpen = sourceSuggestions.length > 0;
  }

  function onSourceKeydown(event) {
    if (event.key === "Tab") {
      event.preventDefault();
      const prefix = completionPrefix(source.slice(0, event.currentTarget.selectionStart));
      const match = completions(prefix)[0];
      if (match) applyTextareaCompletion(event.currentTarget, prefix, match);
      sourceCompletionPrefix = prefix;
      sourceCompletionOpen = completions(prefix).length > 1;
    }
  }

  function applyTextareaCompletion(textarea, prefix, value) {
    const start = textarea.selectionStart - prefix.length;
    const end = textarea.selectionStart;
    source = source.slice(0, start) + value + source.slice(end);
    requestAnimationFrame(() => {
      const pos = start + value.length;
      textarea.setSelectionRange(pos, pos);
      textarea.focus();
    });
  }

  onMount(() => {
    const params = new URLSearchParams(location.search);
    const example = params.get("example");
    if (example) loadPlaygroundExample(example, false);
  });
</script>

<section id="playground" class:embedded-playground-section={embedded}>
  <p class="kicker">Online Playground</p>
  <h2>在线使用</h2>
  <div class="toolbar examples">
    {#each exampleButtons as [name, label]}
      <button type="button" class:current={currentExample === name} aria-pressed={currentExample === name} on:click={() => loadPlaygroundExample(name)}>{label}</button>
    {/each}
  </div>
  <div class="tool-window playground-panel">
    <div class="window-chrome light">
      <div class="window-controls" aria-hidden="true">
        <span></span><span></span><span></span>
      </div>
      <span class="window-title">Zeta Playground</span>
      <small class="window-status">source · output · wasm</small>
    </div>
    <div class="playground">
      <div class="pane source-pane">
        <div class="pane-head">
          <span>Source</span>
          <small>main.zeta</small>
        </div>
        <div class="code-editor">
          <pre class="code-highlight editor-highlight" aria-hidden="true" style={`transform: translate(${-sourceScrollLeft}px, ${-sourceScrollTop}px);`}><code>{@html highlightCode(source)}</code></pre>
          <textarea bind:value={source} on:scroll={syncSourceScroll} on:input={showSourceCompletion} on:keydown={onSourceKeydown} spellcheck="false" aria-label="Zeta source input"></textarea>
          {#if sourceCompletionOpen && sourceSuggestions.length}
            <div class="completion-panel editor-completion">
              {#each sourceSuggestions as item}
                <button type="button" on:click={(event) => applyTextareaCompletion(event.currentTarget.closest(".code-editor").querySelector("textarea"), sourceCompletionPrefix, item)}>{item}</button>
              {/each}
            </div>
          {/if}
        </div>
      </div>
      <div class="pane playground-output">
        <div class="pane-head">
          <span>Output</span>
          <div class="toolbar compact">
            <button title="解析当前源码并输出 AST 结构" disabled={runningMode !== ""} on:click={() => runPlayground("ast")}>AST</button>
            <button title="检查单文件；多文件源码会自动切换为模块图检查" disabled={runningMode !== ""} on:click={() => runPlayground("check")}>检查</button>
            <button title="强制按多个 // file: 文件块建立模块图并检查 import/export" disabled={runningMode !== ""} on:click={() => runPlayground("check-module-graph")}>检查多文件</button>
            <button title="强制按模块图执行多文件程序" disabled={runningMode !== ""} on:click={() => runPlayground("run-module-graph")}>运行多文件</button>
            <button title="运行单文件 main；多文件源码会自动切换为模块图运行" disabled={runningMode !== ""} on:click={() => runPlayground("run")}>运行</button>
          </div>
        </div>
        <div class="mode-guide">{playgroundModeHint}</div>
        <pre class="output"><code>{output}</code></pre>
      </div>
    </div>
    <div class="window-statusbar light">
      <span>wasm frontend</span>
      <span>{runningMode || "idle"}</span>
      <span>AST · 检查 · 运行</span>
    </div>
  </div>
  <p class="note">Playground 直接加载 Zeta 编译器前端编译出的 <code>zeta.wasm</code>，AST、检查和运行都执行当前仓库里的真实 Stage 0 编译器逻辑。</p>
  <div class="feature-tests">
    <div class="feature-tests-head">
      <div>
        <p class="kicker">Feature Tests</p>
        <h3>语言特性在线测试</h3>
      </div>
      <button type="button" disabled={featureTestRunning} on:click={runFeatureTests}>{featureTestRunning ? "Running" : "Run All"}</button>
    </div>
    <p class="note">{featureTestOutput}</p>
    <div class="feature-test-grid">
      {#each featureTests as test}
        <button type="button" class="feature-test-card" on:click={() => loadFeatureTest(test)}>
          <span>{test.name}</span>
          <small>{test.mode} · {test.expected ?? test.expectedIncludes}</small>
          {#if featureTestResults.find((result) => result.name === test.name)}
            <strong class:pass={featureTestResults.find((result) => result.name === test.name)?.passed}>
              {featureTestResults.find((result) => result.name === test.name)?.passed ? "pass" : "fail"}
            </strong>
          {/if}
        </button>
      {/each}
    </div>
  </div>
</section>

<style>
  section {
    scroll-margin-top: 28px;
    padding: 44px 0;
    border-bottom: 1px solid #ddd;
  }

  .embedded-playground-section {
    padding: 0;
    border-bottom: 0;
  }

  .kicker {
    margin: 0 0 16px;
    color: #555;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0;
    text-transform: uppercase;
  }

  h2,
  h3 {
    line-height: 1.05;
    letter-spacing: 0;
  }

  h2 {
    margin: 0 0 24px;
    font-size: clamp(40px, 8vw, 72px);
  }

  h3 {
    margin: 0;
    font-size: 18px;
  }

  button {
    min-height: 40px;
    padding: 8px 14px;
    border: 1px solid #111;
    border-radius: 0;
    background: #111;
    color: #fff;
    font: inherit;
    text-decoration: none;
    cursor: pointer;
  }

  button:disabled {
    cursor: wait;
    opacity: 0.45;
  }

  button + button {
    background: #fff;
    color: #111;
  }

  button.current,
  button[aria-pressed="true"] {
    background: #111;
    color: #fff;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin: 24px 0 0;
  }

  .toolbar.examples {
    margin-bottom: 16px;
  }

  .toolbar.examples button {
    background: #fff;
    color: #111;
  }

  .toolbar.examples button.current,
  .toolbar.examples button[aria-pressed="true"] {
    background: #111;
    color: #fff;
  }

  .toolbar.compact {
    gap: 0;
    margin: 0;
  }

  .toolbar.compact button {
    min-height: 32px;
    padding: 5px 10px;
    font-size: 12px;
  }

  .tool-window {
    border: 1px solid #111;
    background: #fff;
    box-shadow: 8px 8px 0 #111;
  }

  .window-chrome {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 44px;
    padding: 0 14px;
    border-bottom: 1px solid #111;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    font-weight: 800;
    background: #f7f7f7;
    color: #111;
  }

  .window-controls {
    display: flex;
    gap: 7px;
    flex: 0 0 auto;
  }

  .window-controls span {
    width: 10px;
    height: 10px;
    border: 1px solid currentColor;
    border-radius: 999px;
  }

  .window-title {
    flex: 0 0 auto;
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .window-status {
    margin-left: auto;
    color: #555;
    font-weight: 500;
  }

  .playground {
    display: grid;
    grid-template-columns: minmax(0, 1.1fr) minmax(320px, 0.9fr);
  }

  .pane {
    min-width: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
  }

  .pane + .pane {
    border-left: 1px solid #111;
  }

  .pane-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 52px;
    padding: 10px 16px;
    border-bottom: 1px solid #111;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    font-weight: 800;
    text-transform: uppercase;
  }

  .pane-head small {
    color: #666;
    font-weight: 500;
    text-transform: none;
  }

  .playground-output {
    grid-template-rows: auto auto minmax(0, 1fr);
  }

  .mode-guide {
    padding: 8px 16px;
    border-bottom: 1px solid #111;
    color: #555;
    font-size: 12px;
    line-height: 1.45;
    background: #fafafa;
  }

  .code-editor {
    position: relative;
    min-height: 520px;
    background: #fff;
    border: 0;
    overflow: hidden;
  }

  .code-editor textarea {
    position: absolute;
    inset: 0;
    width: 100%;
    min-height: 100%;
    padding: 28px 28px 24px;
    border: 0;
    resize: none;
    background: transparent;
    color: transparent;
    caret-color: #111;
    white-space: pre;
    overflow: auto;
    tab-size: 2;
    font: 16px/1.55 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    outline: 0;
  }

  .code-editor textarea:focus {
    outline: 0;
    box-shadow: inset 0 0 0 2px #111;
  }

  .code-highlight,
  .output {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .code-highlight {
    overflow: auto;
    border: 0;
    padding: 28px 28px 24px;
    white-space: pre-wrap;
    font-size: 16px;
    line-height: 1.55;
  }

  .editor-highlight {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
    white-space: pre;
    will-change: transform;
  }

  .output {
    min-height: 100%;
    margin: 0;
    padding: 18px;
    border: 0;
    background: #fff;
    color: #111;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: break-word;
    font-size: 14px;
    line-height: 1.6;
    box-shadow: none;
  }

  .window-statusbar {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    min-height: 30px;
    padding: 6px 12px;
    border-top: 1px solid #111;
    color: #555;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    background: #f7f7f7;
  }

  .note {
    color: #555;
  }

  .feature-tests {
    margin-top: 22px;
    padding: clamp(18px, 3vw, 28px);
    border: 1px solid #111;
    border-radius: 8px;
    background: #fff;
    box-shadow: 8px 8px 0 #d9d9d9;
  }

  .feature-tests-head {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    align-items: flex-start;
    padding-bottom: 18px;
    border-bottom: 1px solid #d7d7d7;
  }

  .feature-test-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(260px, 100%), 1fr));
    gap: 12px;
    margin-top: 18px;
    background: transparent;
  }

  .feature-test-card {
    display: grid;
    gap: 10px;
    justify-items: start;
    min-width: 0;
    min-height: 112px;
    margin: 0;
    padding: 16px;
    border: 1px solid #d7d7d7;
    border-radius: 8px;
    text-align: left;
    background: #fff;
    color: #111;
    box-shadow: none;
  }

  .feature-test-card:hover {
    border-color: #111;
    background: #f7f7f7;
    color: #111;
    transform: none;
  }

  .feature-test-card span {
    min-width: 0;
    font-weight: 850;
    line-height: 1.25;
  }

  .feature-test-card small {
    max-width: 100%;
    color: #555;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    line-height: 1.45;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .feature-test-card strong {
    width: fit-content;
    max-width: 100%;
    padding: 2px 6px;
    border: 1px solid #111;
    border-radius: 4px;
    background: #111;
    color: #fff;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .feature-test-card strong.pass {
    background: #fff;
    color: #111;
  }

  :global(.tok-keyword) {
    font-weight: 800;
    color: #0b5cad;
  }

  :global(.tok-command) {
    font-weight: 800;
    color: #b45309;
  }

  :global(.tok-type) {
    font-weight: 800;
    color: #7c3aed;
  }

  :global(.tok-operator) {
    font-weight: 800;
    color: #be185d;
  }

  :global(.tok-bool) {
    font-weight: 800;
    color: #047857;
  }

  :global(.tok-string) {
    color: #0f7a3a;
  }

  :global(.tok-number) {
    color: #9f1239;
    background: transparent;
  }

  @media (max-width: 900px) {
    .playground {
      grid-template-columns: 1fr;
    }

    .pane + .pane {
      border-left: 0;
      border-top: 1px solid #111;
    }
  }
</style>
