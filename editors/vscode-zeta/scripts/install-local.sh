#!/usr/bin/env sh
set -eu

extension_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
install_dir="${HOME}/.vscode/extensions/zeta.zeta-language-0.1.0"

mkdir -p "${HOME}/.vscode/extensions"
ln -sfn "$extension_dir" "$install_dir"

echo "Installed Zeta VS Code extension:"
echo "  $install_dir -> $extension_dir"
echo
echo "Reload VS Code, then open a .zeta file."
