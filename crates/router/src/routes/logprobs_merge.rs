//! Logprobs merging utilities for PD disaggregation.
//!
//! Merges prompt logprobs from the prefill response into the decode response so
//! the client receives complete logprobs covering both prompt and generated tokens.

use serde_json::Value;

/// Merge prompt logprobs from the prefill response into the decode response.
///
/// Handles three API shapes:
/// - Chat Completions: copies top-level `prompt_logprobs`
/// - Completions: merges per-choice `prompt_logprobs`, `token_logprobs`, `tokens`,
///   `text_offset`, and `top_logprobs`
/// - Generate: merges `meta_info.input_token_logprobs`
///
/// Returns `true` if any logprobs were merged.
pub fn merge_logprobs_in_json(prefill_json: &Value, decode_json: &mut Value) -> bool {
    let mut merged = false;

    // 1. Generate API: merge meta_info.input_token_logprobs
    if let (Some(prefill_meta), Some(decode_meta)) = (
        prefill_json.get("meta_info"),
        decode_json.get_mut("meta_info"),
    ) {
        if let (Some(prefill_logprobs), Some(decode_logprobs)) = (
            prefill_meta.get("input_token_logprobs"),
            decode_meta.get_mut("input_token_logprobs"),
        ) {
            if let (Some(prefill_arr), Some(decode_arr)) =
                (prefill_logprobs.as_array(), decode_logprobs.as_array_mut())
            {
                let mut merged_logprobs = prefill_arr.clone();
                merged_logprobs.extend(decode_arr.clone());
                decode_meta["input_token_logprobs"] = Value::Array(merged_logprobs);
                merged = true;
            }
        }
    }

    // 2. Chat Completions API: copy top-level prompt_logprobs
    if let Some(prefill_prompt_logprobs) = prefill_json.get("prompt_logprobs") {
        if let Some(decode_obj) = decode_json.as_object_mut() {
            decode_obj.insert(
                "prompt_logprobs".to_string(),
                prefill_prompt_logprobs.clone(),
            );
            merged = true;
        }
    }

    // 3. Completions API: merge per-choice logprobs
    if let Some(choices) = decode_json
        .get_mut("choices")
        .and_then(|v| v.as_array_mut())
    {
        if let Some(prefill_choices) = prefill_json.get("choices").and_then(|v| v.as_array()) {
            for (decode_choice, prefill_choice) in choices.iter_mut().zip(prefill_choices.iter()) {
                if let (Some(decode_obj), Some(prefill_obj)) =
                    (decode_choice.as_object_mut(), prefill_choice.as_object())
                {
                    // 3.1. Copy top-level prompt_logprobs field
                    if let Some(ppl) = prefill_obj.get("prompt_logprobs") {
                        decode_obj.insert("prompt_logprobs".to_string(), ppl.clone());
                        merged = true;
                    }

                    // 3.2. Merge logprobs object (token_logprobs, tokens, text_offset, top_logprobs)
                    if let (Some(prefill_lp), Some(decode_lp)) =
                        (prefill_obj.get("logprobs"), decode_obj.get_mut("logprobs"))
                    {
                        if let (Some(plp_obj), Some(dlp_obj)) =
                            (prefill_lp.as_object(), decode_lp.as_object_mut())
                        {
                            let num_prompt = prefill_obj
                                .get("prompt_logprobs")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);

                            if let (Some(ptl), Some(dtl)) = (
                                plp_obj.get("token_logprobs").and_then(|v| v.as_array()),
                                dlp_obj.get("token_logprobs").and_then(|v| v.as_array()),
                            ) {
                                let prefill_prompt_only = &ptl[..num_prompt.min(ptl.len())];
                                let mut merged_tl = prefill_prompt_only.to_vec();
                                merged_tl.extend(dtl.clone());
                                dlp_obj
                                    .insert("token_logprobs".to_string(), Value::Array(merged_tl));
                                merged = true;
                            }

                            // Capture tokens once for use in both merge and offset adjustment.
                            let prefill_tokens_arr =
                                plp_obj.get("tokens").and_then(|v| v.as_array());

                            if let (Some(pt), Some(dt)) = (
                                prefill_tokens_arr,
                                dlp_obj.get("tokens").and_then(|v| v.as_array()),
                            ) {
                                let prefill_prompt_only = &pt[..num_prompt.min(pt.len())];
                                let mut merged_t = prefill_prompt_only.to_vec();
                                merged_t.extend(dt.clone());
                                dlp_obj.insert("tokens".to_string(), Value::Array(merged_t));
                                merged = true;
                            }

                            if let (Some(po), Some(do_)) = (
                                plp_obj.get("text_offset").and_then(|v| v.as_array()),
                                dlp_obj.get("text_offset").and_then(|v| v.as_array()),
                            ) {
                                let prefill_prompt_only = &po[..num_prompt.min(po.len())];
                                let mut merged_offsets = prefill_prompt_only.to_vec();
                                if !prefill_prompt_only.is_empty() {
                                    let last_offset = prefill_prompt_only
                                        .last()
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    let last_token_len = prefill_tokens_arr
                                        .and_then(|tokens| {
                                            tokens
                                                .get(num_prompt.min(tokens.len()).saturating_sub(1))
                                                .and_then(|t| t.as_str())
                                                .map(|s| s.len() as i64)
                                        })
                                        .unwrap_or(0);
                                    let base = last_offset + last_token_len;
                                    let adjusted: Vec<Value> = do_
                                        .iter()
                                        .filter_map(|v| v.as_i64().map(|o| Value::from(o + base)))
                                        .collect();
                                    merged_offsets.extend(adjusted);
                                } else {
                                    merged_offsets.extend(do_.clone());
                                }
                                dlp_obj.insert(
                                    "text_offset".to_string(),
                                    Value::Array(merged_offsets),
                                );
                                merged = true;
                            }

                            if let (Some(ptl), Some(dtl)) = (
                                plp_obj.get("top_logprobs").and_then(|v| v.as_array()),
                                dlp_obj.get("top_logprobs").and_then(|v| v.as_array()),
                            ) {
                                let prefill_prompt_only = &ptl[..num_prompt.min(ptl.len())];
                                let mut merged_top = prefill_prompt_only.to_vec();
                                merged_top.extend(dtl.clone());
                                dlp_obj
                                    .insert("top_logprobs".to_string(), Value::Array(merged_top));
                                merged = true;
                            }
                        }
                    }
                }
            }
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_merge_chat_completions_api() {
        let prefill = json!({"prompt_logprobs": [null, -0.5, -1.2]});
        let mut decode = json!({"choices": [{"message": {"content": "hi"}}]});
        assert!(merge_logprobs_in_json(&prefill, &mut decode));
        assert_eq!(decode["prompt_logprobs"], json!([null, -0.5, -1.2]));
    }

    #[test]
    fn test_merge_completions_api() {
        let prefill = json!({
            "choices": [{
                "prompt_logprobs": [null, -0.5, -1.2],
                "logprobs": {
                    "token_logprobs": [null, -0.5, -1.2, -2.1],
                    "tokens": ["Hello", " world", " test", " extra"],
                    "text_offset": [0, 5, 11, 16],
                    "top_logprobs": [null, {" world": -0.5}, {" test": -1.2}, {" extra": -2.1}]
                }
            }]
        });
        let mut decode = json!({
            "choices": [{
                "logprobs": {
                    "token_logprobs": [-3.5, -4.2],
                    "tokens": [" output", " token"],
                    "text_offset": [0, 7],
                    "top_logprobs": [{" output": -3.5}, {" token": -4.2}]
                }
            }]
        });
        assert!(merge_logprobs_in_json(&prefill, &mut decode));
        let merged_tokens = decode["choices"][0]["logprobs"]["tokens"]
            .as_array()
            .unwrap();
        assert_eq!(merged_tokens.len(), 5);
        assert_eq!(merged_tokens[0], "Hello");
        assert_eq!(merged_tokens[4], " token");
    }

    #[test]
    fn test_merge_no_logprobs_returns_false() {
        let prefill = json!({"choices": [{"index": 0}]});
        let mut decode = json!({"choices": [{"text": "hi"}]});
        assert!(!merge_logprobs_in_json(&prefill, &mut decode));
    }
}
