// crates/mash-ai/tests/deepseek.rs
use mash_ai::openai::{OpenAiBackend, OpenAiConfig};
use mash_ai::anthropic::{AnthropicBackend, AnthropicConfig};
use mash_ai::{LlmClient, LlmRequest, Message, MessageContent, Role};

fn openai_backend() -> OpenAiBackend {
    OpenAiBackend::new(OpenAiConfig {
        base_url: "https://api.deepseek.com".to_string(),
        api_key: std::env::var("DEEPSEEK_API_KEY")
            .unwrap_or_else(|_| "sk-test".to_string()),
    })
}

fn anthropic_backend() -> AnthropicBackend {
    AnthropicBackend::new(AnthropicConfig {
        base_url: "https://api.deepseek.com/anthropic".to_string(),
        api_key: std::env::var("DEEPSEEK_API_KEY")
            .unwrap_or_else(|_| "sk-test".to_string()),
    })
}

fn simple_request(model: &str) -> LlmRequest {
    LlmRequest {
        model: model.to_string(),
        system: "You are a helpful assistant. Reply briefly.".to_string(),
        max_tokens: 128,
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("Say hello in one sentence.".to_string()),
        }],
        tools: vec![],
    }
}

#[tokio::test]
#[ignore] // requires network + API key
async fn openai_complete_deepseek_chat() {
    let backend = openai_backend();
    let request = simple_request("deepseek-chat");
    let response = backend.complete(&request).await.unwrap();
    assert!(!response.content.is_empty());
    println!("OpenAI complete (deepseek-chat): {:?}", response);
}

#[tokio::test]
#[ignore]
async fn openai_complete_deepseek_reasoner() {
    let backend = openai_backend();
    let request = simple_request("deepseek-reasoner");
    let response = backend.complete(&request).await.unwrap();
    assert!(!response.content.is_empty());
    println!("OpenAI complete (deepseek-reasoner): {:?}", response);
}

#[tokio::test]
#[ignore]
async fn openai_stream_deepseek_chat() {
    use tokio_stream::StreamExt;
    let backend = openai_backend();
    let request = simple_request("deepseek-chat");
    let stream = backend.stream(&request).await.unwrap();
    let mut pinned = std::pin::pin!(stream);
    let mut got_text = false;
    let mut got_done = false;
    while let Some(event) = pinned.next().await {
        match event {
            mash_ai::StreamEvent::TextDelta(t) => {
                print!("{}", t);
                got_text = true;
            }
            mash_ai::StreamEvent::Done { .. } => {
                got_done = true;
                break;
            }
            mash_ai::StreamEvent::Error(e) => panic!("Stream error: {}", e),
            _ => {}
        }
    }
    println!();
    assert!(got_text);
    assert!(got_done);
}

#[tokio::test]
#[ignore]
async fn anthropic_complete_deepseek_chat() {
    let backend = anthropic_backend();
    let request = simple_request("deepseek-chat");
    let response = backend.complete(&request).await.unwrap();
    assert!(!response.content.is_empty());
    println!("Anthropic complete (deepseek-chat): {:?}", response);
}

#[tokio::test]
#[ignore]
async fn anthropic_stream_deepseek_chat() {
    use tokio_stream::StreamExt;
    let backend = anthropic_backend();
    let request = simple_request("deepseek-chat");
    let stream = backend.stream(&request).await.unwrap();
    let mut pinned = std::pin::pin!(stream);
    let mut got_text = false;
    while let Some(event) = pinned.next().await {
        match event {
            mash_ai::StreamEvent::TextDelta(t) => {
                print!("{}", t);
                got_text = true;
            }
            mash_ai::StreamEvent::Done { .. } => break,
            mash_ai::StreamEvent::Error(e) => panic!("Stream error: {}", e),
            _ => {}
        }
    }
    println!();
    assert!(got_text);
}

#[tokio::test]
#[ignore]
async fn anthropic_complete_deepseek_reasoner() {
    let backend = anthropic_backend();
    let request = simple_request("deepseek-reasoner");
    let response = backend.complete(&request).await.unwrap();
    assert!(!response.content.is_empty());
    println!("Anthropic complete (deepseek-reasoner): {:?}", response);
}

#[tokio::test]
#[ignore]
async fn anthropic_stream_deepseek_reasoner() {
    use tokio_stream::StreamExt;
    let backend = anthropic_backend();
    let request = simple_request("deepseek-reasoner");
    let stream = backend.stream(&request).await.unwrap();
    let mut pinned = std::pin::pin!(stream);
    let mut got_text = false;
    while let Some(event) = pinned.next().await {
        match event {
            mash_ai::StreamEvent::TextDelta(t) => {
                print!("{}", t);
                got_text = true;
            }
            mash_ai::StreamEvent::Done { .. } => break,
            mash_ai::StreamEvent::Error(e) => panic!("Stream error: {}", e),
            _ => {}
        }
    }
    println!();
    assert!(got_text);
}

#[tokio::test]
#[ignore]
async fn openai_stream_deepseek_reasoner() {
    use tokio_stream::StreamExt;
    let backend = openai_backend();
    let request = simple_request("deepseek-reasoner");
    let stream = backend.stream(&request).await.unwrap();
    let mut pinned = std::pin::pin!(stream);
    let mut got_text = false;
    let mut got_done = false;
    while let Some(event) = pinned.next().await {
        match event {
            mash_ai::StreamEvent::TextDelta(t) => {
                print!("{}", t);
                got_text = true;
            }
            mash_ai::StreamEvent::Done { .. } => {
                got_done = true;
                break;
            }
            mash_ai::StreamEvent::Error(e) => panic!("Stream error: {}", e),
            _ => {}
        }
    }
    println!();
    assert!(got_text);
    assert!(got_done);
}
