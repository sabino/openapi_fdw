# OpenAPI Foreign Data Wrapper

The OpenAPI FDW converts any OpenAPI/Swagger specification into a read-only schema that can be queried from PostgreSQL via [Multicorn](https://multicorn.org/). For each path + method combination you configure, the wrapper inspects the JSON response schema defined in the OpenAPI document, maps the response fields to columns, and issues HTTP requests at query time to return live data.

The entire FDW implementation is authored in [Hy](https://github.com/hylang/hy), with a tiny Python shim to expose the Hy class to PostgreSQL.

## Highlights

- Works with any OpenAPI 3.x document reachable over HTTP/HTTPS.
- Automatically infers columns from the response schema; optional case-insensitive projection for narrower tables.
- Supports nested response payloads through configurable data paths and passes JSON options (headers, query params) directly to the target API.
- Hy-only codebase, making it a concise reference for implementing Multicorn FDWs in a Lisp syntax.

## Getting Started

Install dependencies and run the test suite using [uv](https://github.com/astral-sh/uv):

```bash
uv sync
uv run python -m unittest discover -s tests -v
```

### Basic Usage

```sql
CREATE EXTENSION multicorn;

CREATE SERVER petstore
  FOREIGN DATA WRAPPER multicorn
  OPTIONS (
    wrapper 'openapi_fdw.OpenAPIForeignDataWrapper',
    openapi_url 'https://petstore3.swagger.io/api/v3/openapi.json',
    path '/pet',
    method 'get'
  );

CREATE FOREIGN TABLE pets (
  id integer,
  name text,
  status text
) SERVER petstore;
```

Each query against `pets` fetches the OpenAPI specification (cached by Multicorn) and the corresponding endpoint, then returns JSON objects mapped to the requested columns.

### Configuration Options

| Option         | Description                                                                                 | Default              |
|----------------|---------------------------------------------------------------------------------------------|----------------------|
| `openapi_url`  | URL pointing to the OpenAPI document (JSON).                                                | _required_           |
| `path`         | Path template (as defined in the spec) to query.                                            | _required_           |
| `method`       | HTTP method to use when calling the path.                                                   | `get`                |
| `server_url`   | Override for the server URL; falls back to the first `servers` entry in the document.      | first server url     |
| `data_path`    | Slash-separated path inside the JSON payload that resolves to the array of rows.           | root (when schema is an array) |
| `query_params` | JSON object defining static query string parameters sent with every request.               | none                 |
| `headers`      | JSON object containing HTTP headers to attach to both the spec and data requests.          | none                 |
| `timeout`      | Request timeout in seconds used for both the spec fetch and data queries.                  | `10.0`               |

Response payloads must ultimately resolve to a JSON array of objects after applying `data_path`.

## Project Layout

```
openapi_fdw/
├── openapi_fdw/
│   ├── __init__.py      # Python shim exposing the Hy wrapper
│   ├── api.hy           # OpenAPI parsing helpers and HTTP utilities
│   └── wrapper.hy       # Hy implementation of the Multicorn FDW
├── tests/               # Unit, integration, and docker-based smoke tests
├── pyproject.toml       # Project metadata / uv configuration
├── requirements.txt     # Runtime dependencies
├── Dockerfile           # Postgres + Multicorn test image
└── README.md            # This document
```

## License

Distributed under the terms of the WTFPL as declared in `setup.py`.
