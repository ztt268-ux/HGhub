"""Serve local HLS fixtures for player-seek.html without fetching upstream URLs."""
import argparse
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, urlparse

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("media_dir", type=Path)
parser.add_argument("--port", type=int, default=17779)
args = parser.parse_args()
root = args.media_dir.resolve()
if not (root / "list.m3u8").is_file():
    parser.error("media_dir must contain list.m3u8 and its TS segments")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/hls":
            lines = (root / "list.m3u8").read_text().splitlines()
            data = "\n".join(
                line if not line or line.startswith("#") else
                "http://hg.localhost/seg?u=" + quote(
                    "https://media.fixture.invalid/" + line, safe="")
                for line in lines
            ).encode()
            kind = "application/vnd.apple.mpegurl"
        elif url.path == "/seg":
            target = parse_qs(url.query).get("u", [""])[0]
            path = root / Path(urlparse(target).path).name
            if not path.is_file():
                self.send_error(404)
                return
            time.sleep(0.12)  # Include buffering and cancellation during seeks.
            data, kind = path.read_bytes(), "video/mp2t"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def log_message(self, *_args):
        pass


ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
