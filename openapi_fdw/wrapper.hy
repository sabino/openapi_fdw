(setv __all__ ["OpenAPIForeignDataWrapper"])

(setv collections-abc (__import__ "collections.abc" None None ["Mapping" "MutableMapping"]))
(setv Mapping (. collections-abc Mapping))
(setv MutableMapping (. collections-abc MutableMapping))

(setv json-module (__import__ "json"))
(setv loads (. json-module loads))
(setv JSONDecodeError (. json-module JSONDecodeError))

(setv requests (__import__ "requests"))

(setv api-module (__import__ "openapi_fdw.api" None None ["OpenAPIError" "fetch_json" "load_spec" "choose_server_url" "get_operation" "extract_response_schema" "schema_column_order" "apply_data_path"]))
(setv OpenAPIError (. api-module OpenAPIError))
(setv fetch_json (. api-module fetch_json))
(setv load-spec (. api-module load_spec))
(setv choose-server-url (. api-module choose_server_url))
(setv get-operation (. api-module get_operation))
(setv extract-response-schema (. api-module extract_response_schema))
(setv schema-column-order (. api-module schema_column_order))
(setv apply-data-path (. api-module apply_data_path))

(setv ForeignDataWrapper (. (__import__ "multicorn") ForeignDataWrapper))


(defn parse-json-object [raw label]
  (if (not raw)
    None
    (do
      (try
        (setv parsed (loads raw))
        (except [JSONDecodeError]
          (raise (ValueError (.format "{} must be valid JSON" label)))))
      (if (isinstance parsed MutableMapping)
        (dict parsed)
        (raise (ValueError (.format "{} must be a JSON object" label)))))))


(defn parse-data-path [raw]
  (if (not raw)
    (tuple [])
    (tuple (lfor segment (.split raw "/")
                 :if (.strip segment)
                 (.strip segment)))))


(defn build-column-map [column-names]
  (dict (lfor name column-names [(.lower name) name])))


(defn filter-row [row requested-map]
  (when (not (isinstance row Mapping))
    (raise (OpenAPIError "Row returned by API is not a JSON object")))
  (setv lowered (dict (lfor [k v] (.items row) [(.lower k) v])))
  (dict
    (lfor [lower-name original-name] (.items requested-map)
          [original-name (. lowered (get lower-name None))])))


(defn normalize-dataset [payload data-path]
  (setv resolved (apply-data-path payload data-path))
  (when (not (isinstance resolved list))
    (raise (OpenAPIError "Expected response payload to be a JSON array")))
  resolved)


(defn join-url [base path]
  (+ (.rstrip base "/") "/" (.lstrip path "/")))


(defclass OpenAPIForeignDataWrapper [ForeignDataWrapper]
  (defn __init__ [self options columns]
    (. (super OpenAPIForeignDataWrapper self) (__init__ options columns))
    (setv openapi-url (. options (get "openapi_url")))
    (when (not openapi-url)
      (raise (ValueError "openapi_url option is required")))
    (setv timeout (float (. options (get "timeout" 10.0))))
    (setv headers (parse-json-object (. options (get "headers")) "headers"))
    (setv spec (load-spec openapi-url timeout headers))
    (setv server-url (choose-server-url spec (. options (get "server_url"))))
    (setv path (. options (get "path")))
    (when (not path)
      (raise (ValueError "path option is required")))
    (setv method (.lower (str (. options (get "method" "get")))))
    (setv operation (get-operation spec path method))
    (setv response-schema (extract-response-schema operation))
    (setv data-path (parse-data-path (. options (get "data_path"))))
    (when (and (= (len data-path) 0) (not (= (. response-schema (get "type")) "array")))
      (raise (OpenAPIError "data_path option is required when the response schema is not an array")))
    (setv column-order (schema-column-order response-schema))
    (setv self._default-column-order column-order)
    (setv self._default-column-map (build-column-map column-order))
    (setv self._columns columns)
    (setv self._server-url server-url)
    (setv self._path path)
    (setv self._method method)
    (setv self._timeout timeout)
    (setv self._headers headers)
    (setv self._data-path (if (> (len data-path) 0) data-path (tuple [])))
    (setv self._query-params (parse-json-object (. options (get "query_params")) "query_params")))

  (defn _prepare-requested-columns [self supplied]
    (if (not supplied)
      self._default-column-map
      (do
        (setv names (if (isinstance supplied Mapping) (.keys supplied) supplied))
        (setv requested {})
        (for [name names]
          (setv lower (.lower name))
          (setv canonical (. self._default-column-map (get lower name)))
          (. requested (update {lower canonical})))
        requested)))

  (defn execute [self quals columns]
    (setv requested-map (. self (_prepare-requested-columns columns)))
    (setv url (join-url self._server-url self._path))
    (setv payload (fetch_json url self._method self._query-params self._headers self._timeout))
    (setv records (normalize-dataset payload self._data-path))
    (for [row records]
      (yield (filter-row row requested-map))))
  )
