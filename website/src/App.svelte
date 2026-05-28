<script>
  const navItems = [
    { id: "overview", label: "概览" },
    { id: "start", label: "快速开始" },
    { id: "repl", label: "交互终端" },
    { id: "playground", label: "Playground" },
    { id: "tutorial", label: "教程" },
    { id: "vscode", label: "VS Code" },
    { id: "design", label: "设计文档" }
  ];

  const sample = `module demo.core;

export fn main() -> Int {
  let answer: Int = 40 + 2;
  return answer;
}`;

  let active = "overview";
  let source = sample;
  let output = "选择 AST 或 Check 查看结果。";

  function roughAst(text) {
    const lines = text.split(/\r?\n/);
    const out = ["Module"];
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith("module ")) {
        out.push(`  ModuleDecl name=${trimmed.replace(/^module\s+/, "").replace(/;$/, "")}`);
      } else if (trimmed.startsWith("import ")) {
        out.push(`  Import path=${trimmed.replace(/^import\s+/, "").replace(/;$/, "")}`);
      } else if (trimmed.includes("struct ")) {
        out.push(`  Struct ${trimmed.replace(/\{$/, "").trim()}`);
      } else if (trimmed.includes("enum ")) {
        out.push(`  Enum ${trimmed.replace(/\{$/, "").trim()}`);
      } else if (trimmed.includes("fn ")) {
        out.push(`  Function ${trimmed.replace(/\{$/, "").trim()}`);
      } else if (trimmed.startsWith("let ")) {
        out.push(`    Let ${trimmed.replace(/;$/, "")}`);
      } else if (trimmed.startsWith("return")) {
        out.push(`    Return ${trimmed.replace(/;$/, "")}`);
      } else if (trimmed.startsWith("if ")) {
        out.push(`    If ${trimmed.replace(/\{$/, "").trim()}`);
      } else if (trimmed.startsWith("while ")) {
        out.push(`    While ${trimmed.replace(/\{$/, "").trim()}`);
      } else if (trimmed.startsWith("match ")) {
        out.push(`    Match ${trimmed.replace(/\{$/, "").trim()}`);
      }
    }
    return out.join("\n");
  }

  function roughCheck(text) {
    const messages = [];
    if (!text.includes("fn ")) messages.push("warning: no function declaration found");
    if (text.includes("if 1")) messages.push("TYPE_IF_CONDITION: if condition should be Bool");
    if (text.includes('return "')) messages.push("TYPE_RETURN_MISMATCH: return String where Int may be expected");
    if (messages.length === 0) messages.push("ok");
    messages.push("");
    messages.push("Browser playground is a prototype. Run cargo run -- check for compiler-backed diagnostics.");
    return messages.join("\n");
  }

  function showAst() {
    output = roughAst(source);
  }

  function showCheck() {
    output = roughCheck(source);
  }
</script>

<svelte:head>
  <title>Zeta 编程语言</title>
</svelte:head>

