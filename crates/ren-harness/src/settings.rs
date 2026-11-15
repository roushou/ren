//! How a run is set up beyond its folder and client: the model, how hard it
//! reasons, and what it may do without asking. Each is optional: none leaves
//! the choice to the client's own configuration.

use crate::Harness;
use crate::drive::CommandRefused;
use crate::models::ModelId;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How much a model reasons before it answers, from none to the most. Each
/// model offers some of these; Pi names them thinking levels.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub enum Effort {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// What a driven client may do without asking the operator.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ApprovalPolicy {
    /// Edits and commands wait for the operator.
    Ask,
    /// Edits in the folder go ahead; commands wait.
    Edits,
    /// The client decides what is safe and asks only beyond that: Claude
    /// Code's auto mode, Codex inside its workspace sandbox.
    Auto,
}

impl ApprovalPolicy {
    /// The policies `harness` can be run under. Pi asks for nothing, so it
    /// offers none; OpenCode allows everything unless told to ask, so it
    /// offers asking first and letting edits go ahead, as permission rules.
    pub fn offered(harness: Harness) -> &'static [Self] {
        harness.profile().approvals
    }
}

/// What a running conversation can take mid-run, by client.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Midway {
    /// A message added to the turn that is running.
    Steer,
    /// Another model from the next turn.
    Model,
    /// Another effort from the next turn.
    Effort,
    /// Another approval policy from the next turn.
    Approvals,
}

impl Midway {
    /// What `harness` takes mid-run: Codex sets each turn's model, effort,
    /// and approvals and takes steering; Claude Code switches model and
    /// permission mode; Pi and OpenCode switch model and thinking level and
    /// take steering.
    pub fn offered(harness: Harness) -> &'static [Self] {
        harness.profile().midway
    }
}

/// A run's settings; each one left out is the client's own.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunSettings {
    pub model: Option<ModelId>,
    pub effort: Option<Effort>,
    pub approvals: Option<ApprovalPolicy>,
}

/// One setting of a run, at its new value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    Model(Option<ModelId>),
    Effort(Option<Effort>),
    Approvals(Option<ApprovalPolicy>),
}

impl Setting {
    /// What a client must take mid-run to change it.
    pub fn midway(&self) -> Midway {
        match self {
            Self::Model(_) => Midway::Model,
            Self::Effort(_) => Midway::Effort,
            Self::Approvals(_) => Midway::Approvals,
        }
    }
}

/// The settings that differ between a run's settings and the ones asked
/// for, in the order model, effort, approvals. Each is one a client takes
/// mid-run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct SettingsChange(Vec<Setting>);

impl SettingsChange {
    pub fn settings(&self) -> &[Setting] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for SettingsChange {
    type Item = Setting;
    type IntoIter = std::vec::IntoIter<Setting>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl RunSettings {
    /// What moving to `next` changes, when a client that takes `offered`
    /// mid-run can change all of it. A setting left as it is needs nothing
    /// offered.
    pub fn change_to(
        &self,
        next: &RunSettings,
        offered: &[Midway],
    ) -> Result<SettingsChange, CommandRefused> {
        let mut changed = Vec::new();
        if next.model != self.model {
            changed.push(Setting::Model(next.model.clone()));
        }
        if next.effort != self.effort {
            changed.push(Setting::Effort(next.effort));
        }
        if next.approvals != self.approvals {
            changed.push(Setting::Approvals(next.approvals));
        }
        match changed
            .iter()
            .map(Setting::midway)
            .find(|midway| !offered.contains(midway))
        {
            Some(midway) => Err(CommandRefused::NotOffered(midway)),
            None => Ok(SettingsChange(changed)),
        }
    }

    /// These settings with `setting` at its new value.
    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::Model(model) => self.model = model,
            Setting::Effort(effort) => self.effort = effort,
            Setting::Approvals(approvals) => self.approvals = approvals,
        }
    }

