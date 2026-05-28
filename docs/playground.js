function sourceText() {
  return document.getElementById("source").value;
}

function setOutput(text) {
  document.getElementById("output").textContent = text;
}

function roughAst(source) {
  var lines = source.split(/\r?\n/);
  var out = ["Module"];
  lines.forEach(function (line) {
    var trimmed = line.trim();
    if (trimmed.startsWith("module ")) {
      out.push("  ModuleDecl name=" + trimmed.replace(/^module\s+/, "").replace(/;$/, ""));
    } else if (trimmed.startsWith("import ")) {
      out.push("  Import path=" + trimmed.replace(/^import\s+/, "").replace(/;$/, ""));
    } else if (trimmed.includes("struct ")) {
      out.push("  Struct " + trimmed.replace(/\{$/, "").trim());
    } else if (trimmed.includes("enum ")) {
      out.push("  Enum " + trimmed.replace(/\{$/, "").trim());
    } else if (trimmed.includes("fn ")) {
      out.push("  Function " + trimmed.replace(/\{$/, "").trim());
    } else if (trimmed.startsWith("let ")) {
      out.push("    Let " + trimmed.replace(/;$/, ""));
    } else if (trimmed.startsWith("return")) {
      out.push("    Return " + trimmed.replace(/;$/, ""));
    } else if (trimmed.startsWith("if ")) {
      out.push("    If " + trimmed.replace(/\{$/, "").trim());
    } else if (trimmed.startsWith("while ")) {
      out.push("    While " + trimmed.replace(/\{$/, "").trim());
    } else if (trimmed.startsWith("match ")) {
      out.push("    Match " + trimmed.replace(/\{$/, "").trim());
    }
  });
  return out.join("\n");
}

function roughCheck(source) {
  var messages = [];
  if (!source.includes("fn ")) {
    messages.push("warning: no function declaration found");
  }
  if (source.includes("if 1")) {
    messages.push("TYPE_IF_CONDITION: if condition should be Bool");
  }
  if (source.includes("return \"")) {
    messages.push("TYPE_RETURN_MISMATCH: return String where Int may be expected");
  }
  if (messages.length === 0) {
    messages.push("ok");
  }
  messages.push("");
  messages.push("Browser playground is a prototype. Run cargo run -- check for compiler-backed diagnostics.");
  return messages.join("\n");
}

document.getElementById("showAst").addEventListener("click", function () {
  setOutput(roughAst(sourceText()));
});

document.getElementById("showCheck").addEventListener("click", function () {
  setOutput(roughCheck(sourceText()));
});
