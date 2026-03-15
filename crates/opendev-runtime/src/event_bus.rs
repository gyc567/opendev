//! Typed event bus for decoupled inter-component communication.
//!
//! Components publish typed [`RuntimeEvent`] variants; subscribers receive
//! copies asynchronously. Supports topic-based filtering so each subscriber
//! only receives events it is interested in.
//!
//! Events are broadcast via `tokio::sync::broadcast`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use tracing::debug;

use crate::session_status::SessionStatus;

/// Maximum number of events buffered per channel.
const DEFAULT_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// Event topic — used for subscriber interest filtering (#94)
// ---------------------------------------------------------------------------

/// Identifies the category (topic) of a [`RuntimeEvent`].
///
/// Subscribers declare which topics they care about; the bus only delivers
/// matching events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventTopic {
    /// Tool execution lifecycle events.
    Tool,
    /// LLM request / response events.
    Llm,
    /// Agent lifecycle events (start, stop, error).
    Agent,
    /// Session lifecycle events.
    Session,
    /// Cost / token usage events.
    Cost,
    /// System-level events (config reload, shutdown).
    System,
    /// Custom / user-defined events.
    Custom,
}

// ---------------------------------------------------------------------------
// RuntimeEvent — typed event variants (#93)
// ---------------------------------------------------------------------------

/// A strongly-typed event published on the bus.
///
/// Each variant carries only the data relevant to that event kind, replacing
/// the previous stringly-typed `Event` struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeEvent {
    // -- Tool events --
    /// A tool call is about to start.
    ToolCallStart {
        tool_name: String,
        call_id: String,
        timestamp_ms: u64,
    },
    /// A tool call completed.
    ToolCallEnd {
        tool_name: String,
        call_id: String,
        duration_ms: u64,
        success: bool,
        timestamp_ms: u64,
    },

    // -- LLM events --
    /// An LLM request was sent.
    LlmRequestStart {
        model: String,
        request_id: String,
        timestamp_ms: u64,
    },
    /// An LLM response was received.
    LlmResponseEnd {
        model: String,
        request_id: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
        timestamp_ms: u64,
    },

    // -- Agent events --
    /// An agent started working.
    AgentStart {
        agent_id: String,
        task: String,
        timestamp_ms: u64,
    },
    /// An agent finished.
    AgentEnd {
        agent_id: String,
        success: bool,
        timestamp_ms: u64,
    },
    /// An agent encountered an error.
    AgentError {
        agent_id: String,
        error: String,
        timestamp_ms: u64,
    },

    // -- Session events --
    /// Session started.
    SessionStart {
        session_id: String,
        timestamp_ms: u64,
    },
    /// Session ended.
    SessionEnd {
        session_id: String,
        timestamp_ms: u64,
    },
    /// Session status changed (idle → busy → retry → idle).
    SessionStatusChanged {
        session_id: String,
        status: SessionStatus,
        timestamp_ms: u64,
    },

    // -- Cost events --
    /// Token usage was recorded.
    TokenUsage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        timestamp_ms: u64,
    },

    // -- Cost events --
    /// Session cost budget has been exhausted.
    ///
    /// Published when [`CostTracker::is_over_budget`] returns `true` after
    /// recording token usage. The agent loop should pause and notify the user.
    BudgetExhausted {
        budget_usd: f64,
        total_cost_usd: f64,
        timestamp_ms: u64,
    },

    // -- System events --
    /// Configuration was reloaded.
    ConfigReloaded { timestamp_ms: u64 },
    /// Graceful shutdown requested.
    ShutdownRequested { reason: String, timestamp_ms: u64 },

    // -- Custom --
    /// Escape hatch for events not covered by the typed variants.
    Custom {
        event_type: String,
        source: String,
        data: Value,
        timestamp_ms: u64,
    },
}

