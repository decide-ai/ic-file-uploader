//! Parallel chunk uploader for Internet Computer canisters
//!
//! This module provides functionality for uploading multiple chunks in parallel
//! with automatic rate limiting and chunk ID tracking.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tempfile::NamedTempFile;
use std::io::Write;

use crate::{dfx, create_error_string, UploadParams};

/// Configuration for parallel upload operations
#[derive(Debug, Clone)]
pub struct ParallelUploadConfig {
    /// Maximum number of concurrent uploads
    pub max_concurrent: usize,
    /// Target upload rate in MiB per second
    pub target_rate_mibs: f64,
    /// Maximum retry attempts per chunk
    pub max_retries: usize,
    /// Delay between retry attempts in milliseconds
    pub retry_delay_ms: u64,
    /// Progress callback for individual chunks
    pub progress_callback: Option<fn(u32, usize, &str)>,
    /// Rate limiting callback (called with current rate)
    pub rate_callback: Option<fn(f64)>,
}

impl Default for ParallelUploadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,        // Start conservative
            target_rate_mibs: 4.0,    // 4 MiB/s target
            max_retries: 3,
            retry_delay_ms: 1000,
            progress_callback: None,
            rate_callback: None,
        }
    }
}

/// Result of a parallel upload operation
#[derive(Debug)]
pub enum ParallelUploadResult {
    /// All chunks uploaded successfully
    Success,
    /// Some chunks failed after all retries
    PartialFailure {
        /// Successfully uploaded chunk IDs
        successful_chunks: Vec<u32>,
        /// Failed chunk IDs with errors
        failed_chunks: HashMap<u32, String>
    },
    /// Upload was completely unsuccessful
    Failed(String),
}

/// Information about a chunk to be uploaded
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    /// Unique chunk ID (used for ordering/tracking)
    pub chunk_id: u32,
    /// The actual chunk data
    pub data: Vec<u8>,
    /// Size of this chunk in bytes
    pub size: usize,
}

/// Tracks upload progress and rate limiting
#[derive(Debug)]
struct UploadTracker {
    /// Total bytes uploaded so far
    bytes_uploaded: usize,
    /// When the upload session started
    start_time: Instant,
    /// Currently active uploads
    active_uploads: usize,
    /// Completed chunks
    completed_chunks: Vec<u32>,
}

impl UploadTracker {
    fn new() -> Self {
        Self {
            bytes_uploaded: 0,
            start_time: Instant::now(),
            active_uploads: 0,
            completed_chunks: Vec::new(),
        }
    }

    /// Calculate current upload rate in MiB/s
    fn current_rate_mibs(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            (self.bytes_uploaded as f64) / (1024.0 * 1024.0) / elapsed
        } else {
            0.0
        }
    }

    /// Should we start another upload based on rate limiting?
    fn should_start_upload(&self, config: &ParallelUploadConfig) -> bool {
        if self.active_uploads >= config.max_concurrent {
            return false;
        }

        let current_rate = self.current_rate_mibs();
        current_rate < config.target_rate_mibs || self.active_uploads == 0
    }

    /// Calculate delay needed to maintain target rate
    fn calculate_delay(&self, config: &ParallelUploadConfig) -> Duration {
        let current_rate = self.current_rate_mibs();
        if current_rate > config.target_rate_mibs {
            // We're going too fast, delay a bit
            Duration::from_millis(100)
        } else {
            // We can go faster or maintain current pace
            Duration::from_millis(10)
        }
    }
}

/// Converts chunk data with ID to a blob string in Candid format
///
/// # Arguments
///
/// * `chunk_id` - The unique identifier for this chunk
/// * `data` - The chunk data bytes
///
/// # Returns
///
/// A string representation suitable for dfx canister calls
pub fn chunk_with_id_to_candid_args(chunk_id: u32, data: &[u8]) -> String {
    // Use single backslash - the string literal \\ becomes a single \ in the actual string
    let data_blob: String = data.iter().map(|&byte| format!("\\{:02X}", byte)).collect();
    // Format as two separate arguments: nat32 and blob
    format!("({} : nat32, blob \"{}\")", chunk_id, data_blob)
    //create_test_format(chunk_id)
}


