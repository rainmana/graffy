//! HTTP adapter for decision models (ADR-0012): the implementation behind
//! [`graffy_core::route::Decider`].
//!
//! It speaks the "system one" wire shape — `POST /v1/systemone` with `state`,
//! named `questions`, and a `choice` question whose `criteria` map each
//! option to when it applies. Verified on 2026-11-15 against TypeSafe's
//! hosted Jev (api.typesafe.ai OpenAPI document) and the Strands Decider
//! server's request/response schema (strands-labs/strands-decider,
//! `src/strands_decider/schema.py`). Both return `choice`, `confidence` and
//! `probabilities`.
//!
//! Nothing is hardcoded: endpoint, model and key come from the environment.
//!
//! ```text
//! GRAFFY_DECIDER_URL=https://api.typesafe.ai     # or http://127.0.0.1:8000
//! GRAFFY_DECIDER_MODEL=<a name from GET /v1/models>   # optional for a local server
//! GRAFFY_DECIDER_API_KEY=<key>                   # optional; sent as a Bearer token
//! ```

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use graffy_core::route::{Decider, DeciderAnswer, DeciderError};

/// A decision model reached over HTTP.
pub struct HttpDecider {
    client: reqwest::Client,
    url: String,
    endpoint: String,
    model: Option<String>,
    key: Option<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl HttpDecider {
    /// `base_url` is the service root, e.g. `https://api.typesafe.ai`.
    pub fn new(
        base_url: &str,
        model: Option<String>,
        key: Option<String>,
    ) -> Result<Self, DeciderError> {
        // Parse, never split by hand: `http://localhost:pw@example.com` has
        // the host example.com, and must not pass for this machine.
        let invalid = |why: &str| DeciderError(format!("decider URL '{base_url}' {why}"));
        let mut url = reqwest::Url::parse(base_url.trim())
            .map_err(|err| invalid(&format!("is not a valid URL: {err}")))?;
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| invalid("has no host"))?
            .to_owned();
        if !url.username().is_empty() || url.password().is_some() {
            // Do not echo the URL: it holds a credential.
            return Err(DeciderError(
                "decider URL must not contain a username or password; use GRAFFY_DECIDER_API_KEY"
                    .into(),
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("must not contain a query or fragment"));
        }
        let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
        match url.scheme() {
            "https" => {}
            // Prompts (and a key) must not cross a network in clear text.
            "http" if local => {}
            "http" => {
                return Err(invalid(
                    "uses plain http:// for a non-local host; use https://",
                ));
            }
            _ => return Err(invalid("must start with https:// or http://")),
        }
        let endpoint = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host,
        };
        let path = format!("{}/v1/systemone", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| DeciderError(format!("cannot build HTTP client: {err}")))?;
        Ok(Self {
            client,
            url: url.into(),
            endpoint,
            model,
            key,
        })
    }

    /// Build from `GRAFFY_DECIDER_URL`, `GRAFFY_DECIDER_MODEL` and
    /// `GRAFFY_DECIDER_API_KEY`. A missing URL is an error, never a default.
    pub fn from_env() -> Result<Self, DeciderError> {
        let url = env("GRAFFY_DECIDER_URL").ok_or_else(|| {
            DeciderError(
                "GRAFFY_DECIDER_URL is not set (e.g. https://api.typesafe.ai for Jev, or \
                 http://127.0.0.1:8000 for a local Strands Decider server)"
                    .into(),
            )
        })?;
        Self::new(
            &url,
            env("GRAFFY_DECIDER_MODEL"),
            env("GRAFFY_DECIDER_API_KEY"),
        )
    }
}

