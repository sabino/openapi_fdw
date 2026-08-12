use pgrx::prelude::*;
use std::sync::OnceLock;

mod error;
mod fdw;
mod http;
mod options;
mod response;
mod spec;

pg_module_magic!();

// reqwest's rustls backend needs one process-wide crypto provider. PostgreSQL
// loads this library once in each backend, so OnceLock is sufficient here.
static RUSTLS_PROVIDER: OnceLock<()> = OnceLock::new();

pub(crate) fn initialize_tls() {
    RUSTLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

extension_sql_file!("../sql/finalize.sql", finalize);
