//! The inspection contract: native decoders emit [`Observation`]s, and only
//! [`Inspector`] turns them into an [`InspectionReport`].
//! [`CallInspector`](crate::CallInspector) drives decoding and inspection
//! for one call.
//!
//! Reports describe what the captured bytes show. They never claim that a tool
//! ran, that the client received the response, or that a subscription
//! allowance was charged.

use super::native::{OutputItemId, ToolCallId};
use super::text::WireText;
use crate::route::Endpoint;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// What the model reported about how its response ended.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ModelOutcome {
    Completed,
    /// Stopped early: output limit, context window, or a pause.
    Incomplete,
    Refused,
    Cancelled,
    Failed,
    /// Terminal evidence was present but not understood.
    Unknown,
}

impl ModelOutcome {
    /// Combine two pieces of terminal evidence. The more conservative outcome
    /// wins, so later evidence can never round a failure up to success:
    /// failed > cancelled > refused > incomplete > unknown > completed.
    pub fn merge(self, other: Self) -> Self {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::Unknown => 1,
            Self::Incomplete => 2,
            Self::Refused => 3,
            Self::Cancelled => 4,
            Self::Failed => 5,
        }
    }
}

/// How much of the response the report could interpret.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Coverage {
    Complete,
    Partial,
    Unavailable,
}

/// Why a report is partial or unavailable.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Limitation {
    UnsupportedFormat,
    UnsupportedEncoding,
    CaptureGap,
    BodyUnfinished,
    IncompleteEvent,
    MalformedEvent,
    MalformedField,
    EventTooLarge,
    DocumentTooLarge,
    DecodedTooLarge,
    DecodingError,
    TooManyToolCalls,
    TooManyUnknownKinds,
    NoTerminalEvent,
}

/// Where a body's format came from.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum FormatSource {
    Declared,
    /// Codex can omit `Content-Type` on `/responses`; SSE is assumed.
    Inferred,
}

/// Which native vocabulary an unknown value belonged to.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum KindFamily {
    Event,
    Document,
    ContentBlock,
    OutputItem,
    StopReason,
    Status,
    IncompleteReason,
}

/// How provider input-token totals treat cached tokens.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum InputAccounting {
    /// OpenAI: input totals include cached tokens.
    IncludesCache,
    /// Anthropic: cached tokens are reported separately.
    ExcludesCache,
}

/// Provider-reported token counts. Usage is a measurement of the response, not
/// of subscription allowance or cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub accounting: InputAccounting,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

impl Usage {
    pub fn new(accounting: InputAccounting) -> Self {
        Self {
            accounting,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cache_read_tokens.is_none()
            && self.cache_creation_tokens.is_none()
            && self.reasoning_tokens.is_none()
    }

    /// Native usage is cumulative: reported fields replace earlier values.
    fn update(&mut self, newer: Self) {
        for (target, value) in [
            (&mut self.input_tokens, newer.input_tokens),
            (&mut self.output_tokens, newer.output_tokens),
            (&mut self.cache_read_tokens, newer.cache_read_tokens),
            (&mut self.cache_creation_tokens, newer.cache_creation_tokens),
            (&mut self.reasoning_tokens, newer.reasoning_tokens),
        ] {
            if value.is_some() {
                *target = value;
            }
        }
    }
}

/// A tool call the model requested, as the wire names it. Requested is not
/// executed: Ren never sees the client run a tool. The activity reader pairs
/// one with its input ([`ToolUse`]).
///
/// [`ToolUse`]: crate::activity::ToolUse
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The identifier the client answers with.
    pub id: Option<ToolCallId>,
    /// The provider's output item identifier (OpenAI `id`), when distinct.
    pub item: Option<OutputItemId>,
    /// The native block or item type (`tool_use`, `function_call`, ...).
    pub kind: WireText,
    pub name: Option<WireText>,
}

impl ToolCall {
    /// A call known only by its kind and name, as a client reports a tool it
    /// ran rather than as the wire requested it. A name that is not wire
    /// text is left out.
    pub fn named(kind: WireText, name: &str) -> Self {
        Self {
            id: None,
            item: None,
            kind,
            name: WireText::new(name).ok(),
        }
    }

    fn same_call(&self, other: &Self) -> bool {
        fn same<T: PartialEq>(left: Option<&T>, right: Option<&T>) -> bool {
            left.zip(right).is_some_and(|(left, right)| left == right)
        }
        same(self.id.as_ref(), other.id.as_ref()) || same(self.item.as_ref(), other.item.as_ref())
    }

