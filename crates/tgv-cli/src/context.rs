//! Shared CLI inputs. Configuration and the store reader open only when a command needs them.
use crate::style::Terminal;
use anyhow::Result;
use std::path::PathBuf;
use tgv_daemon::config::Config;
use tgv_harness::host::HostEnv;
use tgv_store::Reader;

pub struct Context {
    pub config_path: Option<PathBuf>,
    pub data_dir_override: Option<PathBuf>,
    /// The process environment, captured once in `main`.
    pub env: HostEnv,
    /// Whether human-readable output on each stream is colored.
    pub terminal: Terminal,
}

impl Context {
    pub fn new(
        config_path: Option<PathBuf>,
        data_dir_override: Option<PathBuf>,
        env: HostEnv,
        terminal: Terminal,
    ) -> Self {
        Self {
            config_path,
            data_dir_override,
            env,
            terminal,
        }
    }
    pub fn config(&self) -> Result<Config> {
        Ok(Config::load(self.config_path.as_deref())?)
    }
    pub fn data_dir(&self, config: &Config) -> Result<PathBuf> {
        Ok(config.resolve_data_dir(self.data_dir_override.clone(), &self.env)?)
    }
    /// A read-only reader. It works beside a running daemon and never migrates.
    pub async fn reader(&self) -> Result<Reader> {
        let path = self.data_dir(&self.config()?)?;
        Ok(Reader::open(&path, 1).await?)
    }
}
