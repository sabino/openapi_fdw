from __future__ import annotations

import requests


class HTTPResponseError(RuntimeError):
    """Raised when an HTTP request cannot be satisfied."""


def fetch_json(method: str, url: str, params=None, headers=None, timeout: float = 10.0):
    """Perform an HTTP request and return the decoded JSON payload."""
    try:
        response = requests.request(method, url, params=params, headers=headers, timeout=timeout)
    except requests.RequestException as exc:
        raise HTTPResponseError(f"Request to {url} failed: {exc}") from exc

    if response.status_code >= 400:
        raise HTTPResponseError(f"Request to {url} returned {response.status_code}")

    try:
        return response.json()
    except ValueError as exc:  # pragma: no cover - delegated to caller tests
        raise HTTPResponseError("Response did not contain valid JSON") from exc
