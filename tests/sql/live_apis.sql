\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS openapi_fdw;

CREATE SERVER live_pokeapi
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://pokeapi.co/api/v2',
    request_timeout_ms '20000',
    max_retries '2'
  );
CREATE FOREIGN TABLE live_pokemon (
  name text,
  height bigint,
  weight bigint,
  attrs jsonb
)
SERVER live_pokeapi
OPTIONS (
  endpoint '/pokemon/{name}',
  pagination 'none',
  limit_param ''
);

CREATE SERVER live_pokeapi_spec
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://pokeapi.co',
    spec_url 'https://raw.githubusercontent.com/PokeAPI/pokeapi/master/openapi.yml',
    request_timeout_ms '30000',
    max_retries '2'
  );
CREATE SCHEMA live_poke_import;
IMPORT FOREIGN SCHEMA api
  LIMIT TO (pokemon_list)
  FROM SERVER live_pokeapi_spec
  INTO live_poke_import
  OPTIONS (methods 'GET', include_attrs 'true');

CREATE SERVER live_brasilapi
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://brasilapi.com.br/api',
    request_timeout_ms '20000',
    max_retries '2'
  );
CREATE FOREIGN TABLE live_cep (
  cep text,
  state text,
  city text,
  street text,
  attrs jsonb
)
SERVER live_brasilapi
OPTIONS (
  endpoint '/cep/v1/{cep}',
  pagination 'none',
  limit_param ''
);

CREATE SERVER live_weather_gov
  FOREIGN DATA WRAPPER openapi_fdw
  OPTIONS (
    base_url 'https://api.weather.gov',
    user_agent 'openapi_fdw live test (https://github.com/sabino/openapi_fdw)',
    request_timeout_ms '20000',
    max_retries '2'
  );
CREATE FOREIGN TABLE live_weather_point (
  point text,
  grid_id text,
  grid_x bigint,
  grid_y bigint,
  forecast text,
  attrs jsonb
)
SERVER live_weather_gov
OPTIONS (
  endpoint '/points/{point}',
  object_path '/properties',
  pagination 'none',
  limit_param ''
);

DO $test$
DECLARE
  value text;
  number bigint;
BEGIN
  SELECT attrs #>> '{types,0,type,name}', height
    INTO value, number
    FROM live_pokemon
   WHERE name = 'ditto';
  IF value <> 'normal' OR number <> 3 THEN
    RAISE EXCEPTION 'PokéAPI smoke test returned %, %', value, number;
  END IF;

  SELECT name INTO value
    FROM live_poke_import.pokemon_list
   LIMIT 1;
  IF value <> 'bulbasaur' THEN
    RAISE EXCEPTION 'PokéAPI OpenAPI import returned %', value;
  END IF;

  SELECT city INTO value
    FROM live_cep
   WHERE cep = '01001000';
  IF value <> 'São Paulo' THEN
    RAISE EXCEPTION 'BrasilAPI smoke test returned %', value;
  END IF;

  SELECT grid_id INTO value
    FROM live_weather_point
   WHERE point = '39.7456,-97.0892';
  IF value <> 'TOP' THEN
    RAISE EXCEPTION 'api.weather.gov smoke test returned %', value;
  END IF;
END
$test$;

SELECT 'three live HTTPS APIs and a real OpenAPI import passed' AS result;
