# CoinCap Foreign Data Wrapper

CoinCap FDW demonstrates how to expose the [CoinCap](https://coincap.io) API through PostgreSQL using [Multicorn](https://multicorn.org/) and the [Hy](https://github.com/hylang/hy) Lisp dialect. The project is intentionally small and aims to serve as a learning reference for writing Foreign Data Wrappers.

## Features

- Read-only access to cryptocurrency asset data from CoinCap.
- Implemented in Hy and distributed as a standard Python package.

## Installation

1. Install Multicorn in your PostgreSQL instance.
2. Install the FDW package using pip:

```bash
pip install coincap-fdw
```

## Usage

Create the Multicorn extension and server, then define a foreign table to query the assets endpoint.

```sql
-- enable multicorn
CREATE EXTENSION multicorn;

-- create a server that points at the wrapper class
CREATE SERVER coincap
    FOREIGN DATA WRAPPER multicorn
    OPTIONS (wrapper 'coincap_fdw.CoinCapForeignDataWrapper');

-- define a foreign table using the server
CREATE FOREIGN TABLE crypto_assets (
    id TEXT,
    name TEXT,
    rank TEXT,
    symbol TEXT,
    priceusd TEXT,
    changepercent24hr TEXT,
    supply TEXT,
    volumeusd24hr TEXT
) SERVER coincap;
```

Querying the table will fetch data from the API:

```sql
SELECT name,
       CAST(priceusd AS FLOAT)
FROM crypto_assets
ORDER BY 2 DESC
LIMIT 10;
```

## Project Layout

```
coincap_fdw/
├── src/              # Python/Hy source package
│   ├── __init__.py   # Package initializer
│   └── wrapper.hy    # Hy implementation of the FDW
├── requirements.txt  # Runtime dependencies
├── setup.py          # Packaging metadata
└── README.md         # Project documentation (this file)
```

The `wrapper.hy` file contains the `CoinCapForeignDataWrapper` class which makes HTTP requests to the CoinCap API and maps the response to the requested table columns. The package can be installed from source or via pip and is usable anywhere Multicorn is available.

## License

This project is distributed under the terms of the WTFPL license as declared in `setup.py`.
