import json
import requests

DEFAULT_BASE_URL = "https://rest.coincap.io/v3"


def fetch_endpoint(endpoint: str, base_url: str = DEFAULT_BASE_URL, api_key: str | None = None):
    """Fetch JSON data from the given CoinCap API endpoint."""
    url = f"{base_url.rstrip('/')}/{endpoint.lstrip('/')}"
    if api_key:
        sep = "?" if "?" not in url else "&"
        url = f"{url}{sep}apiKey={api_key}"
    resp = requests.request("GET", url)
    resp.raise_for_status()
    return json.loads(resp.content.decode("utf-8"))["data"]

