use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// Import the host function that will be provided by wasmtime
unsafe extern "C" {
    fn host_fetch(
        url_ptr: *const u8,
        url_len: usize,
        method_ptr: *const u8,
        method_len: usize,
    ) -> i64;
}

pub fn fetch(url: &str, method: &str) -> Result<NetworkResponse, String> {
    let url_bytes = url.as_bytes();
    let method_bytes = method.as_bytes();

    // Call the host function
    let result = unsafe {
        host_fetch(
            url_bytes.as_ptr(),
            url_bytes.len(),
            method_bytes.as_ptr(),
            method_bytes.len(),
        )
    };

    // Extract pointer and length from result
    let response_ptr = (result >> 32) as i32;
    let response_len = (result & 0xFFFFFFFF) as usize;

    if response_ptr == 0 {
        return Err("Network request failed".to_string());
    }

    // Read the response
    let response_bytes =
        unsafe { std::slice::from_raw_parts(response_ptr as *const u8, response_len) };

    // Deserialize the response
    let response: NetworkResponse = serde_json::from_slice(response_bytes)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(response)
}
