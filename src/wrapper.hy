(import [multicorn [ForeignDataWrapper]]
        [requests [request]]
        json)

(defn in-list?
  [elements-list search-element]
  (>= (.count elements-list search-element) 1))

(defn dict-filter
  [dict-obj selected-columns]
  (setv lower-columns (lfor col selected-columns (.lower col)))
  (dfor [key value]
         (.items dict-obj) :if (in-list? lower-columns (.lower key))
         [(.lower key) value]))

(defclass CoinCapForeignDataWrapper
  [ForeignDataWrapper]

  (defn --init--
    [self options columns]
    (setv self.columns columns)
    (-> (super CoinCapForeignDataWrapper self)
        (.--init-- options columns)))

  (defn execute
    [self options columns]
    (setv assets-list (-> (request "GET" "https://api.coincap.io/v2/assets")
                          (. content)
                          (.decode "utf-8")
                          json.loads
                          (get "data")))
    (lfor asset assets-list (dict-filter asset self.columns))))