    /// Fill in what a later observation of the same call adds.
    fn absorb(&mut self, later: Self) {
        if self.id.is_none() {
            self.id = later.id;
        }
        if self.item.is_none() {
            self.item = later.item;
        }
        if later.name.is_some() {
            self.name = later.name;
        }
    }

    /// Its name, or its kind when the wire gave none: an OpenAI built-in
    /// tool (`web_search_call`, `local_shell_call`) is named by its type.
    pub fn name(&self) -> &str {
        self.name.as_ref().unwrap_or(&self.kind).as_str()
    }
}

/// An unknown native value, kept with how often it appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKind {
    pub family: KindFamily,
    pub kind: WireText,
    pub count: u64,
}

/// One fact read from a native payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    Model(WireText),
    /// The native stop or incomplete reason, verbatim.
    StopReason(WireText),
    Usage(Usage),
    ToolCall(ToolCall),
    Outcome(ModelOutcome),
    /// The protocol's own end-of-response marker, or a complete document.
    TerminalEvent,
    Unknown {
        family: KindFamily,
        kind: WireText,
    },
    /// A field was present but unusable; its siblings still count.
    MalformedField,
}

/// How a single event or document was classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    Understood,
    Unknown,
    Malformed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventCounts {
    pub understood: u64,
    pub unknown: u64,
    pub malformed: u64,
}

/// Bounds on what one report keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportLimits {
    pub max_tool_calls: usize,
    pub max_unknown_kinds: usize,
}

impl Default for ReportLimits {
    fn default() -> Self {
        Self {
            max_tool_calls: 128,
            max_unknown_kinds: 32,
        }
    }
}

/// The result of inspecting one response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionReport {
    coverage: Coverage,
    format: FormatSource,
    limitations: Vec<Limitation>,
    outcome: Option<ModelOutcome>,
    terminal_event: TerminalEvent,
    model: Option<WireText>,
    stop_reason: Option<WireText>,
    usage: Option<Usage>,
    tool_calls: Vec<ToolCall>,
    unknown_kinds: Vec<UnknownKind>,
    events: EventCounts,
}

/// Whether the protocol's end-of-response marker was seen.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum TerminalEvent {
    Seen,
    Missing,
}

impl InspectionReport {
    pub fn coverage(&self) -> Coverage {
        self.coverage
    }
    pub fn format(&self) -> FormatSource {
        self.format
    }
    pub fn limitations(&self) -> &[Limitation] {
        &self.limitations
    }
    pub fn outcome(&self) -> Option<ModelOutcome> {
        self.outcome
    }
    pub fn terminal_event(&self) -> TerminalEvent {
        self.terminal_event
    }
    pub fn model(&self) -> Option<&WireText> {
        self.model.as_ref()
    }
    pub fn stop_reason(&self) -> Option<&WireText> {
        self.stop_reason.as_ref()
    }
    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }
    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.tool_calls
    }
    pub fn unknown_kinds(&self) -> &[UnknownKind] {
        &self.unknown_kinds
    }
    pub fn events(&self) -> EventCounts {
        self.events
    }
}

/// Accumulates observations for one response.
#[derive(Debug)]
pub struct Inspector {
    limits: ReportLimits,
    report: InspectionReport,
}

impl Inspector {
    pub fn new(format: FormatSource, limits: ReportLimits) -> Self {
        Self {
            limits,
            report: InspectionReport {
                coverage: Coverage::Complete,
                format,
                limitations: Vec::new(),
                outcome: None,
                terminal_event: TerminalEvent::Missing,
                model: None,
                stop_reason: None,
                usage: None,
                tool_calls: Vec::new(),
                unknown_kinds: Vec::new(),
                events: EventCounts::default(),
            },
        }
    }

    /// A report for a body that cannot be interpreted at all.
    pub fn unavailable(format: FormatSource, reason: Limitation) -> InspectionReport {
        let mut inspector = Self::new(format, ReportLimits::default());
        inspector.report.coverage = Coverage::Unavailable;
        inspector.report.limitations.push(reason);
        inspector.report
    }

