use bindings::{
    exports::wasmind::actor::actor::MessageEnvelope,
    wasmind::actor::{http, logger},
};
use serde::Deserialize;
use wasmind_actor_utils::common_messages::{
    assistant::{
        ChatState, ChatStateUpdated, CompactedConversation, QueueStatusChange,
        Response as AssistantResponse, Status, WaitReason,
    },
    litellm::BaseUrlUpdate,
};
use wasmind_actor_utils::llm_client_types::{
    ChatMessage, ChatMessageWithRequestId, ChatRequest, ChatResponse,
};

#[allow(warnings)]
mod bindings;

const SYSTEM_PROMPT: &str = r#"You are a specialized AI assistant named "Context Distiller." Your sole purpose is to analyze a conversation transcript and produce a concise, structured summary of its current state. This summary's goal is to "hydrate" a new AI instance, allowing it to seamlessly continue the task without the full, token-heavy conversation history.

You must be ruthlessly efficient and objective. Your output is not for a human to read for pleasure, but for another AI to use as a functional starting point.

**INPUT FORMAT:**
You will receive a complete conversation transcript formatted with the following tags: `<system>`, `<user>`, `<assistant>`, and `<tool>`.

**YOUR TASK:**
Synthesize the provided transcript into a structured "State Summary." This summary must capture only the essential information needed for a new AI to pick up the work exactly where the previous one left off.

---

**CRITICAL INSTRUCTIONS - WHAT TO EXTRACT:**

1.  **Overall Goal:** Identify the user's primary, high-level objective. What are they ultimately trying to accomplish?
2.  **Current Focus:** What is the specific, immediate sub-task being worked on right now?
3.  **Key Facts & Decisions:** List the critical pieces of information that have been established, confirmed, or decided upon. This includes user preferences, constraints (like budget or deadlines), and important facts discovered through tool use.
4.  **Tools Utilized:** List the *names* of the tools that have been successfully used. Do NOT include the full output of the tools. You can, however, include a one-sentence summary of a critical tool result if it's part of a "Key Fact" (e.g., "flight_search tool confirmed a flight is available for $500").
5.  **Next Step or Open Question:** What was the very last thing the user asked? Or, what is the logical next action the AI needs to take to move the task forward?

**CRITICAL INSTRUCTIONS - WHAT TO IGNORE:**

*   **DO NOT** include conversational pleasantries (e.g., "Hello," "Thank you," "That sounds great").
*   **DO NOT** include the full, verbatim, raw output from any `<tool>` call (e.g., no long JSON blobs or lists).
*   **DO NOT** summarize every single turn. Synthesize the *cumulative* state.
*   **DO NOT** include failed attempts or dead-end paths, unless they established a critical constraint (e.g., "User confirmed they do not want options in the 'downtown' area").
*   **DO NOT** include the original `<system>` prompt in your summary. It will be passed to the new AI separately.

---

**OUTPUT FORMAT:**
You MUST structure your output using the following Markdown format.

