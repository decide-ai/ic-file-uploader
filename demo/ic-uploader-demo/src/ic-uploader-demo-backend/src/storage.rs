// src/storage.rs
//! Ultra-simple storage: one heap buffer, stable storage with keys

use std::cell::RefCell;
use std::collections::HashMap;
use crate::REGISTRIES;

// Single buffer in heap - only one large object at a time
thread_local! {
    static BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    static BUFFER_MAP: RefCell<HashMap<u32, Vec<u8>>> = RefCell::new(HashMap::new());
}

// ─────────────────────────────────────────────────────
//  IC Canister Endpoints - Original Sequential
// ─────────────────────────────────────────────────────

/// Append chunk to the single heap buffer
#[ic_cdk::update]
pub fn append_chunk(chunk: Vec<u8>) {
    BUFFER.with(|buffer| {
        buffer.borrow_mut().extend(chunk);
    });
}

/// Get current buffer size
#[ic_cdk::query]
pub fn buffer_size() -> usize {
    BUFFER.with(|buffer| buffer.borrow().len())
}

/// Clear the heap buffer
#[ic_cdk::update]
pub fn clear_buffer() {
    BUFFER.with(|buffer| {
        buffer.borrow_mut().clear();
    });
}


// ─────────────────────────────────────────────────────
//  IC Canister Endpoints - Parallel Chunk Support
// ─────────────────────────────────────────────────────

/// Append chunk with ID for parallel uploads
#[ic_cdk::update]
pub fn append_parallel_chunk(chunk_id: u32, chunk: Vec<u8>) {
    BUFFER_MAP.with(|buffer_map| {
        buffer_map.borrow_mut().insert(chunk_id, chunk);
    });
}

/// Get number of chunks in the parallel buffer
#[ic_cdk::query]
pub fn parallel_chunk_count() -> usize {
    BUFFER_MAP.with(|buffer_map| buffer_map.borrow().len())
}

/// Get list of chunk IDs currently in the parallel buffer
#[ic_cdk::query]
pub fn parallel_chunk_ids() -> Vec<u32> {
    BUFFER_MAP.with(|buffer_map| {
        let mut ids: Vec<u32> = buffer_map.borrow().keys().copied().collect();
        ids.sort();
        ids
    })
}

/// Get total size of all chunks in parallel buffer
#[ic_cdk::query]
pub fn parallel_buffer_size() -> usize {
    BUFFER_MAP.with(|buffer_map| {
        buffer_map.borrow().values().map(|chunk| chunk.len()).sum()
    })
}

/// Check if all chunks from 0 to max_chunk_id are present (for completeness validation)
#[ic_cdk::query]
pub fn parallel_chunks_complete(expected_count: u32) -> bool {
    BUFFER_MAP.with(|buffer_map| {
        let buffer_map = buffer_map.borrow();
        if buffer_map.len() != expected_count as usize {
            return false;
        }

        // Check that we have consecutive chunks from 0 to expected_count-1
        for i in 0..expected_count {
            if !buffer_map.contains_key(&i) {
                return false;
            }
        }
        true
    })
}

/// Clear all parallel chunks
#[ic_cdk::update]
pub fn clear_parallel_chunks() {
    BUFFER_MAP.with(|buffer_map| {
        buffer_map.borrow_mut().clear();
    });
}

/// Remove a specific chunk from parallel buffer (useful for retry scenarios)
#[ic_cdk::update]
pub fn remove_parallel_chunk(chunk_id: u32) -> bool {
    BUFFER_MAP.with(|buffer_map| {
        buffer_map.borrow_mut().remove(&chunk_id).is_some()
    })
}

// ─────────────────────────────────────────────────────
//  IC Canister Endpoints - Enhanced Stable Storage
// ─────────────────────────────────────────────────────

/// Save buffer to stable storage with key and clear buffer
#[ic_cdk::update]
pub fn save_to_stable(key: String) -> Result<(), String> {
    let data = BUFFER.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        let data = buffer.clone();
        buffer.clear();
        data
    });

    if data.is_empty() {
        return Err(format!("No data in buffer for key: {}", key));
    }

    REGISTRIES.with(|map| {
        map.borrow_mut().insert(key, data);
    });

    Ok(())
}

/// Save parallel chunks directly to stable storage (consolidates and saves in one step)
#[ic_cdk::update]
pub fn save_parallel_to_stable(key: String) -> Result<usize, String> {
    let consolidated_data = BUFFER_MAP.with(|buffer_map| {
        let mut buffer_map = buffer_map.borrow_mut();

        if buffer_map.is_empty() {
            return Vec::new();
        }

        // Sort chunk IDs and collect data in order
        let mut sorted_ids: Vec<u32> = buffer_map.keys().copied().collect();
        sorted_ids.sort();

        let mut consolidated_data = Vec::new();

        for chunk_id in sorted_ids {
            if let Some(chunk) = buffer_map.remove(&chunk_id) {
                consolidated_data.extend(chunk);
            }
        }

        // Clear the map after consolidation
        buffer_map.clear();

        consolidated_data
    });

    if consolidated_data.is_empty() {
        return Err(format!("No parallel chunks to save for key: {}", key));
    }

    let data_size = consolidated_data.len();

    REGISTRIES.with(|map| {
        map.borrow_mut().insert(key, consolidated_data);
    });

    Ok(data_size)
}


/// Load from stable storage to buffer
#[ic_cdk::update]
pub fn load_from_stable(key: String) -> Result<(), String> {
    REGISTRIES.with(|map| {
        if let Some(data) = map.borrow().get(&key) {
            BUFFER.with(|buffer| {
                buffer.borrow_mut().clone_from(&data);
            });
            Ok(())
        } else {
            Err(format!("No data found in stable storage for key: {}", key))
        }
    })
}

/// Get buffered data (consumes the buffer)
#[ic_cdk::update]
pub fn get_data() -> Vec<u8> {
    BUFFER.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        let data = buffer.clone();
        buffer.clear();
        data
    })
}

/// Get data directly from stable storage
#[ic_cdk::query]
pub fn get_stable_data(key: String) -> Result<Vec<u8>, String> {
    REGISTRIES.with(|map| {
        map.borrow().get(&key)
            .ok_or_else(|| format!("No data found in stable storage for key: {}", key))
    })
}

// ─────────────────────────────────────────────────────
//  Helper Functions for Debugging and Monitoring
// ─────────────────────────────────────────────────────

/// Get storage status summary
#[ic_cdk::query]
pub fn storage_status() -> String {
    let buffer_size = BUFFER.with(|buffer| buffer.borrow().len());
    let (chunk_count, parallel_size, chunk_ids) = BUFFER_MAP.with(|buffer_map| {
        let buffer_map = buffer_map.borrow();
        let count = buffer_map.len();
        let size = buffer_map.values().map(|chunk| chunk.len()).sum::<usize>();
        let mut ids: Vec<u32> = buffer_map.keys().copied().collect();
        ids.sort();
        (count, size, ids)
    });

    let stable_keys = REGISTRIES.with(|map| {
        map.borrow().iter().map(|(k, v)| format!("{}: {} bytes", k, v.len())).collect::<Vec<_>>()
    });

    format!(
        "Buffer: {} bytes\nParallel chunks: {} chunks, {} bytes total\nChunk IDs: {:?}\nStable storage: [{}]",
        buffer_size,
        chunk_count,
        parallel_size,
        chunk_ids,
        stable_keys.join(", ")
    )
}