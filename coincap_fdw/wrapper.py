from multicorn import ForeignDataWrapper
from .api import fetch_endpoint, DEFAULT_BASE_URL


def dict_filter(dict_obj, selected_columns):
    lower_columns = [col.lower() for col in selected_columns]
    return {k.lower(): v for k, v in dict_obj.items() if k.lower() in lower_columns}


class CoinCapForeignDataWrapper(ForeignDataWrapper):
    def __init__(self, options, columns):
        super().__init__(options, columns)
        self.columns = columns
        self.base_url = options.get("base_url", DEFAULT_BASE_URL)
        self.endpoint = options.get("endpoint", "assets")

    def execute(self, quals, columns):
        assets_list = fetch_endpoint(self.endpoint, self.base_url)
        return [dict_filter(asset, self.columns) for asset in assets_list]

