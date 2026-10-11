"""Fake Plausible on 127.0.0.1:8000: echoes the path and the proxy headers it got."""
from http.server import BaseHTTPRequestHandler, HTTPServer


class Echo(BaseHTTPRequestHandler):
    def reply(self, code):
        body = (f"fake-plausible {self.command} {self.path} host={self.headers.get('Host')} "
                f"xff={self.headers.get('X-Forwarded-For')} proto={self.headers.get('X-Forwarded-Proto')} "
                f"upgrade={self.headers.get('Upgrade')} connection={self.headers.get('Connection')}").encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/javascript" if self.path.endswith(".js") else "text/plain")
        self.send_header("Cache-Control", "max-age=0")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.reply(200)

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.reply(202)

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", 8000), Echo).serve_forever()