#[async_trait::async_trait]
impl Decider for HttpDecider {
    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn choose(
        &self,
        content: &str,
        instructions: &str,
        choices: &BTreeMap<String, String>,
    ) -> Result<DeciderAnswer, DeciderError> {
        let mut body = serde_json::json!({
            "state": content,
            "questions": {
                "route": { "type": "choice", "instructions": instructions, "criteria": choices }
            }
        });
        if let Some(model) = &self.model {
            body["model"] = serde_json::Value::String(model.clone());
        }
        let mut request = self.client.post(&self.url).json(&body);
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        let started = Instant::now();
        let response = request
            .send()
            .await
            .map_err(|err| DeciderError(format!("request failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DeciderError(format!("unreadable response: {err}")))?;
        let latency_ms = started.elapsed().as_millis() as u64;
        if !status.is_success() {
            let detail: String = text.chars().take(300).collect();
            return Err(DeciderError(format!("HTTP {status}: {detail}")));
        }
        parse_answer(&text, latency_ms)
    }
}

/// Read a `/v1/systemone` response. Every required field must be present:
/// a partial answer is an error, never a guess.
fn parse_answer(text: &str, latency_ms: u64) -> Result<DeciderAnswer, DeciderError> {
    let json: serde_json::Value = serde_json::from_str(text)
        .map_err(|err| DeciderError(format!("response is not JSON: {err}")))?;
    let answer = &json["answers"]["route"];
    let missing =
        |field: &str| DeciderError(format!("response has no usable answers.route.{field}"));
    let choice = answer["choice"].as_str().ok_or_else(|| missing("choice"))?;
    let confidence = answer["confidence"]
        .as_f64()
        .ok_or_else(|| missing("confidence"))?;
    let probabilities = answer["probabilities"]
        .as_object()
        .ok_or_else(|| missing("probabilities"))?
        .iter()
        .filter_map(|(name, value)| value.as_f64().map(|p| (name.clone(), p)))
        .collect();
    Ok(DeciderAnswer {
        choice: choice.to_owned(),
        confidence,
        probabilities,
        model: json["model"]
            .as_str()
            .ok_or_else(|| DeciderError("response does not name the model that answered".into()))?
            .to_owned(),
        // Usage is reported by the service; absent means it reported none.
        input_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: json["usage"]["output_tokens"].as_u64().unwrap_or(0),
        latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A one-request HTTP server returning `status` and `body`; hands back
    /// the raw request it received.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if sent.len() >= length {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }

    fn choices() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("t.decide".to_owned(), "Weighing options.".to_owned()),
            ("t.chat".to_owned(), "Ordinary conversation.".to_owned()),
        ])
    }

    #[tokio::test]
    async fn sends_the_systemone_shape_and_reads_the_models_own_report() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"model":"fixture-1.0","answers":{"route":{"type":"choice","choice":"t.decide","confidence":0.93,"probabilities":{"t.decide":0.93,"t.chat":0.07}}},"usage":{"input_tokens":41,"output_tokens":1}}"#,
        );
        let decider =
            HttpDecider::new(&url, Some("fixture-latest".into()), Some("k-test".into())).unwrap();
        let answer = decider
            .choose("rent or buy?", "Pick one.", &choices())
            .await
            .unwrap();
        assert_eq!(
            (answer.choice.as_str(), answer.confidence),
            ("t.decide", 0.93)
        );
        assert_eq!(
            answer.model, "fixture-1.0",
            "the answering model, not the alias asked for"
        );
        assert_eq!((answer.input_tokens, answer.output_tokens), (41, 1));
        assert_eq!(answer.probabilities["t.chat"], 0.07);

        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/systemone "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer k-test")
        );
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["state"], "rent or buy?");
        assert_eq!(body["model"], "fixture-latest");
        assert_eq!(body["questions"]["route"]["type"], "choice");
        assert_eq!(
            body["questions"]["route"]["criteria"]["t.decide"],
            "Weighing options."
        );
    }

    #[tokio::test]
    async fn errors_are_loud_and_a_partial_answer_is_not_a_guess() {
        let (url, server) = serve_once("401 Unauthorized", r#"{"detail":"bad key"}"#);
        let err = HttpDecider::new(&url, None, None)
            .unwrap()
            .choose("x", "y", &choices())
            .await
            .unwrap_err();
        assert!(err.0.contains("401") && err.0.contains("bad key"), "{err}");
        // No model named and no key: neither is invented in the request.
        let request = server.join().unwrap();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(!request.contains("\"model\""));

        assert!(parse_answer(r#"{"model":"m","answers":{"route":{"choice":"a"}}}"#, 0).is_err());
        assert!(
            parse_answer(
                r#"{"answers":{"route":{"choice":"a","confidence":0.9,"probabilities":{}}}}"#,
                0
            )
            .is_err()
        );
        assert!(parse_answer("not json", 0).is_err());
    }

    #[test]
    fn plain_http_is_only_for_this_machine() {
        assert!(HttpDecider::new("http://127.0.0.1:8000", None, None).is_ok());
        assert!(HttpDecider::new("http://localhost:8000/", None, None).is_ok());
        assert!(HttpDecider::new("https://api.example.com", None, None).is_ok());
        assert!(HttpDecider::new("http://api.example.com", None, None).is_err());
        assert!(HttpDecider::new("api.example.com", None, None).is_err());
        assert_eq!(
            HttpDecider::new("https://api.example.com/base/", None, None)
                .unwrap()
                .endpoint(),
            "api.example.com"
        );
        // The host is whatever the URL parser says it is — credentials in the
        // authority cannot disguise a remote host as this machine.
        for disguised in [
            "http://localhost:password@example.com",
            "http://127.0.0.1@example.com",
            "http://localhost@example.com:8000",
            "https://user:secret@api.example.com",
            "http://localhost.example.com",
            "https://api.example.com/?key=1",
            "ftp://localhost",
        ] {
            assert!(
                HttpDecider::new(disguised, None, None).is_err(),
                "{disguised}"
            );
        }
        // A credential mistakenly placed in the URL is never echoed back.
        let leaked = HttpDecider::new("https://user:s3cret@api.example.com", None, None)
            .err()
            .unwrap();
        assert!(!leaked.0.contains("s3cret") && leaked.0.contains("GRAFFY_DECIDER_API_KEY"));
        let prefixed = HttpDecider::new("https://api.example.com:8443/base/", None, None).unwrap();
        assert_eq!(prefixed.endpoint(), "api.example.com:8443");
        assert_eq!(
            prefixed.url,
            "https://api.example.com:8443/base/v1/systemone"
        );
        assert!(HttpDecider::new("http://[::1]:8000", None, None).is_ok());
    }
}
