#!/usr/bin/env python3
import html
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASES = ROOT / "docs" / "releases"
OUT = ROOT / "docs" / "user" / "downloads.html"


def load_manifests():
    manifests = []
    if not RELEASES.exists():
        return manifests
    for path in sorted(RELEASES.glob("*.json")):
        manifests.append(json.loads(path.read_text(encoding="utf-8")))
    return manifests


def row(manifest):
    archive = manifest["archive"]
    return (
        "        <tr>"
        f"<td><code>{html.escape(manifest['os'])}</code></td>"
        f"<td><code>{html.escape(manifest['arch'])}</code></td>"
        f"<td><code>{html.escape(manifest['target'])}</code></td>"
        f"<td><a href=\"../releases/{html.escape(archive)}\">{html.escape(archive)}</a></td>"
        f"<td><code>{html.escape(manifest.get('sha256', ''))}</code></td>"
        "</tr>"
    )


def main():
    manifests = load_manifests()
    rows = "\n".join(row(manifest) for manifest in manifests)
    if not rows:
        rows = (
            "        <tr><td colspan=\"5\">暂无发布产物。请先运行 "
            "<code>sh tools/package-release.sh --publish-dir website/public/releases</code>。</td></tr>"
        )

    OUT.write_text(
        f"""<!doctype html>
<html lang=\"zh-CN\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>Zeta 下载</title>
  <link rel=\"stylesheet\" href=\"../shared.css\">
</head>
<body>
  <main>
    <nav class=\"doc-nav\" aria-label=\"文档导航\">
      <a href=\"../../\">官网首页</a>
      <a href=\"../index.html\">文档中心</a>
      <a href=\"getting-started.html\">快速开始</a>
    </nav>
    <h1>Zeta 下载</h1>
    <p><strong>状态：</strong>accepted</p>
    <p><strong>更新时间：</strong>2026-05-30</p>
    <p><strong>适用范围：</strong>Zeta Stage 0 CLI 预构建包、校验信息和安装入口。</p>
    <p><strong>验收标准：</strong>用户可以下载当前发布产物，核对 SHA256，并按照安装文档放入 PATH 后运行 <code>zeta repl</code>。</p>

    <section class=\"hero\">
      <p class=\"eyebrow\">Downloads</p>
      <h2>预构建包由 release 打包脚本生成。</h2>
      <p>当前列表只展示已经发布到官网的产物。更多平台可以通过 <code>tools/package-release.sh --target &lt;rust-target&gt;</code> 在发布机生成。</p>
    </section>

    <table>
      <thead>
        <tr>
          <th>OS</th>
          <th>Arch</th>
          <th>Target</th>
          <th>Archive</th>
          <th>SHA256</th>
        </tr>
      </thead>
      <tbody>
{rows}
      </tbody>
    </table>

    <h2>安装</h2>
    <p>下载后解压，把 <code>bin/zeta</code> 复制到 PATH 中的目录。完整步骤见 <a href=\"install.html\">本地安装</a>。</p>
    <pre><code>tar -xzf zeta-0.1.0-macos-aarch64.tar.gz
cp zeta-0.1.0-macos-aarch64/bin/zeta ~/.local/bin/zeta
zeta repl</code></pre>
  </main>
</body>
</html>
""",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