    /// `setting` at the value these settings have.
    fn current(&self, setting: &Setting) -> Setting {
        match setting {
            Setting::Model(_) => Setting::Model(self.model.clone()),
            Setting::Effort(_) => Setting::Effort(self.effort),
            Setting::Approvals(_) => Setting::Approvals(self.approvals),
        }
    }
}

/// A driven client's settings: the ones it acknowledged, and the ones Ren
/// asked for since. A change is diffed against what was asked, so asking
/// again sends nothing new; a setting is committed only when the client
/// acknowledges it, and one it refuses goes back to what it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsBook {
    acknowledged: RunSettings,
    asked: RunSettings,
}

impl SettingsBook {
    /// A client running on `settings`.
    pub(crate) fn new(settings: RunSettings) -> Self {
        Self {
            acknowledged: settings.clone(),
            asked: settings,
        }
    }

    /// What the client runs on, as far as it acknowledged.
    #[cfg(test)]
    pub(crate) fn acknowledged(&self) -> &RunSettings {
        &self.acknowledged
    }

    /// What moving to `next` changes, against what was last asked for.
    pub(crate) fn change_to(
        &self,
        next: &RunSettings,
        offered: &[Midway],
    ) -> Result<SettingsChange, CommandRefused> {
        self.asked.change_to(next, offered)
    }

    /// `setting` was asked for.
    pub(crate) fn ask(&mut self, setting: Setting) {
        self.asked.apply(setting);
    }

    /// The client took `setting`.
    pub(crate) fn acknowledge(&mut self, setting: Setting) {
        self.acknowledged.apply(setting);
    }

    /// The client refused `setting`: it keeps what it had, and asking for
    /// it again sends it again.
    pub(crate) fn refused(&mut self, setting: &Setting) {
        let kept = self.acknowledged.current(setting);
        self.asked.apply(kept);
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalPolicy, Effort, Midway, RunSettings, Setting, SettingsBook};
    use crate::drive::CommandRefused;
    use strum::VariantArray;

    fn settings(model: Option<&str>, effort: Option<Effort>) -> RunSettings {
        RunSettings {
            model: model.map(|model| model.parse().unwrap()),
            effort,
            approvals: None,
        }
    }

    #[test]
    fn a_change_names_what_differs_and_needs_only_that_offered() {
        let now = settings(Some("big"), None);
        let next = settings(Some("small"), Some(Effort::High));
        let change = now
            .change_to(&next, &[Midway::Model, Midway::Effort])
            .unwrap();
        assert_eq!(
            change.settings(),
            [
                Setting::Model(Some("small".parse().unwrap())),
                Setting::Effort(Some(Effort::High))
            ]
        );
        assert_eq!(
            now.change_to(&next, &[Midway::Model]),
            Err(CommandRefused::NotOffered(Midway::Effort))
        );
        // An unchanged setting needs nothing offered.
        let same = RunSettings {
            approvals: Some(ApprovalPolicy::Ask),
            ..now.clone()
        };
        assert!(same.change_to(&same, &[]).unwrap().is_empty());
    }

    #[test]
    fn a_setting_is_committed_only_when_acknowledged() {
        let mut book = SettingsBook::new(settings(None, None));
        let next = settings(Some("small"), Some(Effort::Low));
        for setting in book.change_to(&next, Midway::VARIANTS).unwrap() {
            book.ask(setting);
        }
        // Asked again: nothing new to send.
        assert!(book.change_to(&next, Midway::VARIANTS).unwrap().is_empty());
        assert_eq!(book.acknowledged(), &settings(None, None));
        book.acknowledge(Setting::Model(Some("small".parse().unwrap())));
        book.refused(&Setting::Effort(Some(Effort::Low)));
        assert_eq!(book.acknowledged(), &settings(Some("small"), None));
        // The refused one is sent again when asked for again.
        assert_eq!(
            book.change_to(&next, Midway::VARIANTS).unwrap().settings(),
            [Setting::Effort(Some(Effort::Low))]
        );
    }
}
