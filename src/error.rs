use pgrx::PgSqlErrorCode;
use pgrx::pg_sys::panic::ErrorReport;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum OpenApiFdwError {
    #[error("invalid OpenAPI FDW configuration: {0}")]
    Configuration(String),

    #[error("OpenAPI FDW request failed: {0}")]
    Http(String),

    #[error("OpenAPI FDW received an invalid response: {0}")]
    Response(String),

    #[error("OpenAPI schema import failed: {0}")]
    Spec(String),

    #[error("cannot convert JSON value for column `{column}` to {target}: {reason}")]
    Conversion {
        column: String,
        target: &'static str,
        reason: String,
    },
}

impl From<OpenApiFdwError> for ErrorReport {
    fn from(value: OpenApiFdwError) -> Self {
        let code = match &value {
            OpenApiFdwError::Configuration(_) => PgSqlErrorCode::ERRCODE_FDW_INVALID_OPTION_NAME,
            OpenApiFdwError::Http(_) => PgSqlErrorCode::ERRCODE_FDW_UNABLE_TO_CREATE_EXECUTION,
            OpenApiFdwError::Response(_) | OpenApiFdwError::Conversion { .. } => {
                PgSqlErrorCode::ERRCODE_FDW_INVALID_DATA_TYPE
            }
            OpenApiFdwError::Spec(_) => PgSqlErrorCode::ERRCODE_FDW_ERROR,
        };

        ErrorReport::new(code, value.to_string(), "")
    }
}

pub(crate) type Result<T> = std::result::Result<T, OpenApiFdwError>;
