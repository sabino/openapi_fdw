#!/usr/bin/env python3
"""Deterministic HTTP API used by the native PostgreSQL integration tests."""

from __future__ import annotations

import argparse
import json
import socket
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlsplit


ITEMS = [
    {
        "id": 1,
        "displayName": "Alpha",
        "price": 9.5,
        "active": True,
        "createdAt": "2026-08-10T10:00:00Z",
        "tags": ["one", "fast"],
        "meta": {"color": "red"},
        "futureField": "kept without DDL changes",
    },
    {
        "id": 2,
        "displayName": "Beta",
        "price": 14.25,
        "active": False,
        "createdAt": "2026-08-11T11:30:00Z",
        "tags": ["two"],
        "meta": {"color": "blue"},
        "futureField": "also kept",
    },
    {
        "id": 3,
        "displayName": None,
        "price": 20.0,
        "active": True,
        "createdAt": "2026-08-12T12:45:00Z",
        "tags": [],
        "meta": {"color": "green"},
        "futureField": "still kept",
    },
]


class State:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.requests: list[dict[str, object]] = []
        self.flaky_attempts = 0

    def record(self, handler: BaseHTTPRequestHandler, body: object | None) -> None:
        split = urlsplit(handler.path)
        if split.path in {"/health", "/api/__requests"}:
            return
        entry = {
            "method": handler.command,
            "path": split.path,
            "query": parse_qs(split.query, keep_blank_values=True),
            "body": body,
            "userAgent": handler.headers.get("user-agent"),
            "testHeader": handler.headers.get("x-test-header"),
            "hasApiKey": handler.headers.get("x-api-key") is not None,
            "validEnvAuth": (
                handler.headers.get("x-env-header") == "header-from-environment"
                and handler.headers.get("x-env-api-key") == "key-from-environment"
            ),
        }
        with self.lock:
            self.requests.append(entry)
            # Benchmarks can issue thousands of scans; retain enough evidence
            # for assertions without letting the test process grow unbounded.
            if len(self.requests) > 2_000:
                del self.requests[:-2_000]

    def snapshot(self) -> list[dict[str, object]]:
        with self.lock:
            return list(self.requests)


STATE = State()