## Current State Summary
**Overall Goal:** [A single sentence describing the user's main objective.]
**Current Focus:** [A single sentence describing the immediate task at hand.]
**Key Facts & Decisions Made:**
- [Fact 1]
- [Decision 2]
- [Constraint 3]
- [Key finding from a tool]
**Tools Utilized:**
- `tool_name_1`
- `tool_name_2`
**Next Step / Open Question:** [The immediate next action required or the last question posed by the user.]

---

**EXAMPLE:**

**INPUT TRANSCRIPT:**
<system>
You are a helpful travel planning assistant. You have access to `flight_search` and `hotel_search` tools.
</system>
<user>
Hey, can you help me plan a 5-day trip to San Francisco?
</user>
<assistant>
Of course! To get started, what's your approximate budget and when are you thinking of going?
</assistant>
<user>
Let's aim for the second week of October, and keep the budget for flights and hotel under $2000. I want to fly from NYC.
</user>
<assistant>
Okay, searching for flights from NYC to SFO for the second week of October.
<tool_code>
print(flight_search(origin="NYC", destination="SFO", date_range="2024-10-07-2024-10-14"))
</tool_code>
</assistant>
<tool>
{ "flights": [ { "airline": "United", "price": 450, "flight_number": "UA123" }, { "airline": "JetBlue", "price": 480, "flight_number": "JB456" } ] }
</tool>
<assistant>
I found a round-trip flight on United for $450. That leaves us with about $1550 for the hotel. Does that flight work for you?
</assistant>
<user>
Yes, the United flight for $450 is perfect. Let's lock that in as the plan. Now, can you find me a hotel? I'd like to be near Fisherman's Wharf.
</user>

**YOUR CORRECT OUTPUT:**

## Current State Summary
**Overall Goal:** Plan a 5-day trip to San Francisco from NYC.
**Current Focus:** Finding a hotel in San Francisco.
**Key Facts & Decisions Made:**
- Total budget for flight and hotel is $2000.
- Travel dates are the second week of October.
- A United flight for $450 has been identified and approved by the user.
- The remaining budget for the hotel is approximately $1550.
- The user wants the hotel to be located near Fisherman's Wharf.
**Tools Utilized:**
- `flight_search`
**Next Step / Open Question:** The AI needs to search for hotels near Fisherman's Wharf that fit within the remaining $1550 budget for 5 nights.
"#;

#[derive(Deserialize)]
struct ConversationCompactionConfig {
    token_threshold: i32,
    model_name: String,
}

wasmind_actor_utils::actors::macros::generate_actor_trait!();

#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct ConversationCompactionActor {
    scope: String,
    base_url: Option<String>,
    last_chat_state: Option<ChatState>,
    config: ConversationCompactionConfig,
}

impl GeneratedActorTrait for ConversationCompactionActor {
    fn new(scope: String, config_str: String) -> Self {
        let config: ConversationCompactionConfig =
            toml::from_str(&config_str).expect("Error parsing config");

        logger::log(
            logger::LogLevel::Info,
            &format!(
                "ConversationCompaction initialized with threshold: {} tokens, model: {}",
                config.token_threshold, config.model_name
            ),
        );

        Self {
            scope,
            base_url: None,
            last_chat_state: None,
            config,
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        // Handle LiteLLM base URL updates
        if let Some(base_url_update) = Self::parse_as::<BaseUrlUpdate>(&message) {
            self.base_url = Some(base_url_update.base_url);
            return;
        }

        // Only process messages from our scope
        if message.from_scope != self.scope {
            return;
        }

        // Track chat state updates
        if let Some(chat_state_updated) = Self::parse_as::<ChatStateUpdated>(&message) {
            self.last_chat_state = Some(chat_state_updated.chat_state);
            return;
        }

        // Monitor token usage from responses
        if let Some(response) = Self::parse_as::<AssistantResponse>(&message) {
            if response.usage.total_tokens >= self.config.token_threshold {
                self.trigger_compaction();
            }
        }
    }

    fn destructor(&mut self) {}
}

impl ConversationCompactionActor {
    fn trigger_compaction(&mut self) {
        // Check prerequisites
        let Some(base_url) = &self.base_url else {
            logger::log(
                logger::LogLevel::Warn,
                "Cannot compact: No LiteLLM base URL",
            );
            return;
        };

        let Some(chat_state) = &self.last_chat_state else {
            logger::log(
                logger::LogLevel::Warn,
                "Cannot compact: No chat state available",
            );
            return;
        };

        // Queue a status change for the assistant to wait
        let _ = Self::broadcast_common_message(QueueStatusChange {
            agent: self.scope.clone(),
            status: Status::Wait {
                reason: WaitReason::CompactingConversation,
            },
        });

        // Find the last assistant message to determine compaction boundary
        let last_assistant = chat_state.messages.iter()
            .enumerate()
            .rev()
            .find_map(|(idx, msg)| {
                if let ChatMessageWithRequestId::Assistant(assistant_msg) = msg {
                    Some((idx, assistant_msg.originating_request_id.clone()))
                } else {
                    None
                }
            });

        let (messages_to_compact, compacted_to_id) = if let Some((idx, request_id)) = last_assistant {
            if idx > 0 {
                // Compact everything BEFORE the last assistant message
                (&chat_state.messages[..idx], request_id)
            } else {
                // Last assistant is the first message, nothing to compact
                logger::log(
                    logger::LogLevel::Info,
                    "Last assistant message is first in history, skipping compaction",
                );
                return;
            }
        } else {
            // No assistant messages found, can't determine boundary
            logger::log(
                logger::LogLevel::Warn,
                "No assistant messages found, cannot determine compaction boundary",
            );
            return;
        };

        // Make the compaction request with trimmed messages
        match self.make_compaction_request(base_url, messages_to_compact) {
            Ok(compacted_summary) => {
                logger::log(
                    logger::LogLevel::Info,
                    &format!(
                        "Successfully compacted conversation. Summary length: {} chars, compacted to request: {}",
                        compacted_summary.len(),
                        compacted_to_id
                    ),
                );

                // Broadcast the compacted conversation
                let _ = Self::broadcast_common_message(CompactedConversation {
                    agent: self.scope.clone(),
                    messages: vec![ChatMessageWithRequestId::user(format!("Below is the current state from the last task you were executing before your history was compacted:\n\n<current_state_summary>{}</current_state_summary>\n\nContinue where you left off", compacted_summary))],
                    compacted_to: compacted_to_id,
                });

                logger::log(
                    logger::LogLevel::Info,
                    "Compaction complete, conversation reset",
                );
            }
            Err(e) => {
                logger::log(
                    logger::LogLevel::Error,
                    &format!("Failed to compact conversation: {}", e),
                );

                // On failure, still need to unblock the assistant
                let _ = Self::broadcast_common_message(QueueStatusChange {
                    agent: self.scope.clone(),
                    status: Status::Wait {
                        reason: WaitReason::WaitingForUserInput,
                    },
                });
            }
        }
    }

