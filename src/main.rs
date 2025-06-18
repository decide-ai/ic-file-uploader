#![warn(missing_docs)]

//! This is a command-line tool for uploading files to Internet Computer canisters.
//!
//! It provides functionality to split files into chunks and upload them to specified canisters
//! using the Internet Computer protocol. The tool supports various options such as specifying
//! the canister name, method name, file path, and network type.

use std::fs;
use clap::Parser;
use std::path::Path;
use ic_file_uploader::{
    split_into_chunks, upload_chunks_with_resume, UploadConfig, UploadParams, ChunkUploadResult,
    MAX_CANISTER_HTTP_PAYLOAD_SIZE
};
use ic_file_uploader::parallel::{
    upload_chunks_parallel, chunks_to_chunk_info, ParallelUploadConfig, ParallelUploadResult
};

/// Command line arguments for the ic-file-uploader
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Name of the canister
    //#[arg(short, long)]
    canister_name: String,

    /// Name of the canister method
    //#[arg(short, long)]
    canister_method: String,

    /// Path to the file to be uploaded
    //#[arg(short, long)]
    file_path: String,

    /// Starting index for chunking (optional)
    #[arg(short, long, default_value = "0")]
    offset: usize,

    /// Starting chunk index for resume (0-based, optional)
    #[arg(long, default_value = "0")]
    chunk_offset: usize,

    /// Network type (optional)
    #[arg(short, long)]
    network: Option<String>,

    /// Enable autoresume with retry attempts
    #[arg(short, long)]
    autoresume: bool,

    /// Maximum retry attempts per chunk (default: 3)
    #[arg(long, default_value = "3")]
    max_retries: usize,

    /// Enable parallel uploads (experimental)
    #[arg(long)]
    parallel: bool,

    /// Maximum concurrent uploads for parallel mode (default: 4)
    #[arg(long, default_value = "4")]
    max_concurrent: usize,

    /// Target upload rate in MiB/s for parallel mode (default: 4.0)
    #[arg(long, default_value = "4.0")]
    target_rate: f64,

    /// Retry only specific chunk IDs from a file (comma-separated)
    #[arg(long)]
    retry_chunks_file: Option<String>,
}

/// Progress callback function for upload status
fn progress_callback(current: usize, total: usize, status: &str) {
    println!("Chunk {}/{}: {}", current, total, status);
}

/// Progress callback function for parallel upload status
fn parallel_progress_callback(chunk_id: u32, size: usize, status: &str) {
    println!("Chunk ID {}: {} ({} bytes)", chunk_id, status, size);
}

/// Rate monitoring callback for parallel uploads
fn rate_callback(current_rate: f64) {
    if current_rate > 0.1 {  // Only print if we have meaningful data
        print!("\rCurrent rate: {:.2} MiB/s", current_rate);
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
    }
}