/// Test to create exact working format for debugging
pub fn create_test_format(chunk_id: u32) -> String {
    // Create exactly what your test case does for the first few bytes
    match chunk_id {
        0 => "(0, blob \"\01\02\03\04\")".to_string(),
        _ => format!("({}, blob \"\01\02\03\04\"", chunk_id),
    }
}

/// Upload a chunk with retry logic
fn upload_chunk_with_retry(
    params: &UploadParams<'_>,
    chunk: &ChunkInfo,
    config: &ParallelUploadConfig,
    tracker: Arc<Mutex<UploadTracker>>,
) -> Result<(), String> {
    let mut attempts = 0;

    loop {
        attempts += 1;

        let result = upload_chunk_with_id_sync(params, chunk, config);

        match result {
            Ok(()) => {
                // Update tracker
                {
                    let mut tracker = tracker.lock().unwrap();
                    tracker.bytes_uploaded += chunk.size;
                    tracker.completed_chunks.push(chunk.chunk_id);
                    tracker.active_uploads -= 1;
                }
                return Ok(());
            }
            Err(e) => {
                if attempts >= config.max_retries {
                    // Update tracker for failure
                    {
                        let mut tracker = tracker.lock().unwrap();
                        tracker.active_uploads -= 1;
                    }
                    return Err(format!(
                        "Chunk {} failed after {} attempts. Last error: {}",
                        chunk.chunk_id, attempts, e
                    ));
                }

                if let Some(callback) = config.progress_callback {
                    callback(
                        chunk.chunk_id,
                        chunk.size,
                        &format!("⚠ Attempt {}/{} failed, retrying...", attempts, config.max_retries)
                    );
                }

                thread::sleep(Duration::from_millis(config.retry_delay_ms));
            }
        }
    }
}

/// Synchronous version of upload_chunk_with_id with better error handling
fn upload_chunk_with_id_sync(
    params: &UploadParams<'_>,
    chunk: &ChunkInfo,
    config: &ParallelUploadConfig,
) -> Result<(), String> {
    let candid_args = chunk_with_id_to_candid_args(chunk.chunk_id, &chunk.data);

    //println!("Candid Args {}", candid_args);

    // Create temp file with explicit UTF-8 encoding
    let mut temp_file = NamedTempFile::new()
        .map_err(|e| create_error_string(&format!("Failed to create temporary file: {}", e)))?;

    // Write the data and explicitly flush to ensure it's written
    temp_file
        .as_file_mut()
        .write_all(candid_args.as_bytes())
        .map_err(|e| create_error_string(&format!("Failed to write data to temporary file: {}", e)))?;

    // CRITICAL: Flush the file to ensure data is written before dfx reads it
    temp_file
        .as_file_mut()
        .flush()
        .map_err(|e| create_error_string(&format!("Failed to flush temporary file: {}", e)))?;

    let temp_path = temp_file.path().to_str()
        .ok_or_else(|| create_error_string("temp_file path could not be converted to &str"))?;

    let output = dfx(
        "canister",
        "call",
        &vec![
            params.canister_name,
            params.canister_method,
            "--argument-file",
            temp_path,
        ],
        params.network,
    )?;

    if output.status.success() {
        if let Some(callback) = config.progress_callback {
            callback(chunk.chunk_id, chunk.data.len(), "✓ Uploaded");
        }
        Ok(())
    } else {
        let error_message = String::from_utf8_lossy(&output.stderr).to_string();
        Err(create_error_string(&format!("Chunk {} failed: {}", chunk.chunk_id, error_message)))
    }
}

