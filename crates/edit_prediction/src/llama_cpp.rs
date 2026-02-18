use crate::{
    EditPredictionId, EditPredictionModelInput, cursor_excerpt, prediction::EditPredictionResult,
};
use anyhow::{Context as _, Result};
use futures::AsyncReadExt as _;
use gpui::{App, AppContext as _, Entity, Task, http_client};
use language::{
    Anchor, Buffer, BufferSnapshot, OffsetRangeExt as _, ToOffset, ToPoint as _,
    language_settings::all_language_settings,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc, time::Instant};
use zeta_prompt::ZetaPromptInput;

const FIM_CONTEXT_TOKENS: usize = 512;
const TOP_K: u32 = 40;
const TOP_P: f32 = 0.99;
const TEMPERATURE: f32 = 0.2;
const CACHE_PROMPT: bool = true;
const INFILL_PROMPT: &str =
    "Fill in the missing code between prefix and suffix. Return only the missing text.";

pub struct LlamaCpp;

#[derive(Debug, Serialize)]
struct LlamaCppInfillRequest {
    input_prefix: String,
    input_suffix: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_prompt: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LlamaCppInfillResponse {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

/// Output from the llama.cpp HTTP request, containing all data needed to create the prediction
/// result.
struct LlamaCppRequestOutput {
    prediction_id: String,
    edits: Vec<(std::ops::Range<Anchor>, Arc<str>)>,
    snapshot: BufferSnapshot,
    response_received_at: Instant,
    inputs: ZetaPromptInput,
    buffer: Entity<Buffer>,
    buffer_snapshotted_at: Instant,
}

pub fn is_available(cx: &App) -> bool {
    !all_language_settings(None, cx)
        .edit_predictions
        .llama_cpp
        .api_url
        .trim()
        .is_empty()
}

impl LlamaCpp {
    pub fn new() -> Self {
        Self
    }

    pub fn request_prediction(
        &self,
        EditPredictionModelInput {
            buffer,
            snapshot,
            position,
            events,
            ..
        }: EditPredictionModelInput,
        cx: &mut App,
    ) -> Task<Result<Option<EditPredictionResult>>> {
        let settings = &all_language_settings(None, cx).edit_predictions.llama_cpp;
        let api_url = settings.api_url.to_string();
        let model = settings.model.clone();
        let max_output_tokens = settings.max_output_tokens;

        log::debug!(
            "llama.cpp: Requesting completion (model: {})",
            model.as_deref().unwrap_or("default")
        );

        let full_path: Arc<Path> = snapshot
            .file()
            .map(|file| file.full_path(cx))
            .unwrap_or_else(|| "untitled".into())
            .into();

        let http_client = cx.http_client();
        let cursor_point = position.to_point(&snapshot);
        let buffer_snapshotted_at = Instant::now();

        let result = cx.background_spawn(async move {
            let (excerpt_range, _) =
                cursor_excerpt::editable_and_context_ranges_for_cursor_position(
                    cursor_point,
                    &snapshot,
                    FIM_CONTEXT_TOKENS,
                    0,
                );
            let excerpt_offset_range = excerpt_range.to_offset(&snapshot);
            let cursor_offset = cursor_point.to_offset(&snapshot);

            let inputs = ZetaPromptInput {
                events,
                related_files: Vec::new(),
                cursor_offset_in_excerpt: cursor_offset - excerpt_offset_range.start,
                editable_range_in_excerpt: cursor_offset - excerpt_offset_range.start
                    ..cursor_offset - excerpt_offset_range.start,
                cursor_path: full_path.clone(),
                excerpt_start_row: Some(excerpt_range.start.row),
                cursor_excerpt: snapshot
                    .text_for_range(excerpt_range)
                    .collect::<String>()
                    .into(),
                excerpt_ranges: None,
                preferred_model: None,
                in_open_source_repo: false,
                can_collect_data: false,
            };

            let prefix = inputs.cursor_excerpt[..inputs.cursor_offset_in_excerpt].to_string();
            let suffix = inputs.cursor_excerpt[inputs.cursor_offset_in_excerpt..].to_string();
            let request = LlamaCppInfillRequest {
                input_prefix: prefix,
                input_suffix: suffix,
                prompt: INFILL_PROMPT.to_string(),
                stream: false,
                model,
                n_predict: Some(max_output_tokens),
                temperature: Some(TEMPERATURE),
                top_k: Some(TOP_K),
                top_p: Some(TOP_P),
                stop: Some(get_stop_tokens()),
                cache_prompt: Some(CACHE_PROMPT),
            };

            let request_body = serde_json::to_string(&request)?;
            let base_url = api_url.trim_end_matches('/');
            let request_url = if base_url.ends_with("/infill") {
                base_url.to_string()
            } else {
                format!("{base_url}/infill")
            };
            let http_request = http_client::Request::builder()
                .method(http_client::Method::POST)
                .uri(request_url)
                .header("Content-Type", "application/json")
                .body(http_client::AsyncBody::from(request_body))?;

            let mut response = http_client.send(http_request).await?;
            let status = response.status();

            log::debug!("llama.cpp: Response status: {}", status);

            if !status.is_success() {
                let mut body = String::new();
                response.body_mut().read_to_string(&mut body).await?;
                return Err(anyhow::anyhow!(
                    "llama.cpp API error: {} - {}",
                    status,
                    body
                ));
            }

            let mut body = String::new();
            response.body_mut().read_to_string(&mut body).await?;

            let llama_cpp_response: LlamaCppInfillResponse =
                serde_json::from_str(&body).context("Failed to parse llama.cpp response")?;
            let response_received_at = Instant::now();

            log::debug!(
                "llama.cpp: Completion received ({:.2}s)",
                (response_received_at - buffer_snapshotted_at).as_secs_f64()
            );

            let completion_text = llama_cpp_response
                .content
                .or(llama_cpp_response.completion)
                .unwrap_or_default();
            let completion: Arc<str> = clean_completion(&completion_text).into();
            let edits = if completion.is_empty() {
                vec![]
            } else {
                let anchor = snapshot.anchor_after(cursor_offset);
                vec![(anchor..anchor, completion)]
            };

            let timestamp_millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_millis())
                .unwrap_or_default();

            anyhow::Ok(LlamaCppRequestOutput {
                prediction_id: format!("llama_cpp_{}", timestamp_millis),
                edits,
                snapshot,
                response_received_at,
                inputs,
                buffer,
                buffer_snapshotted_at,
            })
        });

        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let output = result.await.context("llama.cpp edit prediction failed")?;
            anyhow::Ok(Some(
                EditPredictionResult::new(
                    EditPredictionId(output.prediction_id.into()),
                    &output.buffer,
                    &output.snapshot,
                    output.edits.into(),
                    None,
                    output.buffer_snapshotted_at,
                    output.response_received_at,
                    output.inputs,
                    cx,
                )
                .await,
            ))
        })
    }
}

