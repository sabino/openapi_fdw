import json
import unittest
from unittest.mock import patch
import sys
from types import SimpleNamespace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'coincap_fdw'))

class FakeForeignDataWrapper:
    def __init__(self, *args, **kwargs):
        pass

sys.modules.setdefault('multicorn', SimpleNamespace(ForeignDataWrapper=FakeForeignDataWrapper))
sys.modules.setdefault('requests', SimpleNamespace(request=lambda *a, **k: None))

from coincap_fdw.api import fetch_endpoint, DEFAULT_BASE_URL
from coincap_fdw.wrapper import CoinCapForeignDataWrapper


class DummyResponse:
    def __init__(self, data):
        self.content = json.dumps({'data': data}).encode('utf-8')

    def raise_for_status(self):
        pass


def make_mock_request(expected_url, data):
    def _request(method, url):
        assert method == 'GET'
        assert url == expected_url
        return DummyResponse(data)
    return _request


class TestAPI(unittest.TestCase):
    def test_fetch_endpoint(self):
        data = [{'id': 'btc'}]
        url = f"{DEFAULT_BASE_URL}/assets"
        with patch('requests.request', make_mock_request(url, data)):
            self.assertEqual(fetch_endpoint('assets'), data)


class TestWrapper(unittest.TestCase):
    def test_execute_defaults(self):
        data = [{'id': 'btc'}]
        url = f"{DEFAULT_BASE_URL}/assets"
        with patch('requests.request', make_mock_request(url, data)):
            wrapper = CoinCapForeignDataWrapper({}, {'id': {}})
            rows = list(wrapper.execute({}, {}))
            self.assertEqual(rows, [{'id': 'btc'}])

    def test_execute_custom(self):
        data = [{'id': 'btc', 'name': 'Bitcoin'}]
        url = 'https://custom/api/coins'
        with patch('requests.request', make_mock_request(url, data)):
            wrapper = CoinCapForeignDataWrapper({'base_url': 'https://custom/api',
                                                 'endpoint': 'coins'},
                                                {'id': {}, 'name': {}})
            rows = list(wrapper.execute({}, {}))
            self.assertEqual(rows, [{'id': 'btc', 'name': 'Bitcoin'}])


if __name__ == '__main__':
    unittest.main()

