use crate::model::{
    Bundle, ColumnState, ControlState, Discovery, MutationResult, SampleQuery, SampleResult,
    SourceDefinition, SourceState, TableState, validate_identifier,
};
use crate::sql::{drop_server_sql, sample_sql, source_plan};
use anyhow::{Context, Result, anyhow, bail};
use deadpool_postgres::{GenericClient, Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio_postgres::NoTls;
use tracing::warn;
use uuid::Uuid;

pub fn create_pool(database_url: &str, max_size: usize) -> Result<Pool> {
    let postgres = database_url
        .parse()
        .context("DATABASE_URL is not a valid PostgreSQL connection string")?;
    let manager = Manager::from_config(
        postgres,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Verified,
        },
    );
    Pool::builder(manager)
        .max_size(max_size)
        .runtime(Runtime::Tokio1)
        .build()
        .context("could not create PostgreSQL connection pool")
}

pub async fn bootstrap(pool: &Pool) -> Result<()> {
    let client = pool
        .get()
        .await
        .context("could not connect to PostgreSQL")?;
    client
        .batch_execute(
            "CREATE SCHEMA IF NOT EXISTS openapi_fdw_control;
             CREATE TABLE IF NOT EXISTS openapi_fdw_control.sources (
                 server_name text PRIMARY KEY,
                 definition jsonb NOT NULL,
                 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
                 updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
             );
             REVOKE ALL ON SCHEMA openapi_fdw_control FROM PUBLIC;
             REVOKE ALL ON TABLE openapi_fdw_control.sources FROM PUBLIC;",
        )
        .await
        .context("could not initialize control-plane metadata")?;
    Ok(())
}

pub async fn health(pool: &Pool) -> Result<()> {
    let client = pool.get().await.context("database pool is unavailable")?;
    client
        .simple_query("SELECT 1")
        .await
        .context("database health query failed")?;
    Ok(())
}

pub async fn state(pool: &Pool) -> Result<ControlState> {
    let client = pool.get().await.context("database pool is unavailable")?;
    let version_row = client
        .query_one(
            "SELECT current_setting('server_version'),
                    (SELECT extversion FROM pg_extension WHERE extname = 'openapi_fdw')",
            &[],
        )
        .await?;
    let postgres_version: String = version_row.get(0);
    let extension_version: Option<String> = version_row.get(1);

    let rows = client
        .query(
            "SELECT s.srvname::text,
                    m.definition,
                    m.server_name IS NOT NULL,
                    COALESCE(
                      (SELECT jsonb_object_agg(
                                  option_name,
                                  CASE WHEN option_name IN ('api_key', 'bearer_token', 'headers')
                                       THEN '[configured]'
                                       ELSE option_value END)
                         FROM pg_options_to_table(s.srvoptions)),
                      '{}'::jsonb)
               FROM pg_foreign_server s
               JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw
               LEFT JOIN openapi_fdw_control.sources m ON m.server_name = s.srvname
              WHERE w.fdwname = 'openapi_fdw'
              ORDER BY s.srvname",
            &[],
        )
        .await?;

    let mut sources = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get(0);
        let definition_value: Option<Value> = row.get(1);
        let managed: bool = row.get(2);
        let options: Value = row.get(3);
        let definition = definition_value.and_then(|value| match serde_json::from_value(value) {
            Ok(definition) => Some(definition),
            Err(error) => {
                warn!(server = %name, %error, "stored source definition is unreadable");
                None
            }
        });
        sources.push(SourceState {
            tables: tables_for_server(&client, &name).await?,
            name,
            managed,
            definition,
            options,
        });
    }

    Ok(ControlState {
        extension_version,
        postgres_version,
        sources,
    })
}

pub async fn discover(pool: &Pool, source: &SourceDefinition) -> Result<Discovery> {
    source.validate(true).map_err(|error| anyhow!(error))?;
    let mut preview = source.clone();
    preview.tables.clear();
    let suffix = Uuid::new_v4().simple().to_string();
    let server_name = format!("_openapi_preview_{}", &suffix[..20]);
    let schema_name = format!("_openapi_preview_{}", &suffix[..20]);
    let actual = source_plan(&preview, &server_name, &schema_name, false, false)
        .map_err(|error| anyhow!(error))?;
    let display = source_plan(source, &source.name, &source.schema, false, true)
        .map_err(|error| anyhow!(error))?
        .display();

    let mut client = pool.get().await.context("database pool is unavailable")?;
    let transaction = client.transaction().await?;
    let result = async {
        for statement in actual.statements() {
            transaction.batch_execute(statement).await?;
        }
        let tables = tables_for_server(&transaction, &server_name).await?;
        if tables.is_empty() {
            bail!("the OpenAPI document produced no importable GET/POST tables");
        }
        Ok::<_, anyhow::Error>(tables)
    }
    .await;
    transaction.rollback().await?;
    Ok(Discovery {
        tables: result?,
        sql: display,
    })
}

