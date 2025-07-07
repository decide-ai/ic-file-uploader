# ic-file-uploader Demo

This demo showcases how to use the `ic-file-uploader` tool to upload large files to Internet Computer canisters using parallel chunking. The demo includes a simple canister backend that receives and stores uploaded file chunks.

## Demo Components

- **Backend Canister**: A Rust canister that handles chunked file uploads
- **ic-file-uploader**: Command-line tool for uploading files in chunks
- **Storage System**: In-memory buffering with stable storage persistence

## Prerequisites

- [dfx](https://internetcomputer.org/docs/current/developer-docs/setup/install/) installed and configured
- [ic-file-uploader](https://crates.io/crates/ic-file-uploader) installed (`cargo install ic-file-uploader`)
- A binary file to upload (any file type: models, images, documents, etc.)

## Quick Start

### 1. Deploy the Demo Canister

```bash
# Start local dfx environment
dfx start --background

# Deploy the demo backend canister
dfx deploy
```

### 2. Upload a File

Replace `your-file.bin` with any binary file you want to test with:

```bash
# Upload your file using parallel chunking
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./your-file.bin --parallel

# Example with a machine learning model
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./model.safetensors --parallel

# Example with an image
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./large-image.jpg --parallel
```

### 3. Save to Stable Storage

After uploading, save the chunks to persistent storage with a custom key:

```bash
# Save with any key name you choose
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("my-file")'

# Examples with descriptive names
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("ml-model-v1")'
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("user-avatar")'
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("document-backup")'
```

### 4. Retrieve from Storage

Load the file back from stable storage to the working buffer:

```bash
# Load using the same key you used to save
dfx canister call ic-uploader-demo-backend load_from_stable '("my-file")'

# Or load your specific file
dfx canister call ic-uploader-demo-backend load_from_stable '("ml-model-v1")'
```

### 5. Monitor Storage Status

Check the current state of your uploads:

```bash
# View detailed storage information
dfx canister call ic-uploader-demo-backend storage_status

# Check parallel buffer status during upload
dfx canister call ic-uploader-demo-backend parallel_chunk_count
dfx canister call ic-uploader-demo-backend parallel_buffer_size
```

## Demo Workflow

Here's the complete flow the demo demonstrates:

```bash
# 1. Upload file chunks to parallel buffer
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./your-file.bin --parallel

# 2. Consolidate and save to stable storage
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("your-key-name")'

# 3. Verify storage status
dfx canister call ic-uploader-demo-backend storage_status

# 4. Load back to working buffer (for processing)
dfx canister call ic-uploader-demo-backend load_from_stable '("your-key-name")'

# 5. Retrieve data for use
dfx canister call ic-uploader-demo-backend get_data
```

## Canister Methods

The demo canister provides these methods:

### Parallel Upload Methods
- `append_parallel_chunk(chunk_id: nat32, chunk: blob)` - Store a chunk with ID
- `parallel_chunk_count() -> nat` - Get number of uploaded chunks
- `parallel_buffer_size() -> nat` - Get total size of parallel buffer
- `parallel_chunks_complete(expected: nat32) -> bool` - Check if upload is complete

### Storage Management  
- `save_parallel_to_stable(key: text) -> variant { Ok: nat; Err: text }` - Save chunks to stable storage
- `load_from_stable(key: text) -> variant { Ok; Err: text }` - Load from stable storage
- `get_stable_data(key: text) -> variant { Ok: blob; Err: text }` - Get data directly from stable storage

### Utility Methods
- `storage_status() -> text` - Get detailed storage status
- `clear_parallel_chunks()` - Clear parallel buffer
- `get_data() -> blob` - Get and clear working buffer

## File Types and Use Cases

This demo works with any binary file type:

### Machine Learning Models
```bash
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./model.safetensors --parallel
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("face-detection-model")'
```

### Media Files
```bash
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./video.mp4 --parallel
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("tutorial-video")'
```

### Data Archives
```bash
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./dataset.zip --parallel
dfx canister call ic-uploader-demo-backend save_parallel_to_stable '("training-data")'
```

## Performance Tips

- Use `--parallel` for files larger than 10MB
- Monitor upload progress with storage status calls
- Choose descriptive key names for easier file management
- Files are automatically chunked into 2MB pieces for optimal transfer

## Troubleshooting

### Upload hangs or fails
```bash
# Try with single concurrent upload
ic-file-uploader ic-uploader-demo-backend append_parallel_chunk ./file.bin --parallel --max-concurrent 1

# Check canister status
dfx canister call ic-uploader-demo-backend storage_status
```

### Storage issues
```bash
# Clear parallel buffer if needed
dfx canister call ic-uploader-demo-backend clear_parallel_chunks

# Check available methods
dfx canister call ic-uploader-demo-backend --help
```

## Next Steps

After running this demo, you can:

1. **Integrate into your own canister** - Copy the storage methods into your project
2. **Add processing logic** - Process uploaded files after they're loaded
3. **Build a file management system** - Create methods to list, delete, and organize stored files
4. **Add authentication** - Secure uploads with caller verification
5. **Scale for production** - Add memory management and file size limits

## Code Structure

- `lib.rs` - Main canister interface and memory management
- `storage.rs` - Core storage implementation with parallel upload support

The demo showcases a complete file upload pipeline suitable for production use cases like storing ML models, user content, or application assets on the Internet Computer.