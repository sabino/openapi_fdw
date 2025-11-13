import json
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


class FakeForeignDataWrapper:
    def __init__(self, *a, **k):
        pass


sys.modules.setdefault("multicorn", SimpleNamespace(ForeignDataWrapper=FakeForeignDataWrapper))

from openapi_fdw.wrapper import OpenAPIForeignDataWrapper  # noqa: E402


class SimpleHandler(BaseHTTPRequestHandler):
    spec = None
    data = None

    def log_message(self, *args, **kwargs):  # noqa: D401 - silence server logs
        return

    def do_GET(self):  # noqa: N802
        if self.path == "/openapi.json":
            body = json.dumps(self.spec).encode("utf-8")
        elif self.path == "/items":
            body = json.dumps(self.data).encode("utf-8")
        else:
            self.send_response(404)
            self.end_headers()
            return

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class TestIntegrationHTTP(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = [{"id": 1, "title": "Example"}]
        cls.httpd = HTTPServer(("localhost", 0), SimpleHandler)
        host, port = cls.httpd.server_address
        cls.base_url = f"http://{host}:{port}"
        SimpleHandler.spec = {
            "openapi": "3.0.0",
            "servers": [{"url": cls.base_url}],
            "paths": {
                "/items": {
                    "get": {
                        "responses": {
                            "200": {
                                "description": "ok",
                                "content": {
                                    "application/json": {
                                        "schema": {
                                            "type": "array",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "id": {"type": "integer"},
                                                    "title": {"type": "string"},
                                                },
                                            },
                                        }
                                    }
                                },
                            }
                        }
                    }
                }
            },
        }
        SimpleHandler.data = cls.data
        cls.thread = threading.Thread(target=cls.httpd.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()
        cls.thread.join()

    def test_fetch_and_execute(self):
        wrapper = OpenAPIForeignDataWrapper(
            {"openapi_url": f"{self.base_url}/openapi.json", "path": "/items"},
            {"id": {}, "title": {}},
        )
        rows = list(wrapper.execute([], []))
        self.assertEqual(rows, self.data)


if __name__ == "__main__":
    unittest.main()
