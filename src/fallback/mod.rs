use std::time::Duration;
use uuid::Uuid;
use reqwest::multipart;
use crate::{Result, ShrLinkError};
use crate::config::FallbackConfig;
use crate::compression::CompressedChunk;

pub struct HttpFallback {
    client: reqwest::Client,
    config: FallbackConfig,
}

impl HttpFallback {
    pub async fn new(config: FallbackConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ShrLinkError::Network(format!("Failed to create HTTP client: {}", e)))?;
        
        Ok(Self { client, config })
    }
    
    pub async fn upload_chunks(&self, chunks: &[CompressedChunk]) -> Result<String> {
        let bundle = crate::compression::create_shr_bundle(chunks)?;
        let filename = format!("{}.shr", Uuid::new_v4());
        
        // Create upload endpoint URL
        let upload_url = if let Some(endpoint) = &self.config.endpoint {
            format!("{}/upload", endpoint)
        } else {
            format!("http://localhost:8080/upload")
        };
        
        // Create multipart form
        let form = multipart::Form::new()
            .part("file", multipart::Part::bytes(bundle)
                .file_name(filename.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| ShrLinkError::Network(format!("Failed to create form part: {}", e)))?);
        
        // Upload file
        let response = self.client
            .post(&upload_url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| ShrLinkError::Network(format!("Failed to upload file: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ShrLinkError::Network(format!("Upload failed with status: {}", response.status())));
        }
        
        // Get the download URL
        let download_url = if let Some(endpoint) = &self.config.endpoint {
            format!("{}/files/{}", endpoint, filename)
        } else {
            format!("http://localhost:8080/files/{}", filename)
        };
        
        tracing::info!("Uploaded {} chunks to HTTP server: {}", chunks.len(), download_url);
        Ok(download_url)
    }
    
    pub async fn download_chunks(&self, url: &str) -> Result<Vec<CompressedChunk>> {
        let response = self.client.get(url).send().await
            .map_err(|e| ShrLinkError::Network(format!("Failed to download from HTTP server: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ShrLinkError::Network(format!("HTTP download failed with status: {}", response.status())));
        }
        
        let bundle = response.bytes().await
            .map_err(|e| ShrLinkError::Network(format!("Failed to read HTTP response: {}", e)))?;
        
        let chunks = crate::compression::parse_shr_bundle(&bundle)?;
        
        tracing::info!("Downloaded {} chunks from HTTP server", chunks.len());
        Ok(chunks)
    }
    
    pub async fn cleanup_old_files(&self) -> Result<usize> {
        // For HTTP fallback, we'll call a cleanup endpoint on the server
        let cleanup_url = if let Some(endpoint) = &self.config.endpoint {
            format!("{}/cleanup", endpoint)
        } else {
            format!("http://localhost:8080/cleanup")
        };
        
        let response = self.client
            .post(&cleanup_url)
            .json(&serde_json::json!({
                "max_age_seconds": self.config.expiry_secs
            }))
            .send()
            .await
            .map_err(|e| ShrLinkError::Network(format!("Failed to call cleanup endpoint: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ShrLinkError::Network(format!("Cleanup failed with status: {}", response.status())));
        }
        
        let result: serde_json::Value = response.json().await
            .map_err(|e| ShrLinkError::Network(format!("Failed to parse cleanup response: {}", e)))?;
        
        let deleted_count = result.get("deleted_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        
        tracing::info!("Cleanup deleted {} files", deleted_count);
        Ok(deleted_count)
    }
    
    pub async fn get_upload_stats(&self) -> Result<FallbackStats> {
        // For HTTP fallback, we'll call a stats endpoint on the server
        let stats_url = if let Some(endpoint) = &self.config.endpoint {
            format!("{}/stats", endpoint)
        } else {
            format!("http://localhost:8080/stats")
        };
        
        let response = self.client
            .get(&stats_url)
            .send()
            .await
            .map_err(|e| ShrLinkError::Network(format!("Failed to call stats endpoint: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ShrLinkError::Network(format!("Stats request failed with status: {}", response.status())));
        }
        
        let result: serde_json::Value = response.json().await
            .map_err(|e| ShrLinkError::Network(format!("Failed to parse stats response: {}", e)))?;
        
        let total_files = result.get("total_files")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        
        let total_bytes = result.get("total_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        
        Ok(FallbackStats {
            total_files,
            total_bytes,
        })
    }
}

#[derive(Debug, Default)]
pub struct FallbackStats {
    pub total_files: usize,
    pub total_bytes: u64,
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub fn extract_filename_from_url(url: &str) -> Option<String> {
    if let Ok(parsed_url) = url::Url::parse(url) {
        let path = parsed_url.path();
        if let Some(filename) = path.split('/').last() {
            if !filename.is_empty() {
                Some(filename.to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_http_url_detection() {
        assert!(is_http_url("https://example.com/file.shr"));
        assert!(is_http_url("http://example.com/file.shr"));
        assert!(!is_http_url("shr://peer123/hash456"));
        assert!(!is_http_url("file:///local/path"));
    }
    
    #[test]
    fn test_filename_extraction() {
        let url = "http://localhost:8080/files/abc123.shr";
        let filename = extract_filename_from_url(url);
        assert_eq!(filename, Some("abc123.shr".to_string()));
    }
    
    #[tokio::test]
    async fn test_fallback_config() {
        let config = FallbackConfig {
            region: "".to_string(), // Not used for HTTP fallback
            bucket: "".to_string(), // Not used for HTTP fallback
            expiry_secs: 3600,
            endpoint: Some("http://localhost:8080".to_string()),
        };
        
        // Test that the config can be used to create a client
        let result = HttpFallback::new(config).await;
        assert!(result.is_ok());
    }
}