    fn make_compaction_request(
        &self,
        base_url: &str,
        messages: &[ChatMessageWithRequestId],
    ) -> Result<String, String> {
        let request = ChatRequest {
            model: self.config.model_name.clone(),
            messages: vec![ChatMessage::system(SYSTEM_PROMPT)],
            tools: None,
        };

        // Add the conversation to compact as a single user message
        let conversation_text = messages
            .iter()
            .map(|msg| match msg {
                ChatMessage::System(s) => format!("<system>{}</system>", s.content),
                ChatMessage::User(u) => format!("<user>{}</user>", u.content),
                ChatMessage::Assistant(a) => {
                    format!(
                        "<assistant>{}</assistant>",
                        a.message.content.as_deref().unwrap_or("")
                    )
                }
                ChatMessage::Tool(t) => format!(r#"<tool name"{}">{}</tool>"#, t.name, t.content),
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut final_request = request;
        final_request
            .messages
            .push(ChatMessage::user(&conversation_text));

        let body = serde_json::to_vec(&final_request)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        let http_request = http::Request::new("POST", &format!("{}/v1/chat/completions", base_url));

        let response = http_request
            .header("Content-Type", "application/json")
            .body(&body)
            .retry(3, 1000)
            .retry_on_status_codes(&[429, 500, 502, 503, 504])
            .timeout(60)
            .send()
            .map_err(|e| format!("HTTP request failed: {:?}", e))?;

        if response.status != 200 {
            return Err(format!(
                "LiteLLM returned status {}: {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            ));
        }

        let chat_response: ChatResponse = serde_json::from_slice(&response.body)
            .map_err(|e| format!("Failed to deserialize response: {}", e))?;

        if let Some(choice) = chat_response.choices.first() {
            if let ChatMessage::Assistant(assistant_msg) = &choice.message {
                if let Some(content) = &assistant_msg.content {
                    return Ok(content.clone());
                }
            }
        }

        Err("No valid response content from LLM".to_string())
    }
}
