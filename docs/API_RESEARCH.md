# Public API research and live validation

Research date: 2026-08-12

The deterministic test server is the merge oracle, but it cannot prove that the
FDW handles the conventions used by independent services. The live suite uses
three APIs with different contracts and operators.

## Selected APIs

| Service | Access and contract | What it validates | Observed result |
| --- | --- | --- | --- |
| [PokéAPI](https://pokeapi.co/docs/v2) | Public GET API, no authentication; official OpenAPI 3.1 YAML is maintained in the [source repository](https://github.com/PokeAPI/pokeapi/blob/master/openapi.yml) | YAML, local `$ref`, `results` envelope, `next` URL pagination, path parameters, typed and nested fields | Imported `pokemon_list`; live query returned Bulbasaur, Ivysaur, and Venusaur. Manual detail query returned Ditto's height, weight, and nested primary type. |
| [US National Weather Service](https://www.weather.gov/documentation/services-web-api) | US government open data; requires an identifying User-Agent; publishes a live OpenAPI endpoint and commonly returns GeoJSON | Required headers, structured `+json` media types, path parameters containing comma/decimal values, nested `properties`, live HTTPS | Point `39.7456,-97.0892` resolved to office `TOP`, grid 32/81, and a forecast URL. |
| [BrasilAPI](https://github.com/BrasilAPI/BrasilAPI) | Public, community-run Brazilian data API; no OpenAPI document was found in its main repository | Manual JSONB/typed table path when a service has no usable spec, UTF-8 text, Brazilian data | CEP `01001000` returned São Paulo/SP and Praça da Sé. |

PokéAPI asks consumers to cache and follow its fair-use policy even though it
does not currently require authentication or enforce a published rate limit.
NWS documents reasonable unpublished rate limits and transient retry behavior.
The scheduled suite therefore makes only a handful of requests; it is not a
load test against public infrastructure.

## Why these three

- They are independently operated, so passing is not an artifact of one API
  gateway or JSON style.
- They cover an OpenAPI-first service, a government GeoJSON/OpenAPI service,
  and a useful service without a published contract.
- None requires a repository secret, billing account, or trial token.
- Their selected fixtures are long-lived identifiers rather than current
  prices, news, or other intentionally volatile facts.

GitHub REST remains a useful opt-in authentication/Link-pagination exercise,
but it is not in the default live suite because unauthenticated quotas and
shared-runner egress can make it noisy. CoinCap was not retained as the default
fixture because its current production API path is key-oriented and the old
project contract was already stale.

## Reliability boundary

Public endpoints can be unavailable, rate-limited, or changed without a commit
to this repository. Consequently:

- deterministic local HTTP plus real PostgreSQL 14-18 is required on every PR;
- public API checks run in a separate scheduled/manual workflow;
- live failure is diagnostic, not proof of a local regression by itself; and
- benchmark traffic is sent only to the local test service.
