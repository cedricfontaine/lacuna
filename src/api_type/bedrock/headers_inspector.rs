use std::collections::HashMap;

use crate::inspector::protocol::ProtocolInspector;
use crate::inspector::protocol::text::{TextBody, TextProtocol};

use super::super::anthropic::parse_anthropic_json;
use super::super::{ApiTypeHandler, Inspector, ResponseMetadata, ResponseMetadataInspector};

pub struct BedrockModelInvokeJsonHandler;

impl ApiTypeHandler for BedrockModelInvokeJsonHandler {
    fn id(&self) -> &'static str {
        "bedrock_model_invoke"
    }

    fn response_inspector(
        &self,
        _status: u16,
        headers: &http::HeaderMap,
        _request_metadata: &crate::request_metadata::RequestInspectionMetadata,
    ) -> ResponseMetadataInspector {
        let input_tokens =
            parse_token_header(headers, "x-amzn-bedrock-input-token-count").unwrap_or(None);
        let output_tokens =
            parse_token_header(headers, "x-amzn-bedrock-output-token-count").unwrap_or(None);
        let cache_read_input_tokens =
            parse_token_header(headers, "x-amzn-bedrock-cache-read-input-token-count")
                .unwrap_or(None);
        let cache_creation_tokens =
            parse_token_header(headers, "x-amzn-bedrock-cache-write-input-token-count")
                .unwrap_or(None)
                .filter(|tokens| *tokens > 0)
                .map(|tokens| HashMap::from([("unknown".to_owned(), tokens)]));
        Box::new(ProtocolInspector::new(
            TextProtocol::new(),
            BedrockInvokeJsonInspector {
                header_metadata: ResponseMetadata {
                    input_tokens,
                    output_tokens,
                    cache_creation_tokens,
                    cache_read_input_tokens,
                },
                body_metadata: None,
            },
        ))
    }
}

/// Token counts come from the Anthropic usage block in the response body when
/// present, since it splits cache writes by TTL. The Bedrock token count
/// headers are the fallback for models whose body has no Anthropic usage block.
struct BedrockInvokeJsonInspector {
    header_metadata: ResponseMetadata,
    body_metadata: Option<ResponseMetadata>,
}

impl Inspector<TextBody> for BedrockInvokeJsonInspector {
    type Output = ResponseMetadata;

    fn feed(&mut self, body: TextBody) {
        self.body_metadata = parse_anthropic_json(&body.data).ok();
    }

    fn finish(self: Box<Self>) -> Result<ResponseMetadata, anyhow::Error> {
        let headers = self.header_metadata;
        let Some(body) = self.body_metadata else {
            return Ok(headers);
        };
        Ok(ResponseMetadata {
            input_tokens: body.input_tokens.or(headers.input_tokens),
            output_tokens: body.output_tokens.or(headers.output_tokens),
            cache_creation_tokens: body.cache_creation_tokens.or(headers.cache_creation_tokens),
            cache_read_input_tokens: body
                .cache_read_input_tokens
                .or(headers.cache_read_input_tokens),
        })
    }
}

fn parse_token_header(
    headers: &http::HeaderMap,
    header: &str,
) -> Result<Option<u64>, anyhow::Error> {
    match headers.get(header) {
        Some(value) => Ok(Some(value.to_str()?.parse::<u64>()?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_to_map(headers: &[(&str, &str)]) -> http::HeaderMap {
        let mut map = http::HeaderMap::new();
        for (key, value) in headers {
            map.insert(
                http::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn inspect_response_with_headers() {
        let headers = headers_to_map(&[
            ("x-amzn-bedrock-input-token-count", "25"),
            ("x-amzn-bedrock-output-token-count", "150"),
        ]);
        let inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, Some(25));
        assert_eq!(metadata.output_tokens, Some(150));
    }

    #[test]
    fn inspect_response_missing_headers() {
        let headers = http::HeaderMap::new();
        let inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, None);
        assert_eq!(metadata.output_tokens, None);
    }

    #[test]
    fn inspect_response_invalid_header() {
        let headers = headers_to_map(&[("x-amzn-bedrock-input-token-count", "not_a_number")]);
        let inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, None);
        assert_eq!(metadata.output_tokens, None);
    }

    #[test]
    fn inspect_response_body_with_cache_usage() {
        let headers = headers_to_map(&[
            ("x-amzn-bedrock-input-token-count", "25"),
            ("x-amzn-bedrock-output-token-count", "150"),
        ]);
        let mut inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        inspector.feed(
            br#"{"id":"msg_123","type":"message","usage":{"input_tokens":25,"output_tokens":150,"cache_creation_input_tokens":3000,"cache_read_input_tokens":12000,"cache_creation":{"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":2000}}}"#,
        );
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, Some(25));
        assert_eq!(metadata.output_tokens, Some(150));
        assert_eq!(metadata.cache_read_input_tokens, Some(12000));
        assert_eq!(
            metadata.cache_creation_tokens,
            Some(std::collections::HashMap::from([
                ("5m".to_owned(), 1000),
                ("1h".to_owned(), 2000),
            ])),
        );
    }

    #[test]
    fn inspect_response_non_anthropic_body_uses_headers() {
        let headers = headers_to_map(&[
            ("x-amzn-bedrock-input-token-count", "25"),
            ("x-amzn-bedrock-output-token-count", "150"),
        ]);
        let mut inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        inspector.feed(br#"{"generation":"Hi!","prompt_token_count":25}"#);
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, Some(25));
        assert_eq!(metadata.output_tokens, Some(150));
        assert_eq!(metadata.cache_read_input_tokens, None);
        assert_eq!(metadata.cache_creation_tokens, None);
    }

    #[test]
    fn inspect_response_cache_headers_fallback() {
        let headers = headers_to_map(&[
            ("x-amzn-bedrock-input-token-count", "25"),
            ("x-amzn-bedrock-output-token-count", "150"),
            ("x-amzn-bedrock-cache-read-input-token-count", "12000"),
            ("x-amzn-bedrock-cache-write-input-token-count", "3000"),
        ]);
        let mut inspector = BedrockModelInvokeJsonHandler.response_inspector(
            200,
            &headers,
            &crate::request_metadata::RequestInspectionMetadata::default(),
        );
        inspector.feed(br#"{"output":{"message":{"role":"assistant"}}}"#);
        let metadata = inspector.finish().unwrap();
        assert_eq!(metadata.input_tokens, Some(25));
        assert_eq!(metadata.output_tokens, Some(150));
        assert_eq!(metadata.cache_read_input_tokens, Some(12000));
        assert_eq!(
            metadata.cache_creation_tokens,
            Some(HashMap::from([("unknown".to_owned(), 3000)])),
        );
    }
}
