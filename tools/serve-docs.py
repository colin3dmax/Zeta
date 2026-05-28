#!/usr/bin/env python3
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlparse
import argparse
import json
import mimetypes
import os
import threading
import time


ROOT = Path(__file__).resolve().parents[1]
WATCH_SUFFIXES = {".html", ".css", ".js", ".md", ".json", ".zeta", ".ast", ".py", ".rs"}


def snapshot_version():
    latest = 0
    for path in ROOT.rglob("*"):
        if ".git" in path.parts or "target" in path.parts:
            continue
        if path.is_file() and path.suffix in WATCH_SUFFIXES:
            try:
                latest = max(latest, path.stat().st_mtime_ns)
            except OSError:
                pass
    return latest


class ReloadState:
    def __init__(self):
        self.version = snapshot_version()
        self.lock = threading.Lock()

    def update(self):
        current = snapshot_version()
        with self.lock:
            if current > self.version:
                self.version = current

    def get(self):
        with self.lock:
            return self.version


STATE = ReloadState()


def watch_loop(interval):
    while True:
        STATE.update()
        time.sleep(interval)


class DocsHandler(SimpleHTTPRequestHandler):
    def translate_path(self, path):
        parsed = urlparse(path)
        clean = unquote(parsed.path).lstrip("/")
        return str((ROOT / clean).resolve())

    def do_GET(self):
        if self.path.startswith("/__zeta_reload"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(json.dumps({"version": STATE.get()}).encode("utf-8"))
            return
        super().do_GET()

    def send_head(self):
        path = Path(self.translate_path(self.path))
        try:
            path.relative_to(ROOT)
        except ValueError:
            self.send_error(403)
            return None
        if path.is_dir():
            path = path / "docs" / "index.html" if path == ROOT else path / "index.html"
        if path.suffix == ".html" and path.exists():
            return self.send_html_with_reload(path)
        return super().send_head()

    def send_html_with_reload(self, path):
        html = path.read_text(encoding="utf-8")
        if "__zeta_reload" not in html:
            html = html.replace("</body>", RELOAD_SNIPPET + "\n</body>")
        encoded = html.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(encoded)
        return None

    def guess_type(self, path):
        if path.endswith(".zeta"):
            return "text/plain; charset=utf-8"
        return mimetypes.guess_type(path)[0] or "application/octet-stream"


RELOAD_SNIPPET = """<script>
(function () {
  var version = null;
  function poll() {
    fetch('/__zeta_reload', { cache: 'no-store' })
      .then(function (response) { return response.json(); })
      .then(function (state) {
        if (version === null) {
          version = state.version;
        } else if (state.version !== version) {
          location.reload();
        }
      })
      .catch(function () {});
  }
  setInterval(poll, 1000);
  poll();
}());
</script>"""


def main():
    parser = argparse.ArgumentParser(description="Serve Zeta docs with browser auto reload.")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--interval", type=float, default=0.5)
    args = parser.parse_args()

    os.chdir(ROOT)
    threading.Thread(target=watch_loop, args=(args.interval,), daemon=True).start()
    server = ThreadingHTTPServer((args.host, args.port), DocsHandler)
    print(f"Serving Zeta docs at http://{args.host}:{args.port}/docs/index.html")
    print("Auto reload watches docs, README, testdata, tools, editors, and src files.")
    server.serve_forever()


if __name__ == "__main__":
    main()
