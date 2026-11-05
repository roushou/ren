//! The stored form of an inspection report.
//!
//! Reports are versioned: [`InspectionRecord::VERSION`] is stored beside each
//! one, so a later binary can tell which inspector produced it and reinspect.
//! Labels are read back as [`Stored`], keeping values a newer binary wrote.

use serde::{Deserialize, Serialize};
use tgv_protocol::{
    inspect::{
        Coverage, FormatSource, InputAccounting, InspectionReport, KindFamily, Limitation,
        ModelOutcome, TerminalEvent, ToolCall, UnknownKind, Usage,
    },
    label::Stored,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectionRecord {
    pub coverage: Stored<Coverage>,
    pub format: Stored<FormatSource>,
    pub limitations: Vec<Stored<Limitation>>,
    pub outcome: Option<Stored<ModelOutcome>>,
    pub terminal_event: Stored<TerminalEvent>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<UsageRecord>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub unknown_kinds: Vec<UnknownKindRecord>,
    pub events: EventCountsRecord,
}

impl InspectionRecord {
    /// The inspector version that produced records of this shape.
    pub const VERSION: u32 = 1;

    /// Tool names, for search columns.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tool_calls
            .iter()
            .filter_map(|call| call.name.as_deref())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub accounting: Stored<InputAccounting>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub call_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownKindRecord {
    pub family: Stored<KindFamily>,
    pub kind: String,
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCountsRecord {
    pub understood: u64,
    pub unknown: u64,
    pub malformed: u64,
}

/// Lossless: wire text becomes owned text, labels stay labels.
impl From<&InspectionReport> for InspectionRecord {
    fn from(report: &InspectionReport) -> Self {
        let events = report.events();
        Self {
            coverage: report.coverage().into(),
            format: report.format().into(),
            limitations: report
                .limitations()
                .iter()
                .copied()
                .map(Stored::from)
                .collect(),
            outcome: report.outcome().map(Stored::from),
            terminal_event: report.terminal_event().into(),
            model: report.model().map(|text| text.as_str().to_owned()),
            stop_reason: report.stop_reason().map(|text| text.as_str().to_owned()),
            usage: report.usage().map(UsageRecord::from),
            tool_calls: report
                .tool_calls()
                .iter()
                .map(ToolCallRecord::from)
                .collect(),
            unknown_kinds: report
                .unknown_kinds()
                .iter()
                .map(UnknownKindRecord::from)
                .collect(),
            events: EventCountsRecord {
                understood: events.understood,
                unknown: events.unknown,
                malformed: events.malformed,
            },
        }
    }
}

impl From<&Usage> for UsageRecord {
    fn from(usage: &Usage) -> Self {
        Self {
            accounting: usage.accounting.into(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }
    }
}

impl From<&ToolCall> for ToolCallRecord {
    fn from(call: &ToolCall) -> Self {
        let text = |value: &Option<tgv_protocol::text::WireText>| {
            value.as_ref().map(|text| text.as_str().to_owned())
        };
        Self {
            call_id: text(&call.call_id),
            item_id: text(&call.item_id),
            kind: call.kind.as_str().to_owned(),
            name: text(&call.name),
        }
    }
}

impl From<&UnknownKind> for UnknownKindRecord {
    fn from(kind: &UnknownKind) -> Self {
        Self {
            family: kind.family.into(),
            kind: kind.kind.as_str().to_owned(),
            count: kind.count,
        }
    }
}
