#!/usr/bin/env python3
"""
Minimal proxy to capture Claude Code API requests.
Logs the raw request body to see exactly what's being sent.

Usage:
  1. Run this proxy: python capture_proxy.py
  2. Set ANTHROPIC_BASE_URL=http://localhost:9999
  3. Run Claude Code and make a request
  4. Check captured_requests.jsonl for the raw data
"""

import json
import sys
from datetime import datetime
from http.server import HTTPServer, BaseHTTPRequestHandler
import urllib.request
import urllib.error

# Where to forward requests (the real muninn proxy or Anthropic)
UPSTREAM_URL = "http://127.0.0.1:55447"  # muninn proxy port

class CaptureHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        # Read request body
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')

        # Parse and pretty-print for logging
        try:
            parsed = json.loads(body)

            # Extract just the messages for analysis
            messages = parsed.get('messages', [])
            print(f"\n{'='*60}")
            print(f"[{datetime.now().isoformat()}] POST {self.path}")
            print(f"Model: {parsed.get('model', 'unknown')}")
            print(f"Messages ({len(messages)}):")

            for i, msg in enumerate(messages):
                role = msg.get('role', 'unknown')
                content = msg.get('content')

                print(f"  [{i}] {role}:")
                if isinstance(content, str):
                    print(f"      Type: string")
                    print(f"      Preview: {content[:200]}...")
                elif isinstance(content, list):
                    print(f"      Type: array ({len(content)} blocks)")
                    for j, block in enumerate(content[:3]):  # First 3 blocks
                        block_type = block.get('type', 'unknown')
                        if block_type == 'text':
                            text = block.get('text', '')
                            print(f"        [{j}] text: {text[:100]}...")
                        else:
                            print(f"        [{j}] {block_type}: {json.dumps(block)[:100]}...")
                else:
                    print(f"      Type: {type(content)}")
                    print(f"      Raw: {content}")

            # Save full request to file
            with open('captured_requests.jsonl', 'a') as f:
                entry = {
                    'timestamp': datetime.now().isoformat(),
                    'path': self.path,
                    'body': parsed
                }
                f.write(json.dumps(entry) + '\n')

        except json.JSONDecodeError as e:
            print(f"[ERROR] Failed to parse JSON: {e}")
            print(f"Raw body: {body[:500]}")

        # Forward to upstream
        try:
            req = urllib.request.Request(
                f"{UPSTREAM_URL}{self.path}",
                data=body.encode('utf-8'),
                headers={k: v for k, v in self.headers.items()},
                method='POST'
            )

            with urllib.request.urlopen(req) as resp:
                response_body = resp.read()

                # Send response back to client
                self.send_response(resp.status)
                for header, value in resp.headers.items():
                    if header.lower() not in ['transfer-encoding', 'content-encoding']:
                        self.send_header(header, value)
                self.end_headers()
                self.wfile.write(response_body)

        except urllib.error.HTTPError as e:
            self.send_response(e.code)
            self.end_headers()
            self.wfile.write(e.read())
        except Exception as e:
            print(f"[ERROR] Upstream error: {e}")
            self.send_response(502)
            self.end_headers()
            self.wfile.write(f"Proxy error: {e}".encode())

    def do_GET(self):
        # Forward GET requests (like /health)
        try:
            req = urllib.request.Request(f"{UPSTREAM_URL}{self.path}")
            with urllib.request.urlopen(req) as resp:
                self.send_response(resp.status)
                for header, value in resp.headers.items():
                    self.send_header(header, value)
                self.end_headers()
                self.wfile.write(resp.read())
        except Exception as e:
            self.send_response(502)
            self.end_headers()
            self.wfile.write(f"Proxy error: {e}".encode())

    def log_message(self, format, *args):
        # Suppress default logging
        pass

if __name__ == '__main__':
    port = 9999
    print(f"Capture proxy starting on http://localhost:{port}")
    print(f"Forwarding to: {UPSTREAM_URL}")
    print(f"Saving requests to: captured_requests.jsonl")
    print()
    print("To use:")
    print(f"  export ANTHROPIC_BASE_URL=http://localhost:{port}")
    print()

    server = HTTPServer(('localhost', port), CaptureHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")
