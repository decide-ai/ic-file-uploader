//! This crate provides functionality for uploading files to Internet Computer canisters.
//!
//! It includes utilities for splitting files into chunks, converting data to blob strings,
//! and interfacing with the `dfx` command-line tool to upload data to canisters.
#![warn(missing_docs)]

pub mod parallel;

use std::process::Command;
use std::io::Write;
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;

/// The maximum size of the HTTP payload for canister updates, set to 2 MiB.
pub const MAX_CANISTER_HTTP_PAYLOAD_SIZE: usize = 2 * 1000 * 1000; // 2 MiB

/// Configuration for upload operations with retry and resume capabilities.
#[derive(Debug, Clone)]
pub struct UploadConfig {
    /// Maximum number of retry attempts per chunk
    pub max_retries: usize,
    /// Delay between retry attempts in milliseconds
    pub retry_delay_ms: u64,
    /// Whether to enable auto-resume functionality
    pub auto_resume: bool,
    /// Optional callback for progress reporting
    pub progress_callback: Option<fn(usize, usize, &str)>,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay_ms: 1000,
            auto_resume: false,
            progress_callback: None,
        }
    }
}

impl UploadConfig {
    /// Creates a new UploadConfig with auto-resume enabled
    pub fn with_auto_resume() -> Self {
        Self {
            auto_resume: true,
            ..Default::default()
        }
    }

    /// Sets the maximum number of retry attempts
    pub fn with_max_retries(mut self, retries: usize) -> Self {
        self.max_retries = retries;
        self
    }

    /// Sets the retry delay in milliseconds
    pub fn with_retry_delay(mut self, delay_ms: u64) -> Self {
        self.retry_delay_ms = delay_ms;
        self
    }

    /// Sets a progress callback function
    pub fn with_progress_callback(mut self, callback: fn(usize, usize, &str)) -> Self {
        self.progress_callback = Some(callback);
        self
    }
}

/// Result of a chunk upload operation
#[derive(Debug)]
pub enum ChunkUploadResult {
    /// Chunk uploaded successfully
    Success,
    /// Chunk failed after all retry attempts
    Failed(String),
    /// Upload was interrupted and can be resumed
    Interrupted {
        /// The index of the chunk that failed (0-based)
        failed_at_chunk: usize,
        /// The error message describing the failure
        error: String
    },
}

/// Parameters for uploading file chunks to a canister
#[derive(Debug, Clone)]
pub struct UploadParams<'a> {
    /// Name for display purposes
    pub name: &'a str,
    /// Target canister name
    pub canister_name: &'a str,
    /// Canister method to call
    pub canister_method: &'a str,
    /// Optional network specification
    pub network: Option<&'a str>,
}



/// Splits the data into chunks.
///
/// # Arguments
///
/// * `data` - A vector of bytes representing the data to be split.
/// * `chunk_size` - The size of each chunk.
/// * `start_ind` - The starting index for chunking.
///
/// # Returns
///
/// A vector of byte vectors, each representing a chunk of the original data.
pub fn split_into_chunks(data: Vec<u8>, chunk_size: usize, start_ind: usize) -> Vec<Vec<u8>> {
    (start_ind..data.len())
        .step_by(chunk_size)
        .map(|start| {
            let end = usize::min(start + chunk_size, data.len());
            data[start..end].to_vec()
        })
        .collect()
}

/// Converts a vector of bytes to a blob string.
///
/// # Arguments
///
/// * `data` - A slice of bytes to be converted.
///
/// # Returns
///
/// A string representation of the blob data.
pub fn vec_u8_to_blob_string(data: &[u8]) -> String {
    let blob_content: String = data.iter().map(|&byte| format!("\\{:02X}", byte)).collect();
    format!("(blob \"{}\")", blob_content)
}

/// Uploads a chunk of data to the specified canister method.
///
/// # Arguments
///
/// * `name` - The name of the chunk being uploaded.
/// * `canister_name` - The name of the canister.
/// * `bytecode_chunk` - A reference to the vector of bytes representing the chunk.
/// * `canister_method_name` - The name of the canister method to call.
/// * `chunk_number` - The number of the current chunk.
/// * `chunk_total` - The total number of chunks.
/// * `network` - An optional network type.
///
/// # Returns
///
/// A `Result` indicating success (`Ok(())`) or an error message (`Err(String)`).
pub fn upload_chunk(name: &str,
    canister_name: &str,
    bytecode_chunk: &[u8],
    canister_method_name: &str,
    chunk_number: usize,
    chunk_total: usize,
    network: Option<&str>) -> Result<(), String> {

    let blob_string = vec_u8_to_blob_string(bytecode_chunk);

    let mut temp_file = NamedTempFile::new()
        .map_err(|_| create_error_string("Failed to create temporary file"))?;

    temp_file
        .as_file_mut()
        .write_all(blob_string.as_bytes())
        .map_err(|_| create_error_string("Failed to write data to temporary file"))?;

    let output = dfx(
        "canister",
        "call",
        &vec![
            canister_name,
            canister_method_name,
            "--argument-file",
            temp_file.path().to_str().ok_or(create_error_string(
                "temp_file path could not be converted to &str",
            ))?,
        ],
        network, // Pass the optional network argument
    )?;

    // 0-indexing to 1-indexing
    let chunk_number_display = chunk_number + 1;

    if output.status.success() {
        println!("Uploading {name} chunk {chunk_number_display}/{chunk_total}");
    } else {
        let error_message = String::from_utf8_lossy(&output.stderr).to_string();
        eprintln!("Failed to upload chunk {chunk_number_display}: {error_message}");
        return Err(create_error_string(&format!("Chunk {chunk_number_display} failed: {error_message}")));
    }

    Ok(())
}

