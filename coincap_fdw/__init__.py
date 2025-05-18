try:
    import hy  # optional; allows importing .hy sources when available
except Exception:  # pragma: no cover - hy not installed in minimal env
    hy = None

from .wrapper import CoinCapForeignDataWrapper
