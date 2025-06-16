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
    #[arg(short, long, default_value = "true")]
    autoresume: bool,

    /// Maximum retry attempts per chunk (default: 3)
    #[arg(long, default_value = "3")]
    max_retries: usize,
}

/// Progress callback function for upload status
fn progress_callback(current: usize, total: usize, status: &str) {
    println!("Chunk {}/{}: {}", current, total, status);
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
    let model_chunks = split_into_chunks(model_data, MAX_CANISTER_HTTP_PAYLOAD_SIZE, args.offset);

    // Create upload parameters
    let params = UploadParams {
        name: &format!("{} file", args.canister_name),
        canister_name: &args.canister_name,
        canister_method: &args.canister_method,
        network: args.network.as_deref(),
    };

    // Configure upload behavior - provide defaults for all parameters
    let config = UploadConfig {
        max_retries: args.max_retries,
        retry_delay_ms: 1000,  // Default 1 second delay
        auto_resume: args.autoresume,
        progress_callback: Some(progress_callback),
    };

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

    // Perform the upload
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
            println!("ic-file-uploader {} {} {} --offset {} --autoresume{}",
                     args.canister_name,
                     args.canister_method,
                     args.file_path,
                     failed_at_chunk,
                     args.network.as_ref().map(|n| format!(" --network {}", n)).unwrap_or_default());
            Err(format!("Upload interrupted at chunk {}", failed_at_chunk + 1))
        }
    }
}