/// The main function for the ic-file-uploader crate.
///
/// This function parses command line arguments, reads the specified file,
/// splits it into chunks, and uploads each chunk to the specified canister method.
fn main() -> Result<(), String> {
    let args = Args::parse();

    let bytes_path = Path::new(&args.file_path);
    println!("Uploading {}", args.file_path);

    let model_data = fs::read(&bytes_path).map_err(|e| e.to_string())?;

    // Create upload parameters
    let params = UploadParams {
        name: &format!("{} file", args.canister_name),
        canister_name: &args.canister_name,
        canister_method: &args.canister_method,
        network: args.network.as_deref(),
    };

    let model_chunks = split_into_chunks(model_data, MAX_CANISTER_HTTP_PAYLOAD_SIZE, args.offset);


    println!("Total chunks: {}", model_chunks.len());
    if args.offset > 0 {
        println!("Starting from byte offset: {}", args.offset);
    }
    if args.chunk_offset > 0 {
        println!("Starting from chunk {}", args.chunk_offset + 1);
    }
    if args.autoresume {
        println!("Auto-resume enabled with {} max retries per chunk", args.max_retries);
    }


    if args.parallel {
        println!("🚀 Using parallel upload mode");
        println!("Max concurrent: {}, Target rate: {:.1} MiB/s",
                 args.max_concurrent, args.target_rate);

        // Configure parallel upload
        let config = ParallelUploadConfig {
            max_concurrent: args.max_concurrent,
            target_rate_mibs: args.target_rate,
            max_retries: args.max_retries,
            retry_delay_ms: 1000,
            progress_callback: Some(parallel_progress_callback),
            rate_callback: Some(rate_callback),
        };

        // Convert chunks to ChunkInfo with IDs
        let chunk_infos = chunks_to_chunk_info(&model_chunks);

        // Filter chunks based on retry file or chunk_offset
        let chunks_to_upload: Vec<_> = if let Some(retry_file) = &args.retry_chunks_file {
            // Read failed chunk IDs from file
            match std::fs::read_to_string(retry_file) {
                Ok(content) => {
                    let retry_ids: Result<Vec<u32>, _> = content
                        .trim()
                        .split(',')
                        .map(|s| s.trim().parse::<u32>())
                        .collect();

                    match retry_ids {
                        Ok(ids) => {
                            println!("Retrying chunks: {:?}", ids);
                            let filtered: Vec<_> = chunk_infos
                                .into_iter()
                                .filter(|chunk| ids.contains(&chunk.chunk_id))
                                .collect();

                            if filtered.is_empty() {
                                return Err("No chunks to upload after applying chunk offset".to_string());
                            }

                            filtered
                        }
                        Err(e) => {
                            return Err(format!("Failed to parse chunk IDs from {}: {}", retry_file, e));
                        }
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to read retry chunks file {}: {}", retry_file, e));
                }
            }
        } else {
            // Use normal chunk_offset filtering
            chunk_infos
                .into_iter()
                .skip(args.chunk_offset)
                .collect()
        };

        if chunks_to_upload.is_empty() {
            return Err("No chunks to upload after applying chunk offset".to_string());
        }

        println!("Uploading {} chunks starting from ID {}",
                 chunks_to_upload.len(),
                 chunks_to_upload[0].chunk_id);


        // Perform parallel upload
        match upload_chunks_parallel(&params, chunks_to_upload, &config) {
            ParallelUploadResult::Success => {
                println!("\n✓ All chunks uploaded successfully!");
                Ok(())
            }
            ParallelUploadResult::PartialFailure { successful_chunks, failed_chunks } => {
                println!("\n⚠ Partial success:");
                println!("✓ Successful chunks: {:?}", successful_chunks);
                println!("✗ Failed chunks: {:?}", failed_chunks.keys().collect::<Vec<_>>());

                // Write failed chunk IDs to a file for easy retry
                let failed_ids: Vec<u32> = failed_chunks.keys().copied().collect();
                let failed_file = format!("{}.failed_chunks", args.file_path);

                match std::fs::write(&failed_file, failed_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")) {
                    Ok(()) => {
                        println!("\n📝 Failed chunk IDs written to: {}", failed_file);
                        println!("To retry failed chunks, run:");
                        println!("ic-file-uploader {} {} {} --parallel --retry-chunks-file {}{}",
                                 args.canister_name,
                                 args.canister_method,
                                 args.file_path,
                                 failed_file,
                                 args.network.as_ref().map(|n| format!(" --network {}", n)).unwrap_or_default());
                    }
                    Err(e) => {
                        println!("⚠ Could not write failed chunks file: {}", e);
                        println!("Failed chunk IDs: {}", failed_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(","));
                    }
                }

                Err("Some chunks failed to upload".to_string())
            }
            ParallelUploadResult::Failed(e) => {
                println!("\n✗ Upload failed: {}", e);

                Err(e)
            }
        }
    } else {
        println!("Using sequential upload mode");

        // Configure upload behavior - provide defaults for all parameters
        let config = UploadConfig {
            max_retries: args.max_retries,
            retry_delay_ms: 1000,  // Default 1 second delay
            auto_resume: args.autoresume,
            progress_callback: Some(progress_callback),
        };

        // Perform sequential upload with resume
        match upload_chunks_with_resume(&params, &model_chunks, args.chunk_offset, &config) {
            ChunkUploadResult::Success => {
                println!("✓ Upload completed successfully!");
                Ok(())
            }
            ChunkUploadResult::Failed(e) => {
                eprintln!("Upload failed: {}", e);
                Err(e)
            }
            ChunkUploadResult::Interrupted { failed_at_chunk, error } => {
                eprintln!("Upload interrupted at chunk {}: {}", failed_at_chunk + 1, error);
                println!("\nTo resume from this point, run:");
                println!("ic-file-uploader {} {} {} --chunk-offset {} --autoresume{}",
                         args.canister_name,
                         args.canister_method,
                         args.file_path,
                         failed_at_chunk,
                         args.network.as_ref().map(|n| format!(" --network {}", n)).unwrap_or_default());
                Err(format!("Upload interrupted at chunk {}", failed_at_chunk + 1))
            }
        }
    }
}

