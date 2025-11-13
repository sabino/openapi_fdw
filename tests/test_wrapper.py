import json
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


class FakeForeignDataWrapper:
    def __init__(self, *args, **kwargs):
        pass


sys.modules.setdefault("multicorn", SimpleNamespace(ForeignDataWrapper=FakeForeignDataWrapper))

import openapi_fdw.api  # noqa: E402  pylint: disable=wrong-import-position
from openapi_fdw.api import (  # noqa: E402  pylint: disable=wrong-import-position
    OpenAPIError,
    choose_server_url,
    extract_response_schema,
    fetch_json,
    get_operation,
    load_spec,
    schema_column_order,
)
from openapi_fdw.wrapper import OpenAPIForeignDataWrapper  # noqa: E402  pylint: disable=wrong-import-position


class TestAPIHelpers(unittest.TestCase):
    def setUp(self):
        self.spec = {
            "openapi": "3.1.0",
            "servers": [{"url": "https://example.test/api"}],
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
                                                    "name": {"type": "string"},
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

    def test_choose_server_url(self):
        url = choose_server_url(self.spec, None)
        self.assertEqual(url, "https://example.test/api")

    def test_extract_schema(self):
        operation = get_operation(self.spec, "/items", "get")
        schema = extract_response_schema(operation)
        self.assertEqual(schema["type"], "array")
        columns = schema_column_order(schema)
        self.assertEqual(columns, ["id", "name"])

    def test_load_spec_validation(self):
        with patch("openapi_fdw.api.fetch_json", return_value={"paths": {}}):
            spec = load_spec("https://example.test/openapi.json", 5.0, None)
        self.assertIn("paths", spec)

    def test_fetch_json_success(self):
        with patch("openapi_fdw.api.http_fetch", return_value={"ok": True}) as mock_fetch:
            data = fetch_json("https://example.test/api", "post", {"a": 1}, {"X": "1"}, 2.0)
        self.assertEqual(data, {"ok": True})
        mock_fetch.assert_called_once_with("post", "https://example.test/api", {"a": 1}, {"X": "1"}, 2.0)

    def test_fetch_json_error(self):
        with patch(
            "openapi_fdw.api.http_fetch",
            side_effect=openapi_fdw.api.HTTPResponseError("boom"),
        ):
            with self.assertRaises(OpenAPIError):
                fetch_json("https://example.test/api", "get", None, None, 10.0)


class TestOpenAPIWrapper(unittest.TestCase):
    def setUp(self):
        self.openapi_url = "https://spec.test/openapi.json"
        self.server_url = "https://spec.test/api"
        self.path = "/items"
        self.spec = {
            "openapi": "3.0.0",
            "servers": [{"url": self.server_url}],
            "paths": {
                self.path: {
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
                                                    "name": {"type": "string"},
                                                    "price": {"type": "number"},
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
        self.data = [
            {"id": 1, "name": "Widget", "price": 9.99},
            {"id": 2, "name": "Gadget", "price": 14.5},
        ]

    def _patch_requests(self, extra_calls=None):
        from contextlib import ExitStack

        extra_calls = extra_calls or {}

        def _fake_request(url, method, params=None, headers=None, timeout=None):
            if url == self.openapi_url:
                return self.spec
            if url == f"{self.server_url}{self.path}":
                return self.data
            if url in extra_calls:
                return extra_calls[url]
            raise AssertionError(f"Unexpected URL {url}")

        stack = ExitStack()
        stack.enter_context(patch("openapi_fdw.api.fetch_json", side_effect=_fake_request))
        stack.enter_context(patch("openapi_fdw.wrapper.fetch_json", side_effect=_fake_request))
        return stack

    def test_execute_defaults(self):
        with self._patch_requests():
            wrapper = OpenAPIForeignDataWrapper(
                {"openapi_url": self.openapi_url, "path": self.path},
                {"id": {}, "name": {}, "price": {}},
            )
            rows = list(wrapper.execute([], []))
        self.assertEqual(rows, self.data)

    def test_execute_with_data_path_and_query(self):
        spec = json.loads(json.dumps(self.spec))
        spec["paths"][self.path]["get"]["responses"]["200"]["content"]["application/json"]["schema"] = {
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"code": {"type": "string"}, "value": {"type": "integer"}},
                    },
                }
            },
        }
        wrapped_data = {"items": [{"code": "A", "value": 5}]}

        def _fake_request(url, method, params=None, headers=None, timeout=None):
            if url == self.openapi_url:
                return spec
            if url == f"{self.server_url}{self.path}":
                self.assertEqual(params, {"limit": 10})
                return wrapped_data
            raise AssertionError(f"Unexpected URL {url}")

        from contextlib import ExitStack

        with ExitStack() as stack:
            stack.enter_context(patch("openapi_fdw.api.fetch_json", side_effect=_fake_request))
            stack.enter_context(patch("openapi_fdw.wrapper.fetch_json", side_effect=_fake_request))
            wrapper = OpenAPIForeignDataWrapper(
                {
                    "openapi_url": self.openapi_url,
                    "path": self.path,
                    "data_path": "items",
                    "query_params": '{"limit": 10}',
                },
                {"code": {}, "value": {}},
            )
            rows = list(wrapper.execute([], ["code"]))
        self.assertEqual(rows, [{"code": "A"}])

    def test_missing_required_option(self):
        with self.assertRaises(ValueError):
            OpenAPIForeignDataWrapper({}, {"id": {}})

    def test_invalid_response_type_raises(self):
        bad_spec = json.loads(json.dumps(self.spec))
        bad_spec["paths"][self.path]["get"]["responses"]["200"]["content"]["application/json"]["schema"] = {
            "type": "string"
        }

        def _fake_request(url, method, params=None, headers=None, timeout=None):
            if url == self.openapi_url:
                return bad_spec
            raise AssertionError("unexpected url")

        from contextlib import ExitStack

        with ExitStack() as stack:
            stack.enter_context(patch("openapi_fdw.api.fetch_json", side_effect=_fake_request))
            stack.enter_context(patch("openapi_fdw.wrapper.fetch_json", side_effect=_fake_request))
            with self.assertRaises(OpenAPIError):
                OpenAPIForeignDataWrapper({"openapi_url": self.openapi_url, "path": self.path}, {"id": {}})


if __name__ == "__main__":
    unittest.main()
