(import [multicorn [ForeignDataWrapper]]
        [coincap_fdw.api [fetch_endpoint DEFAULT_BASE_URL]])

(defn dict-filter [dict-obj selected-columns]
  (setv lower-cols (lfor col selected-columns (.lower col)))
  (dfor [k v] (.items dict-obj) :if (in lower-cols (.lower k)) [(.lower k) v]))

(defclass CoinCapForeignDataWrapper [ForeignDataWrapper]
  (defn --init-- [self options columns]
    (-> (super CoinCapForeignDataWrapper self) (.--init-- options columns))
    (setv self.columns columns)
    (setv self.base-url (get options "base_url" DEFAULT_BASE_URL))
    (setv self.endpoint (get options "endpoint" "assets"))
    (setv self.api-key (get options "api_key" None)))

  (defn execute [self quals columns]
    (setv assets-list (fetch_endpoint self.endpoint self.base-url self.api-key))
    (lfor asset assets-list (dict-filter asset self.columns)))

