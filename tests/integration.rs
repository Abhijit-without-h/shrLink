use shrlink::compression::ParallelCompressor;
use shrlink::config::Config;
use tempfile::NamedTempFile;
use std::io::Write;

#[tokio::test]
async fn test_end_to_end_compression() {
    let test_data = b"Hello, world! This is a test file for compression.".repeat(1000);
    
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&test_data).unwrap();
    
    let compressor = ParallelCompressor::default();
    let result = compressor.compress_file(temp_file.path()).unwrap();
    
    assert!(!result.chunks.is_empty());
    assert_eq!(result.total_original_size, test_data.len());
    
    let mut reconstructed = Vec::new();
    for chunk in &result.chunks {
        let decompressed = compressor.decompress_chunk(chunk).unwrap();
        reconstructed.extend_from_slice(&decompressed);
    }
    
    assert_eq!(test_data, reconstructed);
}

#[tokio::test]
async fn test_shr_bundle_format() {
    let test_data = b"Bundle test data".repeat(100);
    
    let compressor = ParallelCompressor::default();
    let chunk = compressor.compress_chunk(0, test_data.clone()).unwrap();
    
    let bundle = shrlink::compression::create_shr_bundle(&[chunk]).unwrap();
    let parsed_chunks = shrlink::compression::parse_shr_bundle(&bundle).unwrap();
    
    assert_eq!(parsed_chunks.len(), 1);
    
    let decompressed = compressor.decompress_chunk(&parsed_chunks[0]).unwrap();
    assert_eq!(test_data, decompressed);
}

#[test]
fn test_config_loading() {
    let config = Config::default();
    
    assert_eq!(config.compression.algorithm, "lz4");
    assert_eq!(config.compression.block_size, 4 * 1024 * 1024);
    assert_eq!(config.p2p.timeout_ms, 5000);
    assert_eq!(config.fallback.endpoint, Some("http://localhost:8080".to_string()));
}

#[test]
fn test_parallel_compression_performance() {
    let large_data = vec![0u8; 10 * 1024 * 1024]; // 10 MB
    
    let compressor = ParallelCompressor::default();
    let start = std::time::Instant::now();
    
    let chunk = compressor.compress_chunk(0, large_data.clone()).unwrap();
    let compression_time = start.elapsed();
    
    println!("Compression took: {:?}", compression_time);
    
    let start = std::time::Instant::now();
    let decompressed = compressor.decompress_chunk(&chunk).unwrap();
    let decompression_time = start.elapsed();
    
    println!("Decompression took: {:?}", decompression_time);
    
    assert_eq!(large_data, decompressed);
    assert!(compression_time < std::time::Duration::from_secs(1));
    assert!(decompression_time < std::time::Duration::from_secs(1));
}

#[test]
fn test_url_parsing() {
    use shrlink::p2p::{create_shr_url, parse_shr_url};
    use libp2p::PeerId;
    
    let peer_id = PeerId::random();
    let file_hash = "abc123def456";
    
    let url = create_shr_url(peer_id, file_hash);
    let (parsed_peer_id, parsed_hash) = parse_shr_url(&url).unwrap();
    
    assert_eq!(peer_id, parsed_peer_id);
    assert_eq!(file_hash, parsed_hash);
}

#[test]
fn test_fallback_url_detection() {
    use shrlink::fallback::is_http_url;
    
    assert!(is_http_url("https://example.com/file.shr"));
    assert!(is_http_url("http://localhost:8080/file.shr"));
    assert!(!is_http_url("shr://peer123/hash456"));
    assert!(!is_http_url("file:///local/path"));
}

#[tokio::test]
async fn test_large_file_chunking() {
    let large_data = vec![1u8; 20 * 1024 * 1024]; // 20 MB
    
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&large_data).unwrap();
    
    let compressor = ParallelCompressor::new(4 * 1024 * 1024, 1); // 4 MiB chunks
    let result = compressor.compress_file(temp_file.path()).unwrap();
    
    assert_eq!(result.chunks.len(), 5); // 20 MB / 4 MiB = 5 chunks
    
    let mut reconstructed = Vec::new();
    for chunk in &result.chunks {
        let decompressed = compressor.decompress_chunk(chunk).unwrap();
        reconstructed.extend_from_slice(&decompressed);
    }
    
    assert_eq!(large_data, reconstructed);
}

#[tokio::test]
async fn test_hash_verification() {

    use shrlink::ShrLinkError;
    
    let test_data = b"Test data for hash verification";
    let compressor = ParallelCompressor::default();
    
    let mut chunk = compressor.compress_chunk(0, test_data.to_vec()).unwrap();
    
    // Corrupt the hash
    chunk.hash[0] = chunk.hash[0].wrapping_add(1);
    
    let result = compressor.decompress_chunk(&chunk);
    
    match result {
        Err(ShrLinkError::HashMismatch { .. }) => {
            // Expected error
        }
        _ => panic!("Expected hash mismatch error"),
    }
}
