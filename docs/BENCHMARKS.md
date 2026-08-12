# Native runtime benchmark

Recorded: 2026-08-12

These numbers answer one narrow question: does the FDW add a large fixed
runtime tax before network latency? They are not a capacity promise for an
arbitrary public API.

## Environment

- PostgreSQL 18.4, extension compiled in release mode
- Linux 7.0.0-28-generic, x86-64
- Intel Core i7-6820HQ, 4 cores / 8 threads
- Rust 1.93.1 for this constrained local build; the reproducible Docker build
  pins the declared MSRV toolchain, Rust 1.88
- PostgreSQL, FDW, and the deterministic Python HTTP/1.1 server in one Docker
  network namespace
- persistent connections, `TCP_NODELAY`, warm process state
- one API response row containing typed scalars, an array, an object, and an
  undeclared field retained in JSONB

`pgbench` used simple-query mode for ten seconds. Latency log values include the
complete SQL transaction as seen by the client. The first transaction is kept
in the distribution rather than discarded.

## Results

| Workload | Clients | Transactions | Average | Median | p95 | p99 | Throughput |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| typed `id`, one HTTP request | 1 | 11,529 | 0.867 ms | 0.836 ms | 1.129 ms | 1.262 ms | 1,153/s |
| typed `id`, one HTTP request | 8 | 19,389 | 4.124 ms | 3.834 ms | 7.136 ms | 8.961 ms | 1,940/s |
| full JSONB row | 1 | 11,356 | 0.880 ms | 0.840 ms | 1.165 ms | 1.291 ms | 1,136/s |
| typed values plus full JSONB | 1 | 10,971 | 0.911 ms | 0.866 ms | 1.195 ms | 1.325 ms | 1,098/s |
| three rows over two HTTP pages | 1 | 8,591 | 1.164 ms | 1.120 ms | 1.534 ms | 1.681 ms | 859/s |

The largest single-backend values were 16-17 ms and occurred during the first,
cold transaction. The eight-client maximum was 40.046 ms. Every recorded FDW
transaction completed successfully.

A persistent Python HTTP client against the same endpoint averaged 0.313 ms and
3,192 requests/s. A local `SELECT 1` averaged 0.042 ms with one client. Those
controls suggest roughly 0.6 ms for the combined PostgreSQL plan/execution,
FDW callbacks, JSON decoding, coercion, and tuple/result work in the single-row
case. This is an approximation because the control uses a different HTTP
client.

The eight-backend result is substantially affected by the intentionally small
Python test server. It should not be used to infer a Rust client or production
origin's scaling ceiling.

Schema import created three foreign tables from the local OpenAPI 3.1 document
in 17.454 ms cold and 1.531 ms warm. The cold run includes HTTP-client and TLS
provider initialization plus the first spec fetch.

## Reproduction

After running `tests/sql/native_integration.sql` against the deterministic API:

```bash
pgbench -n -c 1 -j 1 -T 10 -l \
  -f bench/local_scan.sql postgres

pgbench -n -c 8 -j 8 -T 10 -l \
  -f bench/local_scan.sql postgres

pgbench -n -c 1 -j 1 -T 10 -l \
  -f bench/jsonb_scan.sql postgres

pgbench -n -c 1 -j 1 -T 10 -l \
  -f bench/typed_jsonb_scan.sql postgres

pgbench -n -c 1 -j 1 -T 10 -l \
  -f bench/paginated_scan.sql postgres
```

Keep PostgreSQL major, payload, server protocol, connection state, client count,
and machine power policy fixed when comparing commits. Public internet latency
usually dwarfs the sub-millisecond local extension cost.
