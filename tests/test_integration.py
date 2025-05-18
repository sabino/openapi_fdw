import json
import sys
import threading
import unittest
from http.server import HTTPServer, BaseHTTPRequestHandler
from types import SimpleNamespace
from pathlib import Path

# import from local source
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'coincap_fdw'))

# fake multicorn dependency
class FakeForeignDataWrapper:
    def __init__(self, *a, **k):
        pass

sys.modules.setdefault('multicorn', SimpleNamespace(ForeignDataWrapper=FakeForeignDataWrapper))

# provide a minimal requests module using urllib for real HTTP
import urllib.request

def requests_request(method, url):
    assert method == "GET"
    with urllib.request.urlopen(url) as resp:
        data = resp.read()
        class Resp:
            def __init__(self, content, status):
                self.content = content
                self.status_code = status
            def raise_for_status(self):
                if self.status_code >= 400:
                    raise RuntimeError("error")
        return Resp(data, resp.getcode())

sys.modules['requests'] = SimpleNamespace(request=requests_request)

from coincap_fdw.api import fetch_endpoint, DEFAULT_BASE_URL
from coincap_fdw.wrapper import CoinCapForeignDataWrapper


class SimpleHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({'data': [{'id': 'btc'}]}).encode('utf-8')
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class TestIntegrationHTTP(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.httpd = HTTPServer(('localhost', 0), SimpleHandler)
        cls.thread = threading.Thread(target=cls.httpd.serve_forever, daemon=True)
        cls.thread.start()
        host, port = cls.httpd.server_address
        cls.base_url = f"http://{host}:{port}"

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()
        cls.thread.join()

    def test_fetch_and_execute(self):
        data = fetch_endpoint('assets', base_url=self.base_url)
        self.assertEqual(data, [{'id': 'btc'}])

        wrapper = CoinCapForeignDataWrapper({'base_url': self.base_url}, {'id': {}})
        rows = list(wrapper.execute({}, {}))
        self.assertEqual(rows, [{'id': 'btc'}])


if __name__ == '__main__':
    unittest.main()
