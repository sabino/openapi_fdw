# Coincap FDW

This is just an idea of how one could write a [Foreign data wrapper](https://wiki.postgresql.org/wiki/Foreign_data_wrappers) in [hy](https://github.com/hylang/hy).

Honestly I don't think the code is ideal but it shows you how to init the subclass considering the required parameters from the superclass.

Also, it is great to be able to use a lisp dialect to achieve this.

If you want to fork this and implement additional things like **INSERT**, **UPDATE** and **DELETE** support or even **COMMIT**, **ROLLBACK** you should take a look at the [Multicorn tutorial](https://multicorn.readthedocs.io/en/latest/implementing-tutorial.html#write-api). It should be very straightforward.

It wouldn't make sense to have these methods for a read only API, but I might try to create another FDW at some point that might have them.

## Usage

1. After installing [multicorn](https://multicorn.org/), you should be able to just install this fdw using pip:

```shell=
pip3 install coincap_fdw
```

2. Then you need to make  sure you've created the extension within the postgres db:

```sql=
CREATE EXTENSION multicorn;
```

3. You should be able to create the server. You can name it however you want, in this case I'm calling it `coincap`:

```sql=
CREATE SERVER coincap FOREIGN DATA WRAPPER multicorn
    options (wrapper 'coincap_fdw.CoinCapForeignDataWrapper');
```

4. You can create the table now. Any additional field you want (comming from the API), just add here as `character varying`. You can also give any name to the table, in this case I'm calling it `crypto_assets`:
```sql=
 CREATE FOREIGN TABLE crypto_assets (
    id character varying,
    name character varying,
    rank character varying,
    symbol character varying,
    priceusd character varying,
    changepercent24hr character varying,
    supply character varying,
    volumeusd24hr character varying
) server coincap;
```

5. After that, just run a query:
```sql=
SELECT name,
       cast(priceusd AS float)
FROM crypto_assets
ORDER BY 2 DESC
LIMIT 10;
```
```
      name       |      priceusd
-----------------+--------------------
 Bitcoin BEP2    | 32172.540563633753
 Bitcoin         |  32161.39844805744
 Wrapped Bitcoin |  32142.40818287929
 yearn.finance   | 28157.691588592475
 Maker           | 2412.0048277811693
 Ethereum        |  2005.716450530913
 Bitcoin Cash    |   433.355437728395
 Compound        |  388.5222106608794
 Binance Coin    |  283.2426902826701
 Aave            |  270.9662792522684
(10 rows)
```
