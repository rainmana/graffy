//! A revised tool digest must not repeat the external tool effect.
use graffy_core::{
    error::{ModelError, ToolError},
    exec::{
        AutoApprove, Executor, ModelInvoker, ModelRequest, ModelResponse, RunInput, ToolInvoker,
        ToolResponse,
    },
    journal::{JournalReader, wire},
};
use graffy_mcp::{DiscoveredTool, generate_facade};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Model {
    reviews: AtomicUsize,
}
#[async_trait::async_trait]
impl ModelInvoker for Model {
    async fn complete(&self, r: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let text = match r.purpose.as_str() {
            "prepare" => r#"{"message":"hello"}"#,
            "verify" => {
                assert!(r.prompt.contains("TOOL_RESULT_RECEIPT"));
                if self.reviews.fetch_add(1, Ordering::SeqCst) == 0 {
                    "REVISE\nMODE: overcompression\nPreserve the result"
                } else {
                    "PASS"
                }
            }
            _ => "digest of the result",
        };
        Ok(ModelResponse {
            provider: "fake".into(),
            model: "local".into(),
            text: text.into(),
            input_tokens: 1,
            output_tokens: 1,
            cost_usd: 0.0,
            latency_ms: 0,
        })
    }
}
#[derive(Default)]
struct Tool(AtomicUsize);
#[async_trait::async_trait]
impl ToolInvoker for Tool {
    async fn invoke(&self, _: &str, _: &str, args: &str) -> Result<ToolResponse, ToolError> {
        assert!(args.contains("hello"));
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResponse {
            text: "TOOL_RESULT_RECEIPT".into(),
            is_error: false,
            latency_ms: 0,
        })
    }
}
#[tokio::test]
async fn digest_revision_is_reviewed_without_repeating_tool_effect() {
    let spec = generate_facade(
        "fixture",
        &DiscoveredTool {
            name: "echo".into(),
            description: "echo".into(),
            read_only: Some(false),
            destructive: Some(true),
            schema_json: "{}".into(),
        },
        "effector",
        "L1",
        None,
    );
    let tool = Arc::new(Tool::default());
    let model = Model::default();
    let path = std::env::temp_dir().join(format!(
        "graffy-facade-review-{}.journal",
        graffy_core::id::RunId::generate()
    ));
    let result = Executor {
        tool_invoker: Some(tool.clone()),
        ..Default::default()
    }
    .run(
        &spec,
        &spec.to_toml_string().unwrap(),
        RunInput {
            prompt: "echo hello".into(),
            ..Default::default()
        },
        &path,
        &model,
        &AutoApprove,
    )
    .await
    .unwrap();
    assert_eq!(result.status, wire::RunStatus::Succeeded);
    assert_eq!(tool.0.load(Ordering::SeqCst), 1);
    assert_eq!(model.reviews.load(Ordering::SeqCst), 2);
    assert!(JournalReader::read_all(&path).unwrap().iter().any(|e|matches!(&e.event,Some(wire::run_event::Event::BoundaryEvent(b)) if b.role=="verified_response" && b.visibility=="released")));
    std::fs::remove_file(path).unwrap();
}
