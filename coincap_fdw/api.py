import json
import requests

DEFAULT_BASE_URL = "https://api.coincap.io/v2"


def fetch_endpoint(endpoint: str, base_url: str = DEFAULT_BASE_URL):
    """Fetch JSON data from the given CoinCap API endpoint."""
    url = f"{base_url.rstrip('/')}/{endpoint.lstrip('/')}"
    resp = requests.request("GET", url)
    resp.raise_for_status()
    return json.loads(resp.content.decode("utf-8"))["data"]