pub async fn apply_source(
    pool: &Pool,
    source: SourceDefinition,
    replace: bool,
) -> Result<MutationResult> {
    let bundle = Bundle::new(vec![source.clone()]);
    let sql = apply_bundle(pool, &bundle, replace).await?;
    Ok(MutationResult {
        ok: true,
        message: format!(
            "source `{}` is ready in schema `{}`",
            source.name, source.schema
        ),
        sql,
    })
}

pub async fn apply_bundle(pool: &Pool, bundle: &Bundle, replace: bool) -> Result<String> {
    bundle.validate(true).map_err(|error| anyhow!(error))?;
    let mut client = pool.get().await.context("database pool is unavailable")?;
    let transaction = client.transaction().await?;
    let mut displayed = Vec::with_capacity(bundle.sources.len());

    for source in &bundle.sources {
        let existing_wrapper = transaction
            .query_opt(
                "SELECT w.fdwname::text
                   FROM pg_foreign_server s
                   JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw
                  WHERE s.srvname = $1",
                &[&source.name],
            )
            .await?
            .map(|row| row.get::<_, String>(0));
        if let Some(wrapper) = existing_wrapper.as_deref()
            && wrapper != "openapi_fdw"
        {
            bail!(
                "foreign server `{}` belongs to FDW `{wrapper}` and will not be replaced",
                source.name
            );
        }
        let exists = existing_wrapper.is_some();
        if exists && !replace {
            bail!(
                "foreign server `{}` already exists; choose replace explicitly to reconcile it",
                source.name
            );
        }

        let actual = source_plan(
            source,
            &source.name,
            &source.schema,
            exists && replace,
            false,
        )
        .map_err(|error| anyhow!(error))?;
        let redacted = source_plan(
            source,
            &source.name,
            &source.schema,
            exists && replace,
            true,
        )
        .map_err(|error| anyhow!(error))?;
        for statement in actual.statements() {
            transaction.batch_execute(statement).await?;
        }

        let imported: i64 = transaction
            .query_one(
                "SELECT count(*)
                   FROM pg_foreign_table ft
                   JOIN pg_foreign_server s ON s.oid = ft.ftserver
                  WHERE s.srvname = $1",
                &[&source.name],
            )
            .await?
            .get(0);
        if imported == 0 {
            bail!("source `{}` imported no foreign tables", source.name);
        }
        if !source.tables.is_empty() && imported != source.tables.len() as i64 {
            bail!(
                "source `{}` requested {} tables but imported {imported}",
                source.name,
                source.tables.len()
            );
        }

        let safe_definition = serde_json::to_value(source.redacted())?;
        transaction
            .execute(
                "INSERT INTO openapi_fdw_control.sources(server_name, definition)
                 VALUES ($1, $2)
                 ON CONFLICT (server_name) DO UPDATE
                   SET definition = EXCLUDED.definition,
                       updated_at = clock_timestamp()",
                &[&source.name, &safe_definition],
            )
            .await?;
        displayed.push(redacted.display());
    }

    transaction.commit().await?;
    Ok(displayed.join("\n\n-- next source --\n\n"))
}

pub async fn plan_bundle(bundle: &Bundle, replace: bool) -> Result<String> {
    bundle.validate(false).map_err(|error| anyhow!(error))?;
    bundle
        .sources
        .iter()
        .map(|source| {
            source_plan(source, &source.name, &source.schema, replace, true)
                .map(|plan| plan.display())
                .map_err(|error| anyhow!(error))
        })
        .collect::<Result<Vec<_>>>()
        .map(|plans| plans.join("\n\n-- next source --\n\n"))
}

pub async fn export_bundle(pool: &Pool) -> Result<Bundle> {
    let current = state(pool).await?;
    Ok(Bundle::new(
        current
            .sources
            .into_iter()
            .filter_map(|source| source.definition)
            .collect(),
    ))
}

pub async fn delete_source(pool: &Pool, name: &str, confirmation: &str) -> Result<MutationResult> {
    validate_identifier(name, "source name").map_err(|error| anyhow!(error))?;
    if name != confirmation {
        bail!("confirmation must exactly match source name `{name}`");
    }
    let sql = drop_server_sql(name);
    let mut client = pool.get().await.context("database pool is unavailable")?;
    let transaction = client.transaction().await?;
    let existing_wrapper = transaction
        .query_opt(
            "SELECT w.fdwname::text
               FROM pg_foreign_server s
               JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw
              WHERE s.srvname = $1",
            &[&name],
        )
        .await?
        .map(|row| row.get::<_, String>(0));
    let Some(wrapper) = existing_wrapper else {
        bail!("foreign server `{name}` does not exist");
    };
    if wrapper != "openapi_fdw" {
        bail!("foreign server `{name}` belongs to FDW `{wrapper}` and will not be removed");
    }
    transaction.batch_execute(&sql).await?;
    transaction
        .execute(
            "DELETE FROM openapi_fdw_control.sources WHERE server_name = $1",
            &[&name],
        )
        .await?;
    transaction.commit().await?;
    Ok(MutationResult {
        ok: true,
        message: format!(
            "source `{name}` and its foreign tables were removed; schemas were preserved"
        ),
        sql: format!("{sql};"),
    })
}

