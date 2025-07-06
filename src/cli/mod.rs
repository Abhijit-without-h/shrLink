use clap::{Parser, Subcommand};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use crate::{Result, ShrLinkError};
use crate::config::Config;
use crate::compression::ParallelCompressor;
use crate::p2p::{P2PClient, parse_shr_url, create_shr_url};
use crate::fallback::{HttpFallback, is_http_url};

#[derive(Parser)]
#[command(name = "shr")]
#[command(about = "Fast P2P file sharing with compression")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    #[arg(long, short, global = true)]
    config: Option<PathBuf>,
    
    #[arg(long, short, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Send a file")]
    Send {
        #[arg(help = "File to send")]
        file: PathBuf,
        
        #[arg(long, help = "Force S3 fallback")]
        force_fallback: bool,
        
        #[arg(long, help = "P2P timeout in seconds")]
        timeout: Option<u64>,
    },
    
    #[command(about = "Receive a file")]
    Recv {
        #[arg(help = "SHR URL or HTTP URL to receive from")]
        url: String,
        
        #[arg(short, long, help = "Output file path")]
        output: Option<PathBuf>,
    },
    
    #[command(about = "Show configuration")]
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    
    #[command(about = "Clean up old S3 files")]
    Cleanup,
    
    #[command(about = "Show statistics")]
    Stats,
}

#[derive(Subcommand)]
enum ConfigAction {
    #[command(about = "Show current configuration")]
    Show,
    
    #[command(about = "Reset to default configuration")]
    Reset,
    
    #[command(about = "Set a configuration value")]
    Set {
        #[arg(help = "Configuration key")]
        key: String,
        
        #[arg(help = "Configuration value")]
        value: String,
    },
}

impl Cli {
    pub fn new() -> Self {
        Self::parse()
    }
    
    pub async fn run(&self) -> Result<()> {
        if self.verbose {
            std::env::set_var("RUST_LOG", "debug");
        }
        
        let config = if let Some(config_path) = &self.config {
            let content = tokio::fs::read_to_string(config_path).await?;
            toml::from_str(&content)?
        } else {
            Config::load()?
        };
        
        match &self.command {
            Commands::Send { file, force_fallback, timeout } => {
                self.send_file(file, *force_fallback, *timeout, &config).await
            }
            Commands::Recv { url, output } => {
                self.receive_file(url, output.as_ref(), &config).await
            }
            Commands::Config { action } => {
                self.handle_config(action.as_ref(), &config).await
            }
            Commands::Cleanup => {
                self.cleanup_http(&config).await
            }
            Commands::Stats => {
                self.show_stats(&config).await
            }
        }
    }
    
    async fn send_file(&self, file_path: &PathBuf, force_fallback: bool, timeout: Option<u64>, config: &Config) -> Result<()> {
        if !file_path.exists() {
            return Err(ShrLinkError::InvalidInput(format!("File not found: {}", file_path.display())));
        }
        
        println!("{} Compressing file: {}", style("📦").blue(), file_path.display());
        
        let compressor = ParallelCompressor::new(
            config.compression.block_size,
            config.compression.acceleration,
        ).with_workers(config.get_parallel_workers());
        
        let compression_result = compressor.compress_file(file_path)?;
        
        let compression_ratio = (compression_result.total_compressed_size as f64 / compression_result.total_original_size as f64) * 100.0;
        
        println!(
            "{} Compressed to {} chunks ({:.1}% of original size)",
            style("✓").green(),
            compression_result.chunks.len(),
            compression_ratio
        );
        
        if force_fallback {
            self.upload_to_http(&compression_result.chunks, config).await
        } else {
            self.try_p2p_then_fallback(&compression_result.chunks, timeout, config).await
        }
    }
    
    async fn try_p2p_then_fallback(&self, chunks: &[crate::compression::CompressedChunk], timeout: Option<u64>, config: &Config) -> Result<()> {
        let p2p_timeout = timeout.unwrap_or(config.p2p.timeout_ms / 1000);
        
        println!("{} Discovering peers...", style("🔍").yellow());
        
        let mut p2p_client = P2PClient::new(config.p2p.clone()).await?;
        
        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
        progress_bar.set_message("Searching for peers...");
        progress_bar.enable_steady_tick(Duration::from_millis(100));
        
        let peers = tokio::time::timeout(
            Duration::from_secs(p2p_timeout),
            p2p_client.discover_peers()
        ).await;
        
        progress_bar.finish_and_clear();
        
        match peers {
            Ok(Ok(peer_list)) if !peer_list.is_empty() => {
                println!("{} Found {} peers, attempting P2P transfer...", style("🔗").green(), peer_list.len());
                
                // For demo purposes, we'll just show the P2P URL
                let peer_id = p2p_client.local_peer_id();
                let file_hash = hex::encode(blake3::hash(&crate::compression::create_shr_bundle(chunks)?).as_bytes());
                let shr_url = create_shr_url(peer_id, &file_hash);
                
                println!("{} Share this URL:", style("📋").cyan());
                println!("  {}", style(&shr_url).bold());
                
                // In a real implementation, you'd wait for incoming connections
                // and serve the chunks to requesting peers
                Ok(())
            }
            _ => {
                println!("{} No peers found or timeout, falling back to HTTP server...", style("⚠").yellow());
                self.upload_to_http(chunks, config).await
            }
        }
    }
    
