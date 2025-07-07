# ic-file-uploader

[![crates.io](https://img.shields.io/crates/v/ic-file-uploader.svg)](https://crates.io/crates/ic-file-uploader)
[![docs.rs](https://docs.rs/ic-file-uploader/badge.svg)](https://docs.rs/ic-file-uploader)

**ic-file-uploader** is a Rust crate designed to facilitate the efficient uploading of files larger than 2MB to the Internet Computer. This crate focuses on breaking down large files into manageable chunks that fit within packet size limits and passing them to update calls which write these chunks to files.

## Use Cases

- **Large File Handling**: Efficiently manage and upload large singular files.

## Features

- **Chunk-based Uploads**: Automatically splits large files into 2MB chunks for efficient transfer
- **Parallel Uploads**: Upload multiple chunks concurrently with configurable rate limiting
- **Resume Support**: Resume interrupted uploads from where they left off
- **Retry Logic**: Automatically retry failed chunks with exponential backoff
- **Progress Tracking**: Real-time progress reporting and upload rate monitoring
- **Flexible Configuration**: Customizable chunk size, retry attempts, and concurrency limits

## Installation

### From crates.io
```bash
cargo install ic-file-uploader
```

### From source
```bash
git clone <repository-url>
cd ic-file-uploader
cargo install --path .
```

## Usage

### Basic Upload
```bash
ic-file-uploader <canister_name> <method_name> <file_path>
```

### Parallel Upload (Recommended for large files)
```bash
ic-file-uploader <canister_name> <method_name> <file_path> --parallel --max-concurrent 4
```

### Resume an interrupted upload
```bash
ic-file-uploader <canister_name> <method_name> <file_path> --chunk-offset 10 --autoresume
```

### Upload with custom network
```bash
ic-file-uploader <canister_name> <method_name> <file_path> --network ic
```

### Retry specific failed chunks
```bash
ic-file-uploader <canister_name> <method_name> <file_path> --parallel --retry-chunks-file failed_chunks.txt
```

## Command Line Options

- `--parallel`: Enable parallel upload mode for better performance
- `--max-concurrent <N>`: Maximum number of concurrent uploads (default: 4)
- `--target-rate <RATE>`: Target upload rate in MiB/s (default: 4.0)
- `--chunk-offset <N>`: Start uploading from chunk N (for resume)
- `--autoresume`: Enable automatic resume with retry attempts
- `--max-retries <N>`: Maximum retry attempts per chunk (default: 3)
- `--network <NETWORK>`: Specify dfx network (local, ic, etc.)
- `--retry-chunks-file <FILE>`: Retry only specific chunk IDs from file

## Canister Integration

Your canister needs to implement methods that accept chunked data. For parallel uploads, the method should accept:

```candid
// For parallel uploads
append_parallel_chunk : (nat32, blob) -> ();

// For sequential uploads  
append_chunk : (blob) -> ();
```

Example Rust canister implementation:
```rust
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CHUNKS: RefCell<HashMap<u32, Vec<u8>>> = RefCell::new(HashMap::new());
}

#[ic_cdk::update]
fn append_parallel_chunk(chunk_id: u32, data: Vec<u8>) {
    CHUNKS.with(|chunks| {
        chunks.borrow_mut().insert(chunk_id, data);
    });
}
```

## Examples

### Upload a large model file
```bash
# Upload a 50MB machine learning model with parallel chunks
ic-file-uploader my_canister store_model ./large_model.safetensors --parallel --max-concurrent 6
```

### Resume a failed upload
```bash
# If upload fails at chunk 15, resume from there
ic-file-uploader my_canister store_data ./big_file.bin --chunk-offset 15 --autoresume
```

### Production upload with rate limiting
```bash
# Upload to IC mainnet with conservative rate limiting
ic-file-uploader my_canister store_file ./data.zip --parallel --target-rate 2.0 --network ic
```

## Performance Tips

- Use `--parallel` for files larger than 10MB
- Adjust `--max-concurrent` based on your network and canister capacity
- Use `--target-rate` to avoid overwhelming the canister
- Enable `--autoresume` for unreliable network connections

## Troubleshooting

### Upload hangs or doesn't complete
- Try reducing `--max-concurrent` to 1 or 2
- Lower the `--target-rate` 
- Check canister logs for memory or processing limits

### Chunks fail repeatedly
- Verify your canister method signature matches expected format
- Check canister cycle balance
- Ensure sufficient canister memory for storing chunks

### Resume not working
- Use `--chunk-offset` with the exact chunk number where upload failed
- Combine with `--autoresume` for automatic retry logic

## Use Cases

- **Large File Handling**: Upload datasets, models, and media files to IC canisters
- **Bulk Data Transfer**: Efficiently transfer large amounts of data with progress tracking
- **Reliable Uploads**: Resume interrupted transfers without starting over
- **CI/CD Integration**: Automated deployment of large assets to IC canisters

## Requirements

- Rust 1.70+ (for building from source)
- `dfx` command-line tool installed and configured
- Internet Computer canister with appropriate upload methods

## License

All original work is licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT), at your option.