<div class="shell">
  <aside class="sidebar" aria-label="文档导航">
    <a class="brand" href="#overview" on:click={() => (active = "overview")}>
      <span>Zeta</span>
      <small>Programming Language</small>
    </a>
    <nav>
      {#each navItems as item}
        <a class:current={active === item.id} href={`#${item.id}`} on:click={() => (active = item.id)}>{item.label}</a>
      {/each}
    </nav>
  </aside>

  <main>
    <section id="overview" class="hero">
      <p class="kicker">Draft · 2026-05-28</p>
      <h1>低门槛，高上限，小内核，高性能。</h1>
      <p class="lead">
        Zeta 是一门严肃的专业编程语言，面向上层应用、服务端、桌面、移动端、WASM、操作系统、嵌入式和硬件软件。
        核心内核保持小而强，外围能力通过标准库和第三方模块按需扩展。
      </p>
      <div class="actions">
        <a href="#playground" on:click={() => (active = "playground")}>打开 Playground</a>
        <a href="#start" on:click={() => (active = "start")}>快速开始</a>
      </div>
    </section>

    <section class="band">
      <div>
        <span class="metric">Stage 0</span>
        <p>当前原型覆盖 parser、AST dump、基础 name resolution 和 typecheck。</p>
      </div>
      <div>
        <span class="metric">Small Core</span>
        <p>AI、平台 SDK 和高阶运行时能力放在外部模块，不进入最小内核。</p>
      </div>
      <div>
        <span class="metric">Native First</span>
        <p>目标覆盖 native、WASM、Windows、iOS、Android 和 RISC-V。</p>
      </div>
    </section>

    <section id="start">
      <p class="kicker">Getting Started</p>
      <h2>快速开始</h2>
      <div class="grid">
        <article>
          <h3>验证环境</h3>
          <pre><code>cargo test
python3 tools/check-docs.py
python3 tools/check-vscode-extension.py</code></pre>
        </article>
        <article>
          <h3>查看 AST</h3>
          <pre><code>cargo run -- ast-dump testdata/core_items.zeta</code></pre>
        </article>
        <article>
          <h3>检查源码</h3>
          <pre><code>cargo run -- check testdata/core_items.zeta</code></pre>
        </article>
      </div>
    </section>

    <section id="repl">
      <p class="kicker">Interactive Mode</p>
      <h2>交互终端</h2>
      <table>
        <thead>
          <tr><th>命令</th><th>用途</th><th>示例</th></tr>
        </thead>
        <tbody>
          <tr><td><code>:help</code></td><td>显示 REPL 命令和主题列表。</td><td><code>:help</code></td></tr>
          <tr><td><code>:doc &lt;topic&gt;</code></td><td>查询关键词或类型的短文档。</td><td><code>:doc let</code></td></tr>
          <tr><td><code>:complete &lt;prefix&gt;</code></td><td>列出当前已知补全候选。</td><td><code>:complete st</code></td></tr>
          <tr><td><code>:quit</code></td><td>退出 REPL。</td><td><code>:quit</code></td></tr>
        </tbody>
      </table>
      <p>当前 REPL 是 Stage 0 交互式语法终端：实时解析输入并输出 AST dump。真正执行 Zeta 代码需要后续 MIR interpreter。</p>
    </section>

    <section id="playground">
      <p class="kicker">Online Playground</p>
      <h2>在线使用</h2>
      <div class="playground">
        <label>
          <span>Source</span>
          <textarea bind:value={source} spellcheck="false"></textarea>
        </label>
        <div>
          <div class="toolbar">
            <button on:click={showAst}>AST</button>
            <button on:click={showCheck}>Check</button>
          </div>
          <pre class="output"><code>{output}</code></pre>
        </div>
      </div>
    </section>

    <section id="tutorial">
      <p class="kicker">Tutorial</p>
      <h2>在线教程</h2>
      <div class="grid two">
        <article><h3>1. 模块与函数</h3><p>从 <code>module</code>、<code>export fn</code> 和标量类型开始。</p></article>
        <article><h3>2. 控制流</h3><p>学习 <code>if</code>、<code>while</code>、<code>match</code> 和返回类型。</p></article>
        <article><h3>3. 数据建模</h3><p>用 <code>struct</code> 和 <code>enum</code> 描述稳定的数据边界。</p></article>
        <article><h3>4. 工具链</h3><p>掌握 <code>ast-dump</code>、<code>check</code>、REPL 和编辑器插件。</p></article>
      </div>
    </section>

    <section id="vscode">
      <p class="kicker">Editor</p>
      <h2>VS Code 插件</h2>
      <pre><code>sh editors/vscode-zeta/scripts/install-local.sh</code></pre>
      <p>插件提供 Zeta 语法高亮、snippets、静态补全和 hover 文档。安装后打开 <code>.zeta</code> 文件即可应用。</p>
    </section>

    <section id="design">
      <p class="kicker">Design Notes</p>
      <h2>设计文档</h2>
      <ul class="links">
        <li>语言定位与产品原则</li>
        <li>MVP Baseline 与冻结边界</li>
        <li>编译器与自举路线</li>
        <li>跨平台与运行时架构</li>
        <li>AI 原生能力与权限模型</li>
      </ul>
    </section>
  </main>
</div>