/// Upload multiple chunks in parallel with rate limiting
///
/// # Arguments
///
/// * `params` - Upload parameters including canister info
/// * `chunks` - Vector of chunks to upload with their IDs
/// * `config` - Parallel upload configuration
///
/// # Returns
///
/// A `ParallelUploadResult` indicating the outcome
pub fn upload_chunks_parallel(
    params: &UploadParams<'_>,
    chunks: Vec<ChunkInfo>,
    config: &ParallelUploadConfig,
) -> ParallelUploadResult {
    if chunks.is_empty() {
        return ParallelUploadResult::Failed("No chunks to upload".to_string());
    }

    // STORE THE ORIGINAL TOTAL
    let total_chunks_expected = chunks.len() as u32;

    let tracker = Arc::new(Mutex::new(UploadTracker::new()));
    let mut handles = Vec::new();
    let mut successful_chunks = Vec::new();
    let mut failed_chunks = HashMap::new();

    println!("Starting parallel upload of {} chunks", chunks.len());
    println!("Target rate: {:.1} MiB/s, Max concurrent: {}",
             config.target_rate_mibs, config.max_concurrent);

    let chunks_remaining = Arc::new(Mutex::new(chunks));

    // Main upload loop
    loop {
        // Check if we should start more uploads
        let should_start = {
            let tracker = tracker.lock().unwrap();
            tracker.should_start_upload(config)
        };

        if should_start {
            // Get next chunk
            let next_chunk = {
                let mut chunks_lock = chunks_remaining.lock().unwrap();
                chunks_lock.pop()
            };

            if let Some(chunk) = next_chunk {
                // Start upload in a new thread
                {
                    let mut tracker = tracker.lock().unwrap();
                    tracker.active_uploads += 1;
                }

                // Clone all necessary data for the thread
                let chunk_clone = chunk.clone();
                let config_clone = config.clone();
                let tracker_clone = Arc::clone(&tracker);

                // Create owned copies of the params data for the thread
                let canister_name = params.canister_name.to_string();
                let canister_method = params.canister_method.to_string();
                let name = params.name.to_string();
                let network = params.network.map(|s| s.to_string());

                let handle = thread::spawn(move || {
                    // Reconstruct params inside the thread with owned data
                    let thread_params = UploadParams {
                        name: &name,
                        canister_name: &canister_name,
                        canister_method: &canister_method,
                        network: network.as_deref(),
                    };

                    upload_chunk_with_retry(&thread_params, &chunk_clone, &config_clone, tracker_clone)
                });

                handles.push((chunk.chunk_id, handle));
            }
        }

        // Check for completed uploads
        let mut completed_handles = Vec::new();
        for (i, (chunk_id, handle)) in handles.iter().enumerate() {
            if handle.is_finished() {
                completed_handles.push((i, *chunk_id));
            }
        }

        // Process completed uploads
        for (index, chunk_id) in completed_handles.into_iter().rev() {
            let (_, handle) = handles.remove(index);

            // Always decrement active_uploads when a thread completes
            {
                let mut tracker = tracker.lock().unwrap();
                tracker.active_uploads -= 1;
            }

            match handle.join() {
                Ok(Ok(())) => {
                    successful_chunks.push(chunk_id);
                }
                Ok(Err(e)) => {
                    failed_chunks.insert(chunk_id, e);
                }
                Err(_) => {
                    failed_chunks.insert(chunk_id, "Thread panic".to_string());
                }
            }
        }

        // Rate limiting delay
        let delay = {
            let tracker = tracker.lock().unwrap();
            if let Some(rate_callback) = config.rate_callback {
                rate_callback(tracker.current_rate_mibs());
            }
            tracker.calculate_delay(config)
        };

        thread::sleep(delay);

        // Check if we're done
        let (chunks_empty, no_active) = {
            let chunks_lock = chunks_remaining.lock().unwrap();
            let tracker_lock = tracker.lock().unwrap();
            (chunks_lock.is_empty(), tracker_lock.active_uploads == 0)
        };

        // SIMPLE COMPLETION CHECK: All chunks are accounted for (success + failure)
        let total_completed = successful_chunks.len() + failed_chunks.len();
        if total_completed >= total_chunks_expected as usize {
            break;
        }
        
        if chunks_empty && no_active && handles.is_empty() {
            break;
        }
    }

    // Final rate report
    {
        let tracker = tracker.lock().unwrap();
        let final_rate = tracker.current_rate_mibs();
        let total_mb = tracker.bytes_uploaded as f64 / (1024.0 * 1024.0);
        println!("Upload completed. Final rate: {:.2} MiB/s, Total: {:.2} MiB",
                 final_rate, total_mb);
    }

    // Check completion and force exit before returning results
    if failed_chunks.is_empty() {
        // All chunks succeeded, exit cleanly
        println!("✅ All {} chunks uploaded successfully!", successful_chunks.len());
        std::process::exit(0);
    } else if successful_chunks.is_empty() {
        println!("❌ All chunks failed");
        std::process::exit(1);
    } else {
        println!("❌ Upload completed with {} successes and {} failures",
                 successful_chunks.len(), failed_chunks.len());
        std::process::exit(1);
    }
}

