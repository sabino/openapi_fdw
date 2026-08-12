use anyhow::{Context, Result, bail};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct Settings {
    pub listen: SocketAddr,
    pub database_url: String,
    pub admin_token: String,
    pub cookie_secure: bool,
    pub pool_max_size: usize,
}

impl Settings {
    pub fn from_environment() -> Result<Self> {
        let listen = std::env::var("OPENAPI_FDW_LISTEN")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .context("OPENAPI_FDW_LISTEN must be an IP:port socket address")?;
        let database_url = required("DATABASE_URL")?;
        let admin_token = required("OPENAPI_FDW_ADMIN_TOKEN")?;
        if admin_token.len() < 16 {
            bail!("OPENAPI_FDW_ADMIN_TOKEN must contain at least 16 characters");
        }
        let cookie_secure = optional_bool("OPENAPI_FDW_COOKIE_SECURE", true)?;
        let pool_max_size = std::env::var("OPENAPI_FDW_POOL_SIZE")
            .unwrap_or_else(|_| "8".to_string())
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=64).contains(value))
            .context("OPENAPI_FDW_POOL_SIZE must be between 1 and 64")?;
        Ok(Self {
            listen,
            database_url,
            admin_token,
            cookie_secure,
            pool_max_size,
        })
    }
}

fn required(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is required"))
}

fn optional_bool(name: &str, default: bool) -> Result<bool> {
    match std::env::var(name).ok().as_deref() {
        None => Ok(default),
        Some("true" | "1" | "yes" | "on") => Ok(true),
        Some("false" | "0" | "no" | "off") => Ok(false),
        Some(_) => bail!("{name} must be true or false"),
    }
}
