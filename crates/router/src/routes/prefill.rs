//! Prefill request construction helpers for vLLM and SGLang PD separation.
//!
//! Provides functions to modify a request for the prefill stage (forcing
//! `max_tokens=1`) and to build KV transfer parameters for the Nixl connector.

use serde_json::{json, Value};

/// Modify `request` so the vLLM worker only runs the prefill phase.
///
/// Detects the API type from `path` and applies the correct token-limit
/// patching. Always forces `stream=false` and removes `stream_options`.
pub fn prepare_prefill_request(mut request: Value, path: &str) -> Value {
    if path.contains("inference/v1/generate") {
        if let Some(sampling_params) = request.get_mut("sampling_params") {
            sampling_params["max_tokens"] = json!(1);
            if sampling_params
                .get("min_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 1
            {
                sampling_params["min_tokens"] = json!(1);
            }
        } else {
            request["sampling_params"] = json!({"max_tokens": 1, "min_tokens": 1});
        }
    } else if path.contains("/v1/responses") {
        request["max_output_tokens"] = json!(1);
    } else {
        // OpenAI chat/completions and completions
        request["max_tokens"] = json!(1);
        if request.get("max_completion_tokens").is_some() {
            request["max_completion_tokens"] = json!(1);
        }
        if request
            .get("min_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 1
        {
            request["min_tokens"] = json!(1);
        }
    }
    request["stream"] = json!(false);
    if let Some(obj) = request.as_object_mut() {
        obj.remove("stream_options");
    }
    request
}

/// Build the Nixl KV transfer params for the prefill request.
///
/// Tells the vLLM prefill worker to prepare for remote KV cache transfer.
pub fn build_prefill_kv_transfer_params() -> Value {
    json!({
        "do_remote_decode": true,
        "do_remote_prefill": false,
        "remote_engine_id": null,
        "remote_block_ids": null,
        "remote_host": null,
        "remote_port": null,
    })
}

/// Extract KV transfer params from the prefill response for the decode request.
///
/// For Nixl, this is the same `kv_transfer_params` blob that the prefill worker
/// returned. Returns `None` if the field is absent.
pub fn build_decode_kv_transfer_params(prefill_response: &Value) -> Option<Value> {
    prefill_response.get("kv_transfer_params").cloned()
}

// ── SGLang PD helpers ──────────────────────────────────────────────────────

/// Default bootstrap port for SGLang disaggregation.
pub const DEFAULT_BOOTSTRAP_PORT: u16 = 8998;

/// Extract the hostname from a worker URL for SGLang bootstrap.
///
/// Parses the URL and returns `host:port` (without scheme or path).
/// Returns the original URL unchanged if parsing fails.
pub fn extract_bootstrap_host(worker_url: &str) -> String {
    match url::Url::parse(worker_url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("127.0.0.1");
            let port = parsed.port().unwrap_or(8000);
            format!("{}:{}", host, port)
        }
        Err(_) => worker_url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string(),
    }
}

/// Build SGLang disaggregation bootstrap params injected into both prefill
/// and decode requests.
///
/// Returns `(bootstrap_host, bootstrap_port, bootstrap_room)` where
/// `bootstrap_room` is a random ID that pairs the prefill and decode sessions
/// for KV cache transfer.
pub fn build_sglang_bootstrap_params(prefill_url: &str, port: u16) -> (String, u16, i64) {
    let host = extract_bootstrap_host(prefill_url);
    let room = rand::random::<i64>().abs();
    (host, port, room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- chat/completions ---

    #[test]
    fn test_prefill_chat_completion() {
        let req = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}], "max_tokens": 512, "stream": true});
        let result = prepare_prefill_request(req, "/v1/chat/completions");
        assert_eq!(result["max_tokens"], 1);
        assert_eq!(result["stream"], false);
        assert!(result.get("stream_options").is_none());
    }

    #[test]
    fn test_prefill_chat_completion_max_completion_tokens() {
        let req = json!({"model": "m", "max_tokens": 512, "max_completion_tokens": 256});
        let result = prepare_prefill_request(req, "/v1/chat/completions");
        assert_eq!(result["max_completion_tokens"], 1);
    }

    #[test]
    fn test_prefill_clamps_min_tokens() {
        let req = json!({"model": "m", "max_tokens": 512, "min_tokens": 100});
        let result = prepare_prefill_request(req, "/v1/completions");
        assert_eq!(result["min_tokens"], 1);
    }

    #[test]
    fn test_prefill_leaves_small_min_tokens() {
        let req = json!({"model": "m", "max_tokens": 512, "min_tokens": 0});
        let result = prepare_prefill_request(req, "/v1/completions");
        assert_eq!(result["min_tokens"], 0);
    }

    // --- generate ---

    #[test]
    fn test_prefill_generate_patches_sampling_params() {
        let req = json!({"token_ids": [1,2,3], "sampling_params": {"max_tokens": 512, "temperature": 0.7}});
        let result = prepare_prefill_request(req, "/inference/v1/generate");
        assert_eq!(result["sampling_params"]["max_tokens"], 1);
        assert_eq!(result["sampling_params"]["temperature"], 0.7);
        assert!(result.get("max_tokens").is_none());
    }

    #[test]
    fn test_prefill_generate_clamps_min_tokens() {
        let req =
            json!({"token_ids": [1], "sampling_params": {"max_tokens": 512, "min_tokens": 50}});
        let result = prepare_prefill_request(req, "/inference/v1/generate");
        assert_eq!(result["sampling_params"]["min_tokens"], 1);
    }

    #[test]
    fn test_prefill_generate_without_sampling_params() {
        let req = json!({"token_ids": [1]});
        let result = prepare_prefill_request(req, "/inference/v1/generate");
        assert_eq!(result["stream"], false);
        assert_eq!(result["sampling_params"]["max_tokens"], 1);
        assert_eq!(result["sampling_params"]["min_tokens"], 1);
    }

    // --- responses ---

    #[test]
    fn test_prefill_responses() {
        let req =
            json!({"model": "m", "input": "hello", "max_output_tokens": 1024, "stream": true});
        let result = prepare_prefill_request(req, "/v1/responses");
        assert_eq!(result["max_output_tokens"], 1);
        assert!(result.get("max_tokens").is_none());
        assert_eq!(result["stream"], false);
    }

    // --- kv transfer params ---

    #[test]
    fn test_build_prefill_kv_transfer_params_shape() {
        let params = build_prefill_kv_transfer_params();
        assert_eq!(params["do_remote_decode"], true);
        assert_eq!(params["do_remote_prefill"], false);
        assert!(params["remote_engine_id"].is_null());
        assert!(params["remote_block_ids"].is_null());
        assert!(params["remote_host"].is_null());
        assert!(params["remote_port"].is_null());
    }

    #[test]
    fn test_build_decode_kv_transfer_params_extracts() {
        let resp = json!({"kv_transfer_params": {"do_remote_decode": true}});
        let params = build_decode_kv_transfer_params(&resp);
        assert!(params.is_some());
        assert_eq!(params.unwrap()["do_remote_decode"], true);
    }

    #[test]
    fn test_build_decode_kv_transfer_params_missing() {
        let resp = json!({"choices": []});
        assert!(build_decode_kv_transfer_params(&resp).is_none());
    }
}