    pub fn limit(&mut self, reason: Limitation) {
        if !self.report.limitations.contains(&reason) {
            self.report.limitations.push(reason);
        }
        if self.report.coverage == Coverage::Complete {
            self.report.coverage = Coverage::Partial;
        }
    }

    pub fn count(&mut self, class: EventClass) {
        let counter = match class {
            EventClass::Understood => &mut self.report.events.understood,
            EventClass::Unknown => &mut self.report.events.unknown,
            EventClass::Malformed => &mut self.report.events.malformed,
        };
        *counter = counter.saturating_add(1);
        if class == EventClass::Malformed {
            self.limit(Limitation::MalformedEvent);
        }
    }

    pub fn observe(&mut self, observation: Observation) {
        match observation {
            Observation::Model(model) => self.report.model = Some(model),
            Observation::StopReason(reason) => self.report.stop_reason = Some(reason),
            Observation::Usage(usage) => self.usage(usage),
            Observation::ToolCall(call) => self.tool_call(call),
            Observation::Outcome(outcome) => {
                self.report.outcome = Some(
                    self.report
                        .outcome
                        .map_or(outcome, |current| current.merge(outcome)),
                );
            }
            Observation::TerminalEvent => self.report.terminal_event = TerminalEvent::Seen,
            Observation::Unknown { family, kind } => self.unknown(family, kind),
            Observation::MalformedField => self.limit(Limitation::MalformedField),
        }
    }

    fn usage(&mut self, usage: Usage) {
        if usage.is_empty() {
            return;
        }
        match &mut self.report.usage {
            Some(current) => current.update(usage),
            None => self.report.usage = Some(usage),
        }
    }

    fn tool_call(&mut self, call: ToolCall) {
        if call.id.is_none() && call.item.is_none() {
            self.limit(Limitation::MalformedField);
            return;
        }
        if let Some(existing) = self
            .report
            .tool_calls
            .iter_mut()
            .find(|existing| existing.same_call(&call))
        {
            existing.absorb(call);
        } else if self.report.tool_calls.len() >= self.limits.max_tool_calls {
            self.limit(Limitation::TooManyToolCalls);
        } else {
            self.report.tool_calls.push(call);
        }
    }

    fn unknown(&mut self, family: KindFamily, kind: WireText) {
        if let Some(existing) = self
            .report
            .unknown_kinds
            .iter_mut()
            .find(|existing| existing.family == family && existing.kind == kind)
        {
            existing.count = existing.count.saturating_add(1);
        } else if self.report.unknown_kinds.len() >= self.limits.max_unknown_kinds {
            self.limit(Limitation::TooManyUnknownKinds);
        } else {
            self.report.unknown_kinds.push(UnknownKind {
                family,
                kind,
                count: 1,
            });
        }
    }

    pub fn finish(mut self) -> InspectionReport {
        if self.report.terminal_event == TerminalEvent::Missing {
            self.limit(Limitation::NoTerminalEvent);
        }
        self.report
    }
}

/// The native protocol of an inspectable endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Anthropic Messages (`/v1/messages`).
    Messages,
    /// OpenAI Responses, public or Codex (`/responses`).
    Responses,
    /// OpenAI Chat Completions (`/v1/chat/completions`).
    ChatCompletions,
}

impl Protocol {
    /// The protocol an endpoint's responses use, if Ren inspects it.
    pub fn for_endpoint(endpoint: Endpoint) -> Option<Self> {
        match endpoint {
            Endpoint::Messages => Some(Self::Messages),
            Endpoint::Responses => Some(Self::Responses),
            Endpoint::ChatCompletions => Some(Self::ChatCompletions),
            Endpoint::CountTokens | Endpoint::Models | Endpoint::ResponsesCompact => None,
        }
    }
}

/// How the HTTP body ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEnd {
    /// End of body observed (EOF or complete framing).
    Complete,
    /// The body was cut off: disconnect, cancellation, or error.
    Unfinished,
}

#[cfg(test)]
mod tests {
    use super::{
        Coverage, FormatSource, InputAccounting, Inspector, KindFamily, Limitation, ModelOutcome,
        Observation, ReportLimits, TerminalEvent, ToolCall, Usage,
    };
    use crate::{
        label::Labeled,
        native::{OutputItemId, ToolCallId},
        text::WireText,
    };

    fn text(value: &str) -> WireText {
        WireText::new(value).unwrap()
    }