impl RuntimeEvent {
    /// Return the [`EventTopic`] for this event.
    pub fn topic(&self) -> EventTopic {
        match self {
            Self::ToolCallStart { .. } | Self::ToolCallEnd { .. } => EventTopic::Tool,
            Self::LlmRequestStart { .. } | Self::LlmResponseEnd { .. } => EventTopic::Llm,
            Self::AgentStart { .. } | Self::AgentEnd { .. } | Self::AgentError { .. } => {
                EventTopic::Agent
            }
            Self::SessionStart { .. }
            | Self::SessionEnd { .. }
            | Self::SessionStatusChanged { .. } => EventTopic::Session,
            Self::TokenUsage { .. } | Self::BudgetExhausted { .. } => EventTopic::Cost,
            Self::ConfigReloaded { .. } | Self::ShutdownRequested { .. } => EventTopic::System,
            Self::Custom { .. } => EventTopic::Custom,
        }
    }

    /// Return the timestamp in milliseconds since epoch.
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            Self::ToolCallStart { timestamp_ms, .. }
            | Self::ToolCallEnd { timestamp_ms, .. }
            | Self::LlmRequestStart { timestamp_ms, .. }
            | Self::LlmResponseEnd { timestamp_ms, .. }
            | Self::AgentStart { timestamp_ms, .. }
            | Self::AgentEnd { timestamp_ms, .. }
            | Self::AgentError { timestamp_ms, .. }
            | Self::SessionStart { timestamp_ms, .. }
            | Self::SessionEnd { timestamp_ms, .. }
            | Self::SessionStatusChanged { timestamp_ms, .. }
            | Self::TokenUsage { timestamp_ms, .. }
            | Self::BudgetExhausted { timestamp_ms, .. }
            | Self::ConfigReloaded { timestamp_ms, .. }
            | Self::ShutdownRequested { timestamp_ms, .. }
            | Self::Custom { timestamp_ms, .. } => *timestamp_ms,
        }
    }
}

/// Helper: current time as milliseconds since UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Legacy Event — kept for backward compatibility
// ---------------------------------------------------------------------------

/// A legacy untyped event (kept for backward compatibility).
///
/// New code should prefer [`RuntimeEvent`] variants.
#[derive(Debug, Clone)]
pub struct Event {
    /// Event type identifier (e.g., "tool_call_start", "llm_response").
    pub event_type: String,
    /// Component that published the event.
    pub source: String,
    /// Event payload.
    pub data: Value,
    /// Timestamp (milliseconds since epoch).
    pub timestamp_ms: u64,
}

impl Event {
    /// Create a new event.
    pub fn new(event_type: impl Into<String>, source: impl Into<String>, data: Value) -> Self {
        Self {
            event_type: event_type.into(),
            source: source.into(),
            data,
            timestamp_ms: now_ms(),
        }
    }