fn get_stop_tokens() -> Vec<String> {
    vec![
        "<|endoftext|>".to_string(),
        "<|file_separator|>".to_string(),
        "<|fim_pad|>".to_string(),
        "<|fim_prefix|>".to_string(),
        "<|fim_middle|>".to_string(),
        "<|fim_suffix|>".to_string(),
        "<fim_prefix>".to_string(),
        "<fim_middle>".to_string(),
        "<fim_suffix>".to_string(),
        "<PRE>".to_string(),
        "<SUF>".to_string(),
        "<MID>".to_string(),
        "[PREFIX]".to_string(),
        "[SUFFIX]".to_string(),
    ]
}

fn clean_completion(response: &str) -> String {
    let mut result = response.to_string();

    let end_tokens = [
        "<|endoftext|>",
        "<|file_separator|>",
        "<|fim_pad|>",
        "<|fim_prefix|>",
        "<|fim_middle|>",
        "<|fim_suffix|>",
        "<fim_prefix>",
        "<fim_middle>",
        "<fim_suffix>",
        "<PRE>",
        "<SUF>",
        "<MID>",
        "[PREFIX]",
        "[SUFFIX]",
    ];

    for token in &end_tokens {
        if let Some(position) = result.find(token) {
            result.truncate(position);
        }
    }

    result
}