def openapi_document(origin: str) -> dict[str, object]:
    return {
        "openapi": "3.1.0",
        "info": {"title": "openapi_fdw integration API", "version": "1.0.0"},
        "servers": [{"url": f"{origin}/api"}],
        "paths": {
            "/items": {
                "get": {
                    "operationId": "listItems",
                    "responses": {
                        "200": {
                            "description": "A cursor-paginated item collection",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/ItemPage"}
                                }
                            },
                        }
                    },
                }
            },
            "/by-slug/{slug}": {
                "get": {
                    "operationId": "getBySlug",
                    "parameters": [
                        {
                            "name": "slug",
                            "in": "path",
                            "required": True,
                            "schema": {"type": "string"},
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "A path-selected object",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "properties": {"name": {"type": "string"}}
                                    }
                                }
                            },
                        }
                    },
                }
            },
            "/stations": {
                "get": {
                    "operationId": "listStations",
                    "responses": {
                        "200": {
                            "description": "GeoJSON station features",
                            "content": {
                                "application/geo+json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "features": {
                                                "type": "array",
                                                "items": {
                                                    "type": "object",
                                                    "properties": {
                                                        "geometry": {"type": "object"},
                                                        "properties": {
                                                            "type": "object",
                                                            "properties": {
                                                                "stationIdentifier": {
                                                                    "type": "string"
                                                                },
                                                                "name": {"type": "string"},
                                                            },
                                                        },
                                                    },
                                                },
                                            }
                                        },
                                    }
                                }
                            },
                        }
                    },
                }
            },
        },
        "components": {
            "schemas": {
                "Item": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer", "format": "int64"},
                        "displayName": {"type": ["string", "null"]},
                        "price": {"type": "number", "format": "double"},
                        "active": {"type": "boolean"},
                        "createdAt": {"type": "string", "format": "date-time"},
                        "tags": {"type": "array", "items": {"type": "string"}},
                        "meta": {"type": "object", "additionalProperties": True},
                    },
                },
                "ItemPage": {
                    "type": "object",
                    "properties": {
                        "results": {
                            "type": "array",
                            "items": {"$ref": "#/components/schemas/Item"},
                        },
                        "next": {"type": ["string", "null"]},
                    },
                },
            }
        },
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "openapi-fdw-test/1"
    protocol_version = "HTTP/1.1"

    def setup(self) -> None:
        super().setup()
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    def log_message(self, _format: str, *args: object) -> None:
        return

    @property
    def origin(self) -> str:
        advertised = getattr(self.server, "advertised_origin", None)
        if advertised:
            return advertised
        host, port = self.server.server_address
        return f"http://{host}:{port}"

    def send_json(
        self,
        value: object,
        status: HTTPStatus = HTTPStatus.OK,
        extra_headers: dict[str, str] | None = None,
        content_type: str = "application/json",
    ) -> None:
        payload = json.dumps(value, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(payload)))
        for name, header_value in (extra_headers or {}).items():
            self.send_header(name, header_value)
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:  # noqa: N802
        split = urlsplit(self.path)
        query = parse_qs(split.query, keep_blank_values=True)
        STATE.record(self, None)

        if split.path == "/health":
            self.send_json({"ok": True})
            return
        if split.path == "/openapi.json":
            self.send_json(openapi_document(self.origin))
            return
        if split.path == "/api/__requests":
            self.send_json(STATE.snapshot())
            return
        if split.path == "/api/items":
            rows = ITEMS
            if "id" in query:
                requested_id = int(query["id"][0])
                rows = [row for row in rows if row["id"] == requested_id]
            offset = int(query.get("cursor", ["0"])[0])
            page_size = int(query.get("limit", ["2"])[0])
            page_size = max(1, min(page_size, 100))
            page = rows[offset : offset + page_size]
            next_cursor = offset + len(page)
            next_url = f"?cursor={next_cursor}" if next_cursor < len(rows) else None
            self.send_json({"results": page, "next": next_url})
            return
        if split.path.startswith("/api/by-slug/"):
            slug = unquote(split.path.removeprefix("/api/by-slug/"))
            self.send_json({"name": f"resolved:{slug}"})
            return
        if split.path == "/api/stations":
            self.send_json(
                {
                    "type": "FeatureCollection",
                    "features": [
                        {
                            "type": "Feature",
                            "geometry": {"type": "Point", "coordinates": [-122.3, 47.4]},
                            "properties": {
                                "stationIdentifier": "KSEA",
                                "name": "Seattle",
                                "futureObservation": 42,
                            },
                        }
                    ],
                }
            )
            return
        if split.path == "/api/flaky":
            with STATE.lock:
                STATE.flaky_attempts += 1
                attempt = STATE.flaky_attempts
            if attempt == 1:
                self.send_json(
                    {"error": "retry me"},
                    HTTPStatus.SERVICE_UNAVAILABLE,
                    {"retry-after": "0"},
                )
            else:
                self.send_json([{"id": 99}])
            return
        if split.path == "/api/cross-origin":
            self.send_json(
                {
                    "results": [{"id": 1}],
                    "next": "https://example.invalid/credential-sink",
                }
            )
            return
        if split.path == "/api/loop":
            self.send_json({"results": [{"id": 1}], "next": "?cursor=same"})
            return
        if split.path == "/api/bad-type":
            self.send_json([{"id": "not-an-integer"}])
            return
        if split.path == "/api/wrong-content-type":
            self.send_json([{"id": 1}], content_type="text/html")
            return
        if split.path == "/api/large":
            self.send_json([{"payload": "x" * 4096}])
            return

        self.send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:  # noqa: N802
        length = min(int(self.headers.get("content-length", "0")), 1024 * 1024)
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            self.send_json({"error": "invalid json"}, HTTPStatus.BAD_REQUEST)
            return
        STATE.record(self, body)
        if urlsplit(self.path).path == "/api/search":
            self.send_json({"results": [{"term": body.get("term"), "found": True}]})
            return
        self.send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18080)
    parser.add_argument(
        "--advertise-origin",
        help="origin placed in the OpenAPI servers list (useful across Docker)",
    )
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.advertised_origin = args.advertise_origin
    server.serve_forever()


if __name__ == "__main__":
    main()