    /// Convert a legacy `Event` into a [`RuntimeEvent::Custom`].
    pub fn into_runtime_event(self) -> RuntimeEvent {
        RuntimeEvent::Custom {
            event_type: self.event_type,
            source: self.source,
            data: self.data,
            timestamp_ms: self.timestamp_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// EventBus — typed publish / subscribe (#93 + #94)
// ---------------------------------------------------------------------------

/// Typed event bus for broadcasting [`RuntimeEvent`] instances.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    sender: broadcast::Sender<RuntimeEvent>,
    _capacity: usize,
}

impl EventBus {
    /// Create a new event bus with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a new event bus with a specific capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            inner: Arc::new(EventBusInner {
                sender,
                _capacity: capacity,
            }),
        }
    }

    /// Publish a typed event to all subscribers.
    pub fn publish(&self, event: RuntimeEvent) {
        let topic = event.topic();
        match self.inner.sender.send(event) {
            Ok(n) => debug!("Event {:?} sent to {} subscribers", topic, n),
            Err(_) => debug!("Event {:?} published with no subscribers", topic),
        }
    }

    /// Convenience: publish a legacy `Event` by converting it to `RuntimeEvent::Custom`.
    pub fn emit(&self, event_type: &str, source: &str, data: Value) {
        let event = Event::new(event_type, source, data);
        self.publish(event.into_runtime_event());
    }

    /// Subscribe to *all* events (unfiltered).
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.inner.sender.subscribe()
    }

    /// Subscribe with topic-based filtering (#94).
    ///
    /// The returned [`TopicSubscriber`] only yields events whose topic is in
    /// the given set.
    pub fn subscribe_topics(&self, topics: HashSet<EventTopic>) -> TopicSubscriber {
        TopicSubscriber {
            receiver: self.inner.sender.subscribe(),
            topics,
        }
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.inner.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TopicSubscriber — topic-based filtering (#94)
// ---------------------------------------------------------------------------

/// A subscriber that only receives events matching its declared topics.
pub struct TopicSubscriber {
    receiver: broadcast::Receiver<RuntimeEvent>,
    topics: HashSet<EventTopic>,
}

impl TopicSubscriber {
    /// Receive the next event matching the subscriber's topics.
    pub async fn recv(&mut self) -> Option<RuntimeEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if self.topics.contains(&event.topic()) {
                        return Some(event);
                    }
                    // Not interested — skip.
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!("TopicSubscriber lagged, missed {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Return the set of topics this subscriber is interested in.
    pub fn topics(&self) -> &HashSet<EventTopic> {
        &self.topics
    }
}

// ---------------------------------------------------------------------------
// FilteredSubscriber — legacy string-based filtering (backward compat)
// ---------------------------------------------------------------------------

/// Filtered event subscriber — only receives events matching a filter.
///
/// Works with the legacy `event_type` string inside `RuntimeEvent::Custom`.
pub struct FilteredSubscriber {
    receiver: broadcast::Receiver<RuntimeEvent>,
    event_types: Option<Vec<String>>,
}

impl FilteredSubscriber {
    /// Create a filtered subscriber.
    pub fn new(bus: &EventBus, event_types: Option<Vec<String>>) -> Self {
        Self {
            receiver: bus.subscribe(),
            event_types,
        }
    }

    /// Receive the next matching event (returns a legacy `Event`).
    pub async fn recv(&mut self) -> Option<Event> {
        loop {
            match self.receiver.recv().await {
                Ok(runtime_event) => {
                    // Convert RuntimeEvent to legacy Event for compat.
                    let legacy = match &runtime_event {
                        RuntimeEvent::Custom {
                            event_type,
                            source,
                            data,
                            timestamp_ms,
                        } => Event {
                            event_type: event_type.clone(),
                            source: source.clone(),
                            data: data.clone(),
                            timestamp_ms: *timestamp_ms,
                        },
                        other => Event {
                            event_type: format!("{:?}", other.topic()),
                            source: String::new(),
                            data: serde_json::to_value(other).unwrap_or(Value::Null),
                            timestamp_ms: other.timestamp_ms(),
                        },
                    };

                    if let Some(ref types) = self.event_types
                        && !types.iter().any(|t| t == &legacy.event_type)
                    {
                        continue;
                    }
                    return Some(legacy);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!("Subscriber lagged, missed {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Collect events into a map grouped by event type (useful for metrics).
pub fn group_events_by_type(events: &[Event]) -> HashMap<String, Vec<&Event>> {
    let mut groups: HashMap<String, Vec<&Event>> = HashMap::new();
    for event in events {
        groups
            .entry(event.event_type.clone())
            .or_default()
            .push(event);
    }
    groups
}

/// Group [`RuntimeEvent`]s by their [`EventTopic`].
pub fn group_runtime_events_by_topic(
    events: &[RuntimeEvent],
) -> HashMap<EventTopic, Vec<&RuntimeEvent>> {
    let mut groups: HashMap<EventTopic, Vec<&RuntimeEvent>> = HashMap::new();
    for event in events {
        groups.entry(event.topic()).or_default().push(event);
    }
    groups
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let event = Event::new("test", "component", serde_json::json!({"key": "value"}));
        assert_eq!(event.event_type, "test");
        assert_eq!(event.source, "component");
        assert!(event.timestamp_ms > 0);
    }

    #[test]
    fn test_bus_creation() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn test_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.emit("test_event", "test", serde_json::json!({"count": 1}));

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, RuntimeEvent::Custom { .. }));
        if let RuntimeEvent::Custom {
            event_type, data, ..
        } = event
        {
            assert_eq!(event_type, "test_event");
            assert_eq!(data["count"], 1);
        }
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 2);

        bus.emit("event", "src", serde_json::json!(null));

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert!(matches!(e1, RuntimeEvent::Custom { .. }));
        assert!(matches!(e2, RuntimeEvent::Custom { .. }));
    }

    #[tokio::test]
    async fn test_filtered_subscriber() {
        let bus = EventBus::new();
        let mut sub = FilteredSubscriber::new(&bus, Some(vec!["wanted".to_string()]));

        bus.emit("unwanted", "src", serde_json::json!(null));
        bus.emit("wanted", "src", serde_json::json!({"ok": true}));

        let event = sub.recv().await.unwrap();
        assert_eq!(event.event_type, "wanted");
    }

    #[test]
    fn test_no_subscribers() {
        let bus = EventBus::new();
        // Should not panic
        bus.emit("event", "src", serde_json::json!(null));
    }

    #[test]
    fn test_group_events_by_type() {
        let events = vec![
            Event::new("a", "src", serde_json::json!(null)),
            Event::new("b", "src", serde_json::json!(null)),
            Event::new("a", "src", serde_json::json!(null)),
        ];
        let groups = group_events_by_type(&events);
        assert_eq!(groups["a"].len(), 2);
        assert_eq!(groups["b"].len(), 1);
    }

    #[test]
    fn test_bus_clone() {
        let bus1 = EventBus::new();
        let _rx = bus1.subscribe();
        let bus2 = bus1.clone();
        assert_eq!(bus2.subscriber_count(), 1);
    }

    #[test]
    fn test_debug_format() {
        let bus = EventBus::new();
        let debug_str = format!("{:?}", bus);
        assert!(debug_str.contains("EventBus"));
    }

    // -- New typed event tests --

    #[test]
    fn test_runtime_event_topic() {
        let ev = RuntimeEvent::ToolCallStart {
            tool_name: "bash".into(),
            call_id: "1".into(),
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Tool);

        let ev = RuntimeEvent::LlmRequestStart {
            model: "gpt-4".into(),
            request_id: "r1".into(),
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Llm);

        let ev = RuntimeEvent::AgentStart {
            agent_id: "a1".into(),
            task: "test".into(),
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Agent);

        let ev = RuntimeEvent::SessionStart {
            session_id: "s1".into(),
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Session);

        let ev = RuntimeEvent::TokenUsage {
            model: "gpt-4".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Cost);

        let ev = RuntimeEvent::ConfigReloaded {
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::System);
    }

    #[tokio::test]
    async fn test_typed_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let ev = RuntimeEvent::ToolCallStart {
            tool_name: "bash".into(),
            call_id: "c1".into(),
            timestamp_ms: now_ms(),
        };
        bus.publish(ev);

        let received = rx.recv().await.unwrap();
        assert!(matches!(received, RuntimeEvent::ToolCallStart { .. }));
        if let RuntimeEvent::ToolCallStart { tool_name, .. } = received {
            assert_eq!(tool_name, "bash");
        }
    }

    #[tokio::test]
    async fn test_topic_subscriber_filters() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe_topics(HashSet::from([EventTopic::Tool]));

        // Publish an LLM event (should be filtered out)
        bus.publish(RuntimeEvent::LlmRequestStart {
            model: "gpt-4".into(),
            request_id: "r1".into(),
            timestamp_ms: now_ms(),
        });

        // Publish a Tool event (should be received)
        bus.publish(RuntimeEvent::ToolCallStart {
            tool_name: "bash".into(),
            call_id: "c1".into(),
            timestamp_ms: now_ms(),
        });

        let received = sub.recv().await.unwrap();
        assert_eq!(received.topic(), EventTopic::Tool);
    }

    #[tokio::test]
    async fn test_topic_subscriber_multiple_topics() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe_topics(HashSet::from([EventTopic::Tool, EventTopic::Session]));

        bus.publish(RuntimeEvent::LlmRequestStart {
            model: "m".into(),
            request_id: "r".into(),
            timestamp_ms: now_ms(),
        });
        bus.publish(RuntimeEvent::SessionStart {
            session_id: "s1".into(),
            timestamp_ms: now_ms(),
        });

        let received = sub.recv().await.unwrap();
        assert_eq!(received.topic(), EventTopic::Session);
    }

    #[test]
    fn test_legacy_event_into_runtime_event() {
        let legacy = Event::new("test", "comp", serde_json::json!(42));
        let rt = legacy.into_runtime_event();
        assert_eq!(rt.topic(), EventTopic::Custom);
        if let RuntimeEvent::Custom {
            event_type, data, ..
        } = rt
        {
            assert_eq!(event_type, "test");
            assert_eq!(data, serde_json::json!(42));
        } else {
            panic!("expected Custom variant");
        }
    }

    #[test]
    fn test_group_runtime_events_by_topic() {
        let events = vec![
            RuntimeEvent::ToolCallStart {
                tool_name: "a".into(),
                call_id: "1".into(),
                timestamp_ms: 0,
            },
            RuntimeEvent::LlmRequestStart {
                model: "m".into(),
                request_id: "r".into(),
                timestamp_ms: 0,
            },
            RuntimeEvent::ToolCallEnd {
                tool_name: "a".into(),
                call_id: "1".into(),
                duration_ms: 100,
                success: true,
                timestamp_ms: 0,
            },
        ];
        let groups = group_runtime_events_by_topic(&events);
        assert_eq!(groups[&EventTopic::Tool].len(), 2);
        assert_eq!(groups[&EventTopic::Llm].len(), 1);
    }

    #[test]
    fn test_topic_subscriber_topics_accessor() {
        let bus = EventBus::new();
        let topics = HashSet::from([EventTopic::Agent, EventTopic::Cost]);
        let sub = bus.subscribe_topics(topics.clone());
        assert_eq!(*sub.topics(), topics);
    }

    #[test]
    fn test_event_topic_serialization() {
        let topic = EventTopic::Tool;
        let json = serde_json::to_string(&topic).unwrap();
        let deserialized: EventTopic = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, topic);
    }

    #[test]
    fn test_runtime_event_serialization() {
        let event = RuntimeEvent::ToolCallStart {
            tool_name: "bash".into(),
            call_id: "c1".into(),
            timestamp_ms: 12345,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.topic(), EventTopic::Tool);
        assert_eq!(deserialized.timestamp_ms(), 12345);
    }

    #[test]
    fn test_now_ms_is_positive() {
        assert!(now_ms() > 0);
    }

    #[test]
    fn test_budget_exhausted_event_topic() {
        let ev = RuntimeEvent::BudgetExhausted {
            budget_usd: 1.0,
            total_cost_usd: 1.05,
            timestamp_ms: now_ms(),
        };
        assert_eq!(ev.topic(), EventTopic::Cost);
        assert!(ev.timestamp_ms() > 0);
    }

    #[test]
    fn test_budget_exhausted_serialization() {
        let event = RuntimeEvent::BudgetExhausted {
            budget_usd: 2.50,
            total_cost_usd: 2.75,
            timestamp_ms: 99999,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.topic(), EventTopic::Cost);
        assert_eq!(deserialized.timestamp_ms(), 99999);
        if let RuntimeEvent::BudgetExhausted {
            budget_usd,
            total_cost_usd,
            ..
        } = deserialized
        {
            assert!((budget_usd - 2.50).abs() < 1e-10);
            assert!((total_cost_usd - 2.75).abs() < 1e-10);
        } else {
            panic!("expected BudgetExhausted variant");
        }
    }

    #[tokio::test]
    async fn test_budget_exhausted_received_by_cost_subscriber() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe_topics(HashSet::from([EventTopic::Cost]));

        bus.publish(RuntimeEvent::BudgetExhausted {
            budget_usd: 1.0,
            total_cost_usd: 1.5,
            timestamp_ms: now_ms(),
        });

        let received = sub.recv().await.unwrap();
        assert!(matches!(received, RuntimeEvent::BudgetExhausted { .. }));
    }
}
