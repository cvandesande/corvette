#!/usr/bin/env python3
"""A local HTTP responder for route_parity.sh's own self-test.

Not a stand-in for nginx or Frigate: it exists only to prove the checker can
fail. It serves the two rows in selftest/expectations correctly, unless
started with --broken, in which case /api/config answers 200 text/html
instead of the table's expected application/json -- the exact silent-failure
shape (a 200 carrying the wrong body) route_parity.sh exists to catch.

Usage: fixture_server.py PORT [--broken]
"""
import http.server
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    broken = False

    def do_GET(self):  # noqa: N802 (BaseHTTPRequestHandler's own naming)
        if self.path == "/api/config":
            if Handler.broken:
                body = b"<html><body>not json</body></html>"
                content_type = "text/html"
            else:
                body = b'{"ok":true}'
                content_type = "application/json"
        elif self.path == "/":
            body = b"<html><body>selftest-shell</body></html>"
            content_type = "text/html"
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # silence per-request stderr noise
        pass


def main():
    port = int(sys.argv[1])
    Handler.broken = "--broken" in sys.argv[2:]
    server = http.server.HTTPServer(("127.0.0.1", port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