/// Uploads a single chunk with retry logic based on the provided configuration.
///
/// # Arguments
///
/// * `params` - Upload parameters including canister info
/// * `chunk` - The chunk data to upload
/// * `chunk_index` - The index of the current chunk (0-based)
/// * `total_chunks` - The total number of chunks
/// * `config` - Upload configuration with retry settings
///
/// # Returns
///
/// A `Result` indicating success or failure after all attempts
pub fn upload_chunk_with_config(
    params: &UploadParams,
    chunk: &[u8],
    chunk_index: usize,
    total_chunks: usize,
    config: &UploadConfig,
) -> Result<(), String> {
    let mut attempts = 0;
    let max_attempts = if config.auto_resume { config.max_retries } else { 1 };

    loop {
        attempts += 1;

        match upload_chunk(
            params.name,
            params.canister_name,
            chunk,
            params.canister_method,
            chunk_index,
            total_chunks,
            params.network,
        ) {
            Ok(()) => {
                if let Some(callback) = config.progress_callback {
                    let status = if attempts > 1 {
                        format!("✓ Uploaded after {} attempts", attempts)
                    } else {
                        "✓ Uploaded".to_string()
                    };
                    callback(chunk_index + 1, total_chunks, &status);
                }
                return Ok(());
            }
            Err(e) => {
                if attempts >= max_attempts {
                    return Err(format!(
                        "Failed to upload chunk {}/{} after {} attempts. Last error: {}",
                        chunk_index + 1, total_chunks, attempts, e
                    ));
                }

                if let Some(callback) = config.progress_callback {
                    callback(
                        chunk_index + 1,
                        total_chunks,
                        &format!("⚠ Attempt {}/{} failed, retrying...", attempts, max_attempts)
                    );
                }

                thread::sleep(Duration::from_millis(config.retry_delay_ms));
            }
        }
    }
}

/// Uploads multiple chunks with comprehensive error handling and resume capability.
///
/// This is the main high-level function that handles the entire upload process
/// with built-in retry logic and resume functionality.
///
/// # Arguments
///
/// * `params` - Upload parameters including canister info
/// * `chunks` - Vector of chunks to upload
/// * `start_from_chunk` - Chunk index to start from (for resume functionality)
/// * `config` - Upload configuration
///
/// # Returns
///
/// A `ChunkUploadResult` indicating the outcome of the upload operation
pub fn upload_chunks_with_resume(
    params: &UploadParams,
    chunks: &[Vec<u8>],
    start_from_chunk: usize,
    config: &UploadConfig,
) -> ChunkUploadResult {
    if chunks.is_empty() {
        return ChunkUploadResult::Failed("No chunks to upload".to_string());
    }

    if start_from_chunk >= chunks.len() {
        return ChunkUploadResult::Failed("Start chunk index exceeds total chunks".to_string());
    }

    for (relative_index, chunk) in chunks.iter().enumerate().skip(start_from_chunk) {
        match upload_chunk_with_config(params, chunk, relative_index, chunks.len(), config) {
            Ok(()) => continue,
            Err(e) => {
                if config.auto_resume {
                    return ChunkUploadResult::Interrupted {
                        failed_at_chunk: relative_index,
                        error: e,
                    };
                } else {
                    return ChunkUploadResult::Failed(e);
                }
            }
        }
    }

    ChunkUploadResult::Success
}

/// Executes a dfx command with the specified arguments.
///
/// # Arguments
///
/// * `command` - The main dfx command to run.
/// * `subcommand` - The subcommand to execute.
/// * `args` - A vector of arguments for the command.
/// * `network` - An optional network type.
///
/// # Returns
///
/// A `Result` containing the output of the command or an error message.
pub fn dfx(command: &str, subcommand: &str, args: &Vec<&str>, network: Option<&str>) -> Result<std::process::Output, String> {
    let mut dfx_command = Command::new("dfx");
    dfx_command.arg(command);
    dfx_command.arg(subcommand);

    if let Some(net) = network {
        dfx_command.arg("--network");
        dfx_command.arg(net);
    }

    for arg in args {
        dfx_command.arg(arg);
    }

    dfx_command.output().map_err(|e| e.to_string())
}

/// Creates a formatted error string.
///
/// # Arguments
///
/// * `message` - The error message to format.
///
/// # Returns
///
/// A formatted error string.
pub fn create_error_string(message: &str) -> String {
    format!("Upload Error: {message}")
}
