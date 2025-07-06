use blake3::Hasher;
use lz4_flex::compress_prepend_size;
use rayon::prelude::*;
use std::io::Read;
use std::path::Path;
use std::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt};
use crate::{Result, ShrLinkError};

pub const BLOCK_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
pub const LZ4_ACCELERATION: i32 = 1;

#[derive(Debug, Clone)]
pub struct CompressedChunk {
    pub index: usize,
    pub data: Vec<u8>,
    pub hash: [u8; 32],
    pub original_size: usize,
}

#[derive(Debug)]
pub struct CompressionResult {
    pub chunks: Vec<CompressedChunk>,
    pub total_original_size: usize,
    pub total_compressed_size: usize,
}

pub struct ParallelCompressor {
    block_size: usize,
    acceleration: i32,
    num_workers: usize,
}

impl Default for ParallelCompressor {
    fn default() -> Self {
        Self {
            block_size: BLOCK_SIZE,
            acceleration: LZ4_ACCELERATION,
            num_workers: num_cpus::get(),
        }
    }
}

impl ParallelCompressor {
    pub fn new(block_size: usize, acceleration: i32) -> Self {
        Self {
            block_size,
            acceleration,
            num_workers: num_cpus::get(),
        }
    }

    pub fn with_workers(mut self, num_workers: usize) -> Self {
        self.num_workers = num_workers.max(1);
        self
    }

    pub fn compress_file<P: AsRef<Path>>(&self, path: P) -> Result<CompressionResult> {
        let file = File::open(path)?;
        let file_size = file.metadata()?.len() as usize;
        
        let chunks = self.read_file_chunks(file)?;
        let compressed_chunks = self.compress_chunks_parallel(chunks)?;
        
        let total_compressed_size = compressed_chunks.iter()
            .map(|c| c.data.len())
            .sum();

        Ok(CompressionResult {
            chunks: compressed_chunks,
            total_original_size: file_size,
            total_compressed_size,
        })
    }

    pub async fn compress_async_reader<R: AsyncRead + Unpin>(&self, reader: &mut R) -> Result<CompressionResult> {
        let chunks = self.read_async_chunks(reader).await?;
        let total_original_size = chunks.iter().map(|c| c.len()).sum();
        
        let compressed_chunks = self.compress_chunks_parallel(chunks)?;
        let total_compressed_size = compressed_chunks.iter()
            .map(|c| c.data.len())
            .sum();

        Ok(CompressionResult {
            chunks: compressed_chunks,
            total_original_size,
            total_compressed_size,
        })
    }

    fn read_file_chunks(&self, mut file: File) -> Result<Vec<Vec<u8>>> {
        let mut chunks = Vec::new();
        let mut buffer = vec![0u8; self.block_size];
        
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            
            chunks.push(buffer[..bytes_read].to_vec());
            
            if bytes_read < self.block_size {
                break;
            }
        }
        
        Ok(chunks)
    }

    async fn read_async_chunks<R: AsyncRead + Unpin>(&self, reader: &mut R) -> Result<Vec<Vec<u8>>> {
        let mut chunks = Vec::new();
        let mut buffer = vec![0u8; self.block_size];
        
        loop {
            let bytes_read = reader.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            
            chunks.push(buffer[..bytes_read].to_vec());
            
            if bytes_read < self.block_size {
                break;
            }
        }
        
        Ok(chunks)
    }

    fn compress_chunks_parallel(&self, chunks: Vec<Vec<u8>>) -> Result<Vec<CompressedChunk>> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.num_workers)
            .build()
            .map_err(|e| ShrLinkError::Compression(e.to_string()))?;

        let results: Result<Vec<_>> = pool.install(|| {
            chunks
                .into_par_iter()
                .enumerate()
                .map(|(index, chunk)| self.compress_chunk(index, chunk))
                .collect()
        });

        results
    }

    pub fn compress_chunk(&self, index: usize, chunk: Vec<u8>) -> Result<CompressedChunk> {
        let original_size = chunk.len();
        
        // Hash the original data
        let mut hasher = Hasher::new();
        hasher.update(&chunk);
        let hash = hasher.finalize();

        // Compress with LZ4
        let compressed = compress_prepend_size(&chunk);
        
        Ok(CompressedChunk {
            index,
            data: compressed,
            hash: hash.into(),
            original_size,
        })
    }

    pub fn decompress_chunk(&self, chunk: &CompressedChunk) -> Result<Vec<u8>> {
        use lz4_flex::decompress_size_prepended;
        
        let decompressed = decompress_size_prepended(&chunk.data)
            .map_err(|e| ShrLinkError::Compression(e.to_string()))?;
        
        // Verify hash
        let mut hasher = Hasher::new();
        hasher.update(&decompressed);
        let hash = hasher.finalize();
        
        if hash.as_bytes() != &chunk.hash {
            return Err(ShrLinkError::HashMismatch {
                expected: hex::encode(chunk.hash),
                actual: hex::encode(hash.as_bytes()),
            });
        }
        
        Ok(decompressed)
    }
}