/// Convert regular chunks to ChunkInfo with sequential IDs
///
/// # Arguments
///
/// * `chunks` - Vector of raw chunk data
/// * `start_id` - Starting chunk ID (for resume scenarios)
///
/// # Returns
///
/// Vector of ChunkInfo with assigned IDs
pub fn chunks_to_chunk_info(chunks: &[Vec<u8>]) -> Vec<ChunkInfo> {
    chunks
        .iter()
        .enumerate()
        .map(|(i, data)| ChunkInfo {
            chunk_id: i as u32,
            data: data.clone(),
            size: data.len(),
        })
        .collect()
}




#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candid_args_format() {
        let test_data = vec![0x00, 0x01, 0x02];
        let result = chunk_with_id_to_candid_args(0, &test_data);

        // Should produce: (0 : nat32, blob "\00\01\02")
        let expected = r#"(0 : nat32, blob "\00\01\02")"#;
        assert_eq!(result, expected);

        println!("Generated: {}", result);
        println!("Expected:  {}", expected);
    }

    #[test]
    fn test_single_byte() {
        let test_data = vec![0xFF];
        let result = chunk_with_id_to_candid_args(5, &test_data);
        let expected = r#"(5 : nat32, blob "\FF")"#;
        assert_eq!(result, expected);
    }

    #[test]
    fn test_chunk_info_sequential_ids() {
        let chunks = vec![
            vec![1, 2, 3],
            vec![4, 5, 6],
            vec![7, 8, 9],
            vec![10, 11, 12],
        ];

        let chunk_infos = chunks_to_chunk_info(&chunks);

        // Verify IDs are sequential starting from 0
        assert_eq!(chunk_infos.len(), 4);
        assert_eq!(chunk_infos[0].chunk_id, 0);
        assert_eq!(chunk_infos[1].chunk_id, 1);
        assert_eq!(chunk_infos[2].chunk_id, 2);
        assert_eq!(chunk_infos[3].chunk_id, 3);

        // Verify data is preserved
        assert_eq!(chunk_infos[0].data, vec![1, 2, 3]);
        assert_eq!(chunk_infos[3].data, vec![10, 11, 12]);
    }

    #[test]
    fn test_resume_logic_skips_correct_chunks() {
        let chunks = vec![
            vec![1, 2, 3],    // chunk_id: 0
            vec![4, 5, 6],    // chunk_id: 1
            vec![7, 8, 9],    // chunk_id: 2
            vec![10, 11, 12], // chunk_id: 3
            vec![13, 14, 15], // chunk_id: 4
        ];

        let chunk_infos = chunks_to_chunk_info(&chunks);

        // Simulate resuming from chunk offset 2 (should start from chunk_id 2)
        let chunk_offset = 2;
        let chunks_to_upload: Vec<_> = chunk_infos
            .into_iter()
            .skip(chunk_offset)
            .collect();

        // Should have 3 chunks remaining (IDs 2, 3, 4)
        assert_eq!(chunks_to_upload.len(), 3);
        assert_eq!(chunks_to_upload[0].chunk_id, 2);
        assert_eq!(chunks_to_upload[1].chunk_id, 3);
        assert_eq!(chunks_to_upload[2].chunk_id, 4);

        // Verify the data matches
        assert_eq!(chunks_to_upload[0].data, vec![7, 8, 9]);
        assert_eq!(chunks_to_upload[2].data, vec![13, 14, 15]);
    }

    #[test]
    fn test_retry_chunks_filter() {
        let chunks = vec![
            vec![1, 2],    // chunk_id: 0
            vec![3, 4],    // chunk_id: 1
            vec![5, 6],    // chunk_id: 2
            vec![7, 8],    // chunk_id: 3
            vec![9, 10],   // chunk_id: 4
        ];

        let chunk_infos = chunks_to_chunk_info(&chunks);

        // Simulate retrying specific failed chunks: 1, 3
        let retry_ids = vec![1u32, 3u32];
        let chunks_to_upload: Vec<_> = chunk_infos
            .into_iter()
            .filter(|chunk| retry_ids.contains(&chunk.chunk_id))
            .collect();

        // Should have exactly 2 chunks
        assert_eq!(chunks_to_upload.len(), 2);
        assert_eq!(chunks_to_upload[0].chunk_id, 1);
        assert_eq!(chunks_to_upload[1].chunk_id, 3);

        // Verify the data matches
        assert_eq!(chunks_to_upload[0].data, vec![3, 4]);
        assert_eq!(chunks_to_upload[1].data, vec![7, 8]);
    }

    #[test]
    fn test_no_double_offset_bug() {
        // This test specifically verifies the bug is fixed
        let chunks = vec![
            vec![0],  // chunk_id: 0
            vec![1],  // chunk_id: 1
            vec![2],  // chunk_id: 2
            vec![3],  // chunk_id: 3
            vec![4],  // chunk_id: 4
        ];

        let chunk_offset = 2;
        let chunk_infos = chunks_to_chunk_info(&chunks); // Start IDs from 0

        // Apply resume logic (skip first chunk_offset chunks)
        let chunks_to_upload: Vec<_> = chunk_infos
            .into_iter()
            .skip(chunk_offset)
            .collect();

        // Should start from chunk_id 2 (not 4 like the bug would cause)
        assert_eq!(chunks_to_upload[0].chunk_id, 2);
        assert_eq!(chunks_to_upload[0].data, vec![2]);

        // Should have 3 chunks total (IDs 2, 3, 4)
        assert_eq!(chunks_to_upload.len(), 3);
        assert_eq!(chunks_to_upload[2].chunk_id, 4);
    }

    #[test]
    fn test_edge_case_resume_from_last_chunk() {
        let chunks = vec![vec![1], vec![2], vec![3]];
        let chunk_infos = chunks_to_chunk_info(&chunks);

        // Resume from the last chunk
        let chunks_to_upload: Vec<_> = chunk_infos
            .into_iter()
            .skip(2)
            .collect();

        assert_eq!(chunks_to_upload.len(), 1);
        assert_eq!(chunks_to_upload[0].chunk_id, 2);
        assert_eq!(chunks_to_upload[0].data, vec![3]);
    }

    #[test]
    fn test_edge_case_resume_beyond_chunks() {
        let chunks = vec![vec![1], vec![2]];
        let chunk_infos = chunks_to_chunk_info(&chunks);

        // Try to resume beyond available chunks
        let chunks_to_upload: Vec<_> = chunk_infos
            .into_iter()
            .skip(5)  // Skip more than available
            .collect();

        // Should result in empty vector
        assert_eq!(chunks_to_upload.len(), 0);
    }
}