# Zeta Language

VS Code syntax highlighting and snippets for Zeta.

## Features

- `.zeta` file association.
- TextMate syntax highlighting for modules, imports, functions, structs, enums, control flow, literals, comments, and operators.
- Basic snippets for modules, functions, structs, enums, and local bindings.
- Static completion and hover documentation for Stage 0 keywords and scalar types.

## Local Development

Open `editors/vscode-zeta` in VS Code and run the extension host with `F5`.

## Install Locally On macOS

From the repository root:

```sh
sh editors/vscode-zeta/scripts/install-local.sh
```

Then reload VS Code with `Developer: Reload Window`, or quit and reopen VS Code.

Open a `.zeta` file, such as `testdata/core_items.zeta`. The language mode should show `Zeta`.

## Manual Install

Create a symlink from the VS Code extensions directory to this extension:

```sh
mkdir -p ~/.vscode/extensions
ln -sfn "$(pwd)/editors/vscode-zeta" ~/.vscode/extensions/zeta.zeta-language-0.1.0
```

Verify that VS Code sees the extension:

```sh
code --list-extensions | grep zeta
```

Then reload VS Code and open a `.zeta` file.