pub fn create_shr_bundle(chunks: &[CompressedChunk]) -> Result<Vec<u8>> {
    let mut bundle = Vec::new();
    
    // Write header: magic + version + chunk count
    bundle.extend_from_slice(b"SHR\x01");
    bundle.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
    
    // Write chunk metadata
    for chunk in chunks {
        bundle.extend_from_slice(&(chunk.index as u32).to_le_bytes());
        bundle.extend_from_slice(&(chunk.original_size as u32).to_le_bytes());
        bundle.extend_from_slice(&(chunk.data.len() as u32).to_le_bytes());
        bundle.extend_from_slice(&chunk.hash);
    }
    
    // Write chunk data
    for chunk in chunks {
        bundle.extend_from_slice(&chunk.data);
    }
    
    Ok(bundle)
}

pub fn parse_shr_bundle(bundle: &[u8]) -> Result<Vec<CompressedChunk>> {
    if bundle.len() < 8 || &bundle[0..4] != b"SHR\x01" {
        return Err(ShrLinkError::InvalidInput("Invalid SHR bundle format".to_string()));
    }
    
    let chunk_count = u32::from_le_bytes([bundle[4], bundle[5], bundle[6], bundle[7]]) as usize;
    let mut chunks = Vec::with_capacity(chunk_count);
    
    let mut offset = 8;
    let metadata_size = chunk_count * (4 + 4 + 4 + 32); // index + original_size + compressed_size + hash
    
    if bundle.len() < offset + metadata_size {
        return Err(ShrLinkError::InvalidInput("Bundle too short for metadata".to_string()));
    }
    
    // Parse metadata
    let mut chunk_infos = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        let index = u32::from_le_bytes([bundle[offset], bundle[offset + 1], bundle[offset + 2], bundle[offset + 3]]) as usize;
        let original_size = u32::from_le_bytes([bundle[offset + 4], bundle[offset + 5], bundle[offset + 6], bundle[offset + 7]]) as usize;
        let compressed_size = u32::from_le_bytes([bundle[offset + 8], bundle[offset + 9], bundle[offset + 10], bundle[offset + 11]]) as usize;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bundle[offset + 12..offset + 44]);
        
        chunk_infos.push((index, original_size, compressed_size, hash));
        offset += 44;
    }
    
    // Parse chunk data
    for (index, original_size, compressed_size, hash) in chunk_infos {
        if offset + compressed_size > bundle.len() {
            return Err(ShrLinkError::InvalidInput("Bundle too short for chunk data".to_string()));
        }
        
        let data = bundle[offset..offset + compressed_size].to_vec();
        
        chunks.push(CompressedChunk {
            index,
            data,
            hash,
            original_size,
        });
        
        offset += compressed_size;
    }
    
    // Sort chunks by index
    chunks.sort_by_key(|c| c.index);
    
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    
    #[test]
    fn test_compression_roundtrip() {
        let compressor = ParallelCompressor::default();
        let test_data = b"Hello, world! This is a test compression string.".repeat(1000);
        
        let chunk = compressor.compress_chunk(0, test_data.clone()).unwrap();
        let decompressed = compressor.decompress_chunk(&chunk).unwrap();
        
        assert_eq!(test_data, decompressed);
    }
    
    #[test]
    fn test_shr_bundle_roundtrip() {
        let compressor = ParallelCompressor::default();
        let test_data = b"Test data for bundle".repeat(100);
        
        let chunk = compressor.compress_chunk(0, test_data.clone()).unwrap();
        let bundle = create_shr_bundle(&[chunk]).unwrap();
        let parsed_chunks = parse_shr_bundle(&bundle).unwrap();
        
        assert_eq!(parsed_chunks.len(), 1);
        let decompressed = compressor.decompress_chunk(&parsed_chunks[0]).unwrap();
        assert_eq!(test_data, decompressed);
    }
    
    #[tokio::test]
    async fn test_parallel_compression() {
        let compressor = ParallelCompressor::default();
        let test_data = b"A".repeat(10 * 1024 * 1024); // 10 MB
        
        let mut cursor = Cursor::new(test_data.clone());
        let result = compressor.compress_async_reader(&mut cursor).await.unwrap();
        
        assert!(result.chunks.len() > 1); // Should be split into multiple chunks
        assert_eq!(result.total_original_size, test_data.len());
        
        // Verify decompression
        let mut decompressed = Vec::new();
        for chunk in &result.chunks {
            let chunk_data = compressor.decompress_chunk(chunk).unwrap();
            decompressed.extend_from_slice(&chunk_data);
        }
        
        assert_eq!(test_data, decompressed);
    }
}
