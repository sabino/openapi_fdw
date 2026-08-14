# Public API research and live validation

Research dates: 2026-08-12 and 2026-08-14

The deterministic test server is the merge oracle, but it cannot prove that the
FDW handles the conventions used by independent services. The live suite uses
three APIs with different contracts and operators.

## Selected APIs

| Service | Access and contract | What it validates | Observed result |
| --- | --- | --- | --- |
| [PokéAPI](https://pokeapi.co/docs/v2) | Public GET API, no authentication; official OpenAPI 3.1 YAML is maintained in the [source repository](https://github.com/PokeAPI/pokeapi/blob/master/openapi.yml) | YAML, local `$ref`, `results` envelope, `next` URL pagination, path parameters, typed and nested fields | Imported `pokemon_list`; live query returned Bulbasaur, Ivysaur, and Venusaur. Manual detail query returned Ditto's height, weight, and nested primary type. |
| [US National Weather Service](https://www.weather.gov/documentation/services-web-api) | US government open data; requires an identifying User-Agent; publishes a live OpenAPI endpoint and commonly returns GeoJSON | Required headers, structured `+json` media types, path parameters containing comma/decimal values, nested `properties`, live HTTPS | Point `39.7456,-97.0892` resolved to office `TOP`, grid 32/81, and a forecast URL. |
| [BrasilAPI](https://github.com/BrasilAPI/BrasilAPI) | Public, community-run Brazilian data API; its documentation is rendered from contract fragments but provides no stable raw OpenAPI URL | A small repository-supplied OpenAPI 3.1 adapter contract, direct arrays, single objects, path parameters, UTF-8 text, and JSONB evolution | Banks, rates, brokers, and CEP endpoints were checked against their live contracts; CEP `01001000` returned São Paulo/SP and Praça da Sé. |
| [RESTful API.dev](https://restful-api.dev/) | Public object CRUD API, no authentication for the legacy public endpoints, with a documented 50-request daily allowance | Real POST/GET/PUT/PATCH/DELETE contracts, generated string identities, arbitrary nested `data`, and cleanup behavior | A disposable object was created, fetched by ID, replaced, and deleted successfully on 2026-08-14. The global list did not expose that fresh object, so the automated public workflow preserves the create response ID explicitly. |

PokéAPI asks consumers to cache and follow its fair-use policy even though it
does not currently require authentication or enforce a published rate limit.
NWS documents reasonable unpublished rate limits and transient retry behavior.
The scheduled suite therefore makes only a handful of requests; it is not a
load test against public infrastructure.

## Why these services

- They are independently operated, so passing is not an artifact of one API
  gateway or JSON style.
- They cover an OpenAPI-first service, a government GeoJSON/OpenAPI service,
  and a useful service whose rendered documentation needs a directly fetchable
  adapter document for automated import.
- None requires a repository secret, billing account, or trial token.
- Their selected fixtures are long-lived identifiers rather than current
  prices, news, or other intentionally volatile facts.

GitHub REST remains a useful opt-in authentication/Link-pagination exercise,
but it is not in the default live suite because unauthenticated quotas and
shared-runner egress can make it noisy. CoinCap was not retained as the default
fixture because its current production API path is key-oriented and the old
project contract was already stale.

RESTful API.dev is kept out of the scheduled read smoke test because it mutates
shared public infrastructure and has a low daily allowance. Its separate manual
workflow creates a uniquely named disposable object, passes the returned ID to
PostgreSQL, validates PATCH/GET/DELETE through the FDW, and accepts only a
successful deletion or an already-cleaned 404 in its final cleanup step. The
repository-supplied OpenAPI file is a small adapter contract transcribed from the
service's public documentation because the site does not publish a stable raw
specification URL.

## Reliability boundary

Public endpoints can be unavailable, rate-limited, or changed without a commit
to this repository. Consequently:

- deterministic local HTTP plus real PostgreSQL 14-18 is required on every PR;
- public API checks run in a separate scheduled/manual workflow;
- live failure is diagnostic, not proof of a local regression by itself; and
- benchmark traffic is sent only to the local test service.