pub async fn sample_rows(
    pool: &Pool,
    source: &str,
    schema: &str,
    table: &str,
    query: &SampleQuery,
) -> Result<SampleResult> {
    for (value, label) in [
        (source, "source name"),
        (schema, "schema"),
        (table, "table"),
    ] {
        validate_identifier(value, label).map_err(|error| anyhow!(error))?;
    }
    let limit = query.limit.clamp(1, 100);
    let client = pool.get().await.context("database pool is unavailable")?;
    let exists: bool = client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1
                   FROM pg_foreign_table ft
                   JOIN pg_foreign_server s ON s.oid = ft.ftserver
                   JOIN pg_class c ON c.oid = ft.ftrelid
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                   JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw
                  WHERE s.srvname = $1 AND n.nspname = $2 AND c.relname = $3
                    AND w.fdwname = 'openapi_fdw')",
            &[&source, &schema, &table],
        )
        .await?
        .get(0);
    if !exists {
        bail!("the requested OpenAPI foreign table does not exist");
    }

    let filter = match (&query.filter_column, &query.filter_value) {
        (None, None) => None,
        (Some(column), Some(value)) => {
            validate_identifier(column, "filter column").map_err(|error| anyhow!(error))?;
            let is_column: bool = client
                .query_one(
                    "SELECT EXISTS (
                         SELECT 1 FROM information_schema.columns
                          WHERE table_schema = $1 AND table_name = $2 AND column_name = $3)",
                    &[&schema, &table, &column],
                )
                .await?
                .get(0);
            if !is_column {
                bail!("filter column `{column}` does not exist");
            }
            Some((column.as_str(), value.as_str()))
        }
        _ => bail!("filterColumn and filterValue must be provided together"),
    };
    let sql = sample_sql(schema, table, limit, filter).map_err(|error| anyhow!(error))?;
    let wrapped = format!("SELECT to_jsonb(sample_row) FROM ({sql}) AS sample_row");
    let rows = client
        .query(&wrapped, &[])
        .await?
        .into_iter()
        .map(|row| row.get::<_, Value>(0))
        .collect();
    Ok(SampleResult { rows, sql })
}

async fn tables_for_server<C>(client: &C, server: &str) -> Result<Vec<TableState>>
where
    C: GenericClient + Sync,
{
    let table_rows = client
        .query(
            "SELECT n.nspname::text,
                    c.relname::text,
                    COALESCE((SELECT option_value FROM pg_options_to_table(ft.ftoptions)
                               WHERE option_name = 'endpoint'), ''),
                    COALESCE((SELECT option_value FROM pg_options_to_table(ft.ftoptions)
                               WHERE option_name = 'method'), 'GET'),
                    array_remove(ARRAY[
                      CASE WHEN EXISTS (SELECT 1 FROM pg_options_to_table(ft.ftoptions)
                                         WHERE option_name = 'insert_endpoint') THEN 'INSERT' END,
                      CASE WHEN EXISTS (SELECT 1 FROM pg_options_to_table(ft.ftoptions)
                                         WHERE option_name = 'update_endpoint') THEN 'UPDATE' END,
                      CASE WHEN EXISTS (SELECT 1 FROM pg_options_to_table(ft.ftoptions)
                                         WHERE option_name = 'delete_endpoint') THEN 'DELETE' END
                    ]::text[], NULL)
               FROM pg_foreign_table ft
               JOIN pg_foreign_server s ON s.oid = ft.ftserver
               JOIN pg_class c ON c.oid = ft.ftrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE s.srvname = $1
              ORDER BY n.nspname, c.relname",
            &[&server],
        )
        .await?;

    let column_rows = client
        .query(
            "SELECT n.nspname::text,
                    c.relname::text,
                    a.attname::text,
                    format_type(a.atttypid, a.atttypmod),
                    a.attnum
               FROM pg_foreign_table ft
               JOIN pg_foreign_server s ON s.oid = ft.ftserver
               JOIN pg_class c ON c.oid = ft.ftrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace
               JOIN pg_attribute a ON a.attrelid = c.oid
              WHERE s.srvname = $1 AND a.attnum > 0 AND NOT a.attisdropped
              ORDER BY n.nspname, c.relname, a.attnum",
            &[&server],
        )
        .await?;

    let mut columns = BTreeMap::<(String, String), Vec<ColumnState>>::new();
    for row in column_rows {
        columns
            .entry((row.get(0), row.get(1)))
            .or_default()
            .push(ColumnState {
                name: row.get(2),
                data_type: row.get(3),
                ordinal: row.get(4),
            });
    }

    Ok(table_rows
        .into_iter()
        .map(|row| {
            let schema: String = row.get(0);
            let name: String = row.get(1);
            TableState {
                columns: columns
                    .remove(&(schema.clone(), name.clone()))
                    .unwrap_or_default(),
                schema,
                name,
                endpoint: row.get(2),
                method: row.get(3),
                write_operations: row.get(4),
            }
        })
        .collect())
}
