use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use crate::{Result, ShrLinkError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub p2p: P2PConfig,
    pub compression: CompressionConfig,
    pub fallback: FallbackConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PConfig {
    pub bootstrap: Vec<String>,
    pub timeout_ms: u64,
    pub port: Option<u16>,
    pub enable_mdns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    pub algorithm: String,
    pub block_size: usize,
    pub acceleration: i32,
    pub parallel_workers: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    pub region: String,
    pub bucket: String,
    pub expiry_secs: u64,
    pub endpoint: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            p2p: P2PConfig {
                bootstrap: vec![
                    "/dns4/bootstrap.libp2p.io/tcp/443/quic-v1".to_string(),
                    "/dns4/bootstrap.libp2p.io/tcp/443/quic-v1/p2p/12D3KooWGCYDpyGwFvjNbFWQXCCK9G4RZekkKfXXc2QnP8HWqDek".to_string(),
                ],
                timeout_ms: 5000,
                port: None,
                enable_mdns: true,
            },
            compression: CompressionConfig {
                algorithm: "lz4".to_string(),
                block_size: 4 * 1024 * 1024, // 4 MiB
                acceleration: 1,
                parallel_workers: None,
            },
            fallback: FallbackConfig {
                region: "".to_string(), // Not used for HTTP fallback
                bucket: "".to_string(), // Not used for HTTP fallback
                expiry_secs: 86400, // 24 hours
                endpoint: Some("http://localhost:8080".to_string()),
            },
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();
        
        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            let default_config = Config::default();
            default_config.save()?;
            Ok(default_config)
        }
    }
    
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path();
        
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let content = toml::to_string_pretty(self)
            .map_err(|e| ShrLinkError::Other(e.into()))?;
        
        fs::write(&config_path, content)?;
        Ok(())
    }
    
    fn config_path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("shrlink");
        path.push("config.toml");
        path
    }
    
    pub fn get_parallel_workers(&self) -> usize {
        self.compression.parallel_workers.unwrap_or_else(num_cpus::get)
    }
}

// Add dirs dependency to Cargo.toml
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.compression.algorithm, "lz4");
        assert_eq!(config.compression.block_size, 4 * 1024 * 1024);
        assert_eq!(config.p2p.timeout_ms, 5000);
    }
    
    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        
        assert_eq!(config.compression.algorithm, deserialized.compression.algorithm);
        assert_eq!(config.p2p.timeout_ms, deserialized.p2p.timeout_ms);
    }
}
