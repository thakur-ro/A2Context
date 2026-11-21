"""Serve the dashboard and an optional model partner on one loopback origin."""
import argparse
from http.client import HTTPConnection, HTTPSConnection, HTTPException
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from threading import BoundedSemaphore
from urllib.parse import urlsplit


def loopback(host):
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return host == "localhost"


class Dashboard(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body, mime="application/json", csp=None):
        self.send_response(status)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        if csp:
            self.send_header("Content-Security-Policy", csp)
        self.end_headers()
        self.wfile.write(body)

    def error(self, status, message):
        self.reply(status, json.dumps({"message": message}).encode())

    def handle_dashboard(self):
        config = self.server.config
        port = self.server.server_port
        host = self.headers.get("Host")
        if (host not in {f"127.0.0.1:{port}", f"localhost:{port}"}
                or self.headers.get("Origin", f"http://{host}") != f"http://{host}"
                or self.headers.get("Sec-Fetch-Site") == "cross-site"):
            return self.error(403, "Open the dashboard on its loopback origin.")
        if not self.path.startswith("/") or self.path.startswith("//"):
            return self.error(400, "Invalid path.")
        if self.command == "GET" and self.path.split("?")[0] == "/api/sidecar/chat-config":
            return self.reply(200, json.dumps({"models": config.model}).encode())
        partner = self.path == "/api/chat/completions" and self.command == "POST"
        if self.command == "POST" and not partner and self.path.split("?")[0] not in {
                "/api/sidecar/preview", "/api/pending", "/api/context-comparison"}:
            return self.error(405, "Send writes and telemetry to the sidecar.")
        if partner and not self.server.slots.acquire(blocking=False):
            return self.error(429, "Both partner requests are busy.")
        try:
            body = None
            headers = {}
            if self.command == "POST":
                length = int(self.headers.get("Content-Length", "-1"))
                if not 0 <= length <= (131072 if partner else 2097152):
                    return self.error(413, "Invalid request size.")
                body = self.rfile.read(length)
                headers["Content-Type"] = "application/json"
            if partner:
                data = json.loads(body)
                if not isinstance(data, dict) or data.get("model") not in config.model:
                    return self.error(400, "Select a configured model.")
                messages = data.get("messages")
                if not isinstance(messages, list) or not 1 <= len(messages) <= 8 or any(
                        not isinstance(m, dict) or m.get("role") not in ("system", "user", "assistant")
                        or not isinstance(m.get("content"), str) for m in messages):
                    return self.error(400, "Invalid chat messages.")
                max_tokens = data.get("max_tokens", 450)
                if type(max_tokens) is not int or max_tokens < 1:
                    return self.error(400, "Invalid response token limit.")
                body = json.dumps({"model": data["model"], "messages": messages, "max_tokens": min(max_tokens, 450)}).encode()
                if self.server.key:
                    headers["Authorization"] = "Bearer " + self.server.key
            endpoint = config.endpoint if partner else config.sidecar
            connection = HTTPSConnection if endpoint.scheme == "https" else HTTPConnection
            upstream = connection(endpoint.hostname, endpoint.port, timeout=55 if partner else 20)
            try:
                upstream.request(self.command, endpoint.path if partner else self.path, body, headers)
                response = upstream.getresponse()
                # Keep model errors and upstream credentials out of the browser.
                if partner and response.status != 200:
                    return self.error(502, f"Model endpoint returned HTTP {response.status}.")
                limit = 1048576 if partner else 16777216
                result = response.read(limit + 1)
                if len(result) > limit:
                    return self.error(502, "Response exceeded the gateway limit.")
                self.reply(response.status, result, response.getheader("Content-Type", "application/json"),
                           response.getheader("Content-Security-Policy"))
            finally:
                upstream.close()
        except (ValueError, TypeError):
            self.error(400, "Invalid request.")
        except (OSError, HTTPException):
            self.error(502, "Could not reach the configured service.")
        finally:
            if partner:
                self.server.slots.release()

    do_GET = handle_dashboard
    do_POST = handle_dashboard


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sidecar", type=urlsplit, required=True)
    parser.add_argument("--endpoint", type=urlsplit, required=True, help="Full chat completions URL")
    parser.add_argument("--model", action="append", required=True)
    parser.add_argument("--port", type=int, default=8041)
    config = parser.parse_args()
    for endpoint in (config.sidecar, config.endpoint):
        if (not endpoint.hostname or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment
                or not (endpoint.scheme == "https" or (endpoint.scheme == "http" and loopback(endpoint.hostname)))):
            parser.error("Use HTTPS or loopback HTTP endpoints without embedded credentials.")
    if not loopback(config.sidecar.hostname) or config.sidecar.path.strip("/"):
        parser.error("--sidecar must be a loopback origin.")
    with ThreadingHTTPServer(("127.0.0.1", config.port), Dashboard) as server:
        server.config = config
        server.key = os.environ.get("A2CONTEXT_CHAT_KEY") or os.environ.get("NEXUSWEAVE_CHAT_KEY")
        server.slots = BoundedSemaphore(2)
        print(f"Open http://127.0.0.1:{server.server_port}", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