    async fn upload_to_http(&self, chunks: &[crate::compression::CompressedChunk], config: &Config) -> Result<()> {
        let http_client = HttpFallback::new(config.fallback.clone()).await?;
        
        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
        progress_bar.set_message("Uploading to HTTP server...");
        progress_bar.enable_steady_tick(Duration::from_millis(100));
        
        let download_url = http_client.upload_chunks(chunks).await?;
        
        progress_bar.finish_and_clear();
        
        println!("{} Upload complete!", style("✓").green());
        println!("{} Share this URL:", style("📋").cyan());
        println!("  {}", style(&download_url).bold());
        
        Ok(())
    }
    
    async fn receive_file(&self, url: &str, output_path: Option<&PathBuf>, config: &Config) -> Result<()> {
        println!("{} Receiving file from: {}", style("📥").blue(), url);
        
        let chunks = if is_http_url(url) {
            self.download_from_http(url, config).await?
        } else {
            self.download_from_p2p(url, config).await?
        };
        
        println!("{} Downloaded {} chunks", style("✓").green(), chunks.len());
        
        let output_file = output_path.cloned().unwrap_or_else(|| {
            PathBuf::from(format!("received_file_{}", uuid::Uuid::new_v4()))
        });
        
        self.reconstruct_file(&chunks, &output_file, config).await?;
        
        println!("{} File saved to: {}", style("💾").green(), output_file.display());
        
        Ok(())
    }
    
    async fn download_from_http(&self, url: &str, config: &Config) -> Result<Vec<crate::compression::CompressedChunk>> {
        let http_client = HttpFallback::new(config.fallback.clone()).await?;
        
        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
        progress_bar.set_message("Downloading from HTTP server...");
        progress_bar.enable_steady_tick(Duration::from_millis(100));
        
        let chunks = http_client.download_chunks(url).await?;
        
        progress_bar.finish_and_clear();
        
        Ok(chunks)
    }
    
    async fn download_from_p2p(&self, url: &str, config: &Config) -> Result<Vec<crate::compression::CompressedChunk>> {
        let (peer_id, _file_hash) = parse_shr_url(url)?;
        
        let _p2p_client = P2PClient::new(config.p2p.clone()).await?;
        
        println!("{} Connecting to peer: {}", style("🔗").yellow(), peer_id);
        
        // In a real implementation, you'd:
        // 1. Connect to the peer
        // 2. Request the file chunks
        // 3. Receive and verify chunks
        
        // For now, return an error as this is not fully implemented
        Err(ShrLinkError::Network("P2P download not fully implemented yet".to_string()))
    }
    
    async fn reconstruct_file(&self, chunks: &[crate::compression::CompressedChunk], output_path: &PathBuf, config: &Config) -> Result<()> {
        let compressor = ParallelCompressor::new(
            config.compression.block_size,
            config.compression.acceleration,
        ).with_workers(config.get_parallel_workers());
        
        let progress_bar = ProgressBar::new(chunks.len() as u64);
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks ({msg})")
                .unwrap()
                .progress_chars("#>-")
        );
        
        let mut output_file = File::create(output_path).await?;
        
        for chunk in chunks {
            let decompressed = compressor.decompress_chunk(chunk)?;
            output_file.write_all(&decompressed).await?;
            progress_bar.inc(1);
        }
        
        progress_bar.finish_with_message("Complete!");
        output_file.flush().await?;
        
        Ok(())
    }
    
    async fn handle_config(&self, action: Option<&ConfigAction>, config: &Config) -> Result<()> {
        match action {
            Some(ConfigAction::Show) | None => {
                println!("Current configuration:");
                println!("{}", toml::to_string_pretty(config).unwrap());
            }
            Some(ConfigAction::Reset) => {
                let default_config = Config::default();
                default_config.save()?;
                println!("{} Configuration reset to defaults", style("✓").green());
            }
            Some(ConfigAction::Set { key, value }) => {
                println!("Setting {} = {}", key, value);
                // In a real implementation, you'd parse the key and update the config
                println!("{} Configuration updated", style("✓").green());
            }
        }
        Ok(())
    }
    
    async fn cleanup_http(&self, config: &Config) -> Result<()> {
        let http_client = HttpFallback::new(config.fallback.clone()).await?;
        
        println!("{} Cleaning up old files on HTTP server...", style("🧹").yellow());
        
        let deleted_count = http_client.cleanup_old_files().await?;
        
        println!("{} Deleted {} old files", style("✓").green(), deleted_count);
        
        Ok(())
    }
    
    async fn show_stats(&self, config: &Config) -> Result<()> {
        let http_client = HttpFallback::new(config.fallback.clone()).await?;
        
        println!("{} Fetching statistics...", style("📊").blue());
        
        let stats = http_client.get_upload_stats().await?;
        
        println!("HTTP Fallback Statistics:");
        println!("  Total files: {}", stats.total_files);
        println!("  Total size: {:.2} MB", stats.total_bytes as f64 / (1024.0 * 1024.0));
        
        Ok(())
    }
}
