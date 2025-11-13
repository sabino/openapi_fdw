(setv __all__ ["OpenAPIError"
               "fetch-json"
               "load-spec"
               "choose-server-url"
               "get-operation"
               "extract-response-schema"
               "schema-column-order"
               "schema-column-types"
               "apply-data-path"])

(setv collections-abc (__import__ "collections.abc" None None ["Mapping" "MutableMapping"]))
(setv Mapping (. collections-abc Mapping))
(setv MutableMapping (. collections-abc MutableMapping))

(setv http-module (__import__ "openapi_fdw.http" None None ["fetch_json" "HTTPResponseError"]))
(setv http-fetch (. http-module fetch_json))
(setv HTTPResponseError (. http-module HTTPResponseError))


(defclass OpenAPIError [RuntimeError])


(defn fetch-json [url method params headers timeout]
  (setv result (try
    (http-fetch method url params headers timeout)
    (except [HTTPResponseError]
      (raise (OpenAPIError "HTTP request failed")))))
  result)


(defn load-spec [openapi-url timeout headers]
  (setv document (fetch-json openapi-url "get" None headers timeout))
  (when (not (isinstance document Mapping))
    (raise (OpenAPIError "OpenAPI document must be a JSON object")))
  document)


(defn choose-server-url [spec override]
  (if override
    override
    (let [servers (. spec (get "servers"))]
      (if (and servers (isinstance servers list) servers)
        (let [first (get servers 0)]
          (if (and (isinstance first Mapping) (. first (get "url")))
            (. first (get "url"))
            (raise (OpenAPIError "OpenAPI server entry is missing a url"))))
        (raise (OpenAPIError "OpenAPI document does not declare any servers"))))))


(defn get-operation [spec path method]
  (setv paths (. spec (get "paths")))
  (when (not (isinstance paths Mapping))
    (raise (OpenAPIError "OpenAPI document is missing 'paths'")))
  (setv item (. paths (get path)))
  (when (not (isinstance item Mapping))
    (raise (OpenAPIError (.format "OpenAPI document does not define path '{}'" path))))
  (setv op (. item (get method)))
  (when (not (isinstance op Mapping))
    (raise (OpenAPIError (.format "OpenAPI document does not define method '{}' for path '{}'" method path))))
  op)


(defn extract-response-schema [operation]
  (setv responses (. operation (get "responses")))
  (when (not (isinstance responses Mapping))
    (raise (OpenAPIError "Operation is missing responses section")))
  (setv success None)
  (for [[status resp] (.items responses)]
    (when (and (isinstance status str) (. status (startswith "2")))
      (setv success resp)
      (break)))
  (when (not (isinstance success Mapping))
    (raise (OpenAPIError "No success response (2xx) found for operation")))
  (setv content (. success (get "content")))
  (when (not (isinstance content Mapping))
    (raise (OpenAPIError "Success response is missing content section")))
  (setv media (. content (get "application/json")))
  (when (not (isinstance media Mapping))
    (setv media None)
    (for [[_candidate value] (.items content)]
      (when (isinstance value Mapping)
        (setv media value)
        (break))))
  (when (not (isinstance media Mapping))
    (raise (OpenAPIError "No JSON-compatible media type found in response")))
  (setv schema (. media (get "schema")))
  (when (not (isinstance schema Mapping))
    (raise (OpenAPIError "Response content is missing a schema definition")))
  schema)


(defn schema-object [schema]
  (when (not (isinstance schema Mapping))
    (raise (OpenAPIError "Response schema must be a mapping")))
  (setv schema-type (. schema (get "type")))
  (cond
    (= schema-type "array")
    (schema-object (. schema (get "items")))
    (or (not schema-type) (= schema-type "object"))
    schema
    True
    (raise (OpenAPIError (.format "Unsupported schema type '{}'" schema-type)))))


(defn schema-column-order [schema]
  (setv obj (schema-object schema))
  (setv props (. obj (get "properties")))
  (when (not (isinstance props Mapping))
    (raise (OpenAPIError "Object schema is missing properties")))
  (list (.keys props)))


(defn schema-column-types [schema]
  (setv obj (schema-object schema))
  (setv props (. obj (get "properties")))
  (when (not (isinstance props Mapping))
    (raise (OpenAPIError "Object schema is missing properties")))
  (dict
    (lfor [name subschema] (.items props)
          [(.lower name) (. subschema (get "type" "string"))])))


(defn apply-data-path [payload data-path]
  (setv current payload)
  (for [segment data-path]
    (when (not (isinstance current Mapping))
      (raise (OpenAPIError (.format "Segment '{}' did not resolve to an object" segment))))
    (setv current (. current (get segment))))
  current)
