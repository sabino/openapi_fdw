# Guidance for AAzqasz

Welcome to **CoinCap Foreign Data Wrapper (FDW)**. This project exposes cryptocurrency data from [CoinCap](https://coincap.io) through PostgreSQL using the Multicorn FDW framework. It is implemented primarily in Python with a small portion in [Hy](https://github.com/hylang/hy), a Lisp dialect that compiles to Python.

## What the project does

CoinCap FDW lets you create a PostgreSQL foreign table that proxies a CoinCap API endpoint (by default `https://api.coincap.io/v2/assets`). Queries against this table trigger HTTP requests to fetch the data and return the JSON fields matching the table columns. The wrapper is read‐only and meant as a simple reference implementation for FDW development.

Key components:

- `coincap_fdw/api.py` &ndash; minimal helper to perform HTTP requests to a CoinCap endpoint.
- `coincap_fdw/wrapper.py` and `coincap_fdw/wrapper.hy` &ndash; the `CoinCapForeignDataWrapper` class used by Multicorn. The Hy source mirrors the Python version.
- `tests/` &ndash; unit and integration tests showing how the wrapper is expected to behave.

## How it works

1. Install the package (and Multicorn) in your PostgreSQL environment:
   ```bash
   pip install coincap-fdw
   ```
2. Enable Multicorn and define a server pointing at the wrapper class:
   ```sql
   CREATE EXTENSION multicorn;
   CREATE SERVER coincap
       FOREIGN DATA WRAPPER multicorn
       OPTIONS (wrapper 'coincap_fdw.CoinCapForeignDataWrapper',
                base_url 'https://api.coincap.io/v2',
                endpoint 'assets');
   ```
   The `base_url` and `endpoint` options are configurable when you create the server.
3. Create a foreign table describing the columns you want to expose and query it like a normal table. Each query fetches from the configured CoinCap endpoint.

For a full example see `README.md`.

## Running tests

The project uses the built-in `unittest` module. Run all tests with:
```bash
python -m unittest discover -s tests -v
```
This command executes both unit and integration tests without requiring the real Multicorn extension.

## Repository layout

```
coincap_fdw/
├── coincap_fdw/      # Source package
│   ├── __init__.py   # Package initializer
│   ├── api.py        # API helper functions
│   ├── wrapper.hy    # FDW implementation in Hy
│   └── wrapper.py    # FDW implementation in Python
├── tests/            # Unit and integration tests
├── requirements.txt  # Runtime dependencies
├── setup.py          # Packaging metadata
└── README.md         # Project overview and usage
```

This should give you enough context to start exploring the code. The wrapper itself is small and can serve as a template for other FDWs.