    #[test]
    fn outcome_merge_is_conservative_and_order_independent() {
        for left in ModelOutcome::ALL.iter().copied() {
            for right in ModelOutcome::ALL.iter().copied() {
                assert_eq!(left.merge(right), right.merge(left));
            }
            assert_eq!(left.merge(ModelOutcome::Completed), left);
            assert_eq!(left.merge(ModelOutcome::Failed), ModelOutcome::Failed);
        }
        assert_eq!(
            ModelOutcome::Completed.merge(ModelOutcome::Unknown),
            ModelOutcome::Unknown
        );
        assert_eq!(
            ModelOutcome::Refused.merge(ModelOutcome::Incomplete),
            ModelOutcome::Refused
        );
    }

    #[test]
    fn usage_is_cumulative_and_empty_usage_is_ignored() {
        let mut inspector = Inspector::new(FormatSource::Declared, ReportLimits::default());
        inspector.observe(Observation::Usage(Usage::new(
            InputAccounting::ExcludesCache,
        )));
        for (input, output) in [(Some(5), Some(8)), (None, Some(3))] {
            inspector.observe(Observation::Usage(Usage {
                input_tokens: input,
                output_tokens: output,
                ..Usage::new(InputAccounting::ExcludesCache)
            }));
        }
        inspector.observe(Observation::TerminalEvent);
        let report = inspector.finish();
        let usage = report.usage().unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens),
            (Some(5), Some(3))
        );
        assert_eq!(report.coverage(), Coverage::Complete);
    }

    #[test]
    fn a_tool_call_without_a_name_is_named_by_its_kind() {
        let named = ToolCall::named(text("tool_use"), "Read");
        assert_eq!(named.name(), "Read");
        // Not wire text: left out, so the kind names it.
        let unnamed = ToolCall::named(text("web_search_call"), "bad\u{7}name");
        assert_eq!(unnamed.name, None);
        assert_eq!(unnamed.name(), "web_search_call");
    }

    #[test]
    fn tool_calls_merge_by_either_identifier_and_are_bounded() {
        let limits = ReportLimits {
            max_tool_calls: 2,
            max_unknown_kinds: 1,
        };
        let mut inspector = Inspector::new(FormatSource::Declared, limits);
        let call = |call_id: Option<&str>, item_id: Option<&str>, name: Option<&str>| {
            Observation::ToolCall(ToolCall {
                id: call_id.map(|id| id.parse().unwrap()),
                item: item_id.map(|id| id.parse().unwrap()),
                kind: text("function_call"),
                name: name.map(text),
            })
        };
        inspector.observe(call(None, Some("item_1"), None));
        inspector.observe(call(Some("call_1"), Some("item_1"), Some("lookup")));
        inspector.observe(call(Some("call_1"), None, None));
        inspector.observe(call(None, None, Some("nameless")));
        inspector.observe(call(Some("call_2"), None, None));
        inspector.observe(call(Some("call_3"), None, None));
        for kind in ["future_a", "future_a", "future_b"] {
            inspector.observe(Observation::Unknown {
                family: KindFamily::Event,
                kind: text(kind),
            });
        }
        let report = inspector.finish();
        assert_eq!(report.tool_calls().len(), 2);
        let first = &report.tool_calls()[0];
        assert_eq!(first.id.as_ref().map(ToolCallId::as_str), Some("call_1"));
        assert_eq!(
            first.item.as_ref().map(OutputItemId::as_str),
            Some("item_1")
        );
        assert_eq!(first.name.as_ref().map(WireText::as_str), Some("lookup"));
        assert_eq!(report.unknown_kinds().len(), 1);
        assert_eq!(report.unknown_kinds()[0].count, 2);
        assert_eq!(
            report.limitations(),
            [
                Limitation::MalformedField,
                Limitation::TooManyToolCalls,
                Limitation::TooManyUnknownKinds,
                Limitation::NoTerminalEvent,
            ]
        );
        assert_eq!(report.terminal_event(), TerminalEvent::Missing);
        assert_eq!(report.coverage(), Coverage::Partial);
    }

    #[test]
    fn unavailable_reports_carry_their_reason_only() {
        let report =
            Inspector::unavailable(FormatSource::Declared, Limitation::UnsupportedEncoding);
        assert_eq!(report.coverage(), Coverage::Unavailable);
        assert_eq!(report.limitations(), [Limitation::UnsupportedEncoding]);
        assert_eq!(report.outcome(), None);
    }
}
