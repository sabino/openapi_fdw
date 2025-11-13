try:
    import hy  # noqa: F401; ensure Hy importer available
except Exception:  # pragma: no cover - Hy may be absent in minimal env
    hy = None  # type: ignore

from .wrapper import OpenAPIForeignDataWrapper

__all__ = ["OpenAPIForeignDataWrapper"]
