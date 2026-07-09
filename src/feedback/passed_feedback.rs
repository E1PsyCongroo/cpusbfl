use std::{borrow::Cow, fmt::Debug};

use libafl::{
    Error, HasMetadata, HasNamedMetadata,
    corpus::Testcase,
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
};
use libafl_bolts::{Named, tuples::MatchName};
use serde::{Deserialize, Serialize};

pub const PASSEDFEEDBACK_PREFIX: &str = "passedfeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassedMetadata {
    pub is_passed: bool,
}

libafl_bolts::impl_serdeany!(PassedMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PassedFeedback {
    name: Cow<'static, str>,
    pending: Option<PassedMetadata>,
}

impl PassedFeedback {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: Cow::from(PASSEDFEEDBACK_PREFIX.to_string()),
            pending: None,
        }
    }
}

impl Named for PassedFeedback {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for PassedFeedback
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, _state: &mut S) -> Result<(), Error> {
        Ok(())
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for PassedFeedback
where
    OT: MatchName,
    S: HasNamedMetadata,
{
    fn is_interesting(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _input: &I,
        _observers: &OT,
        exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        self.pending = Some(PassedMetadata {
            is_passed: matches!(exit_kind, ExitKind::Ok),
        });

        Ok(true)
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _observers: &OT,
        testcase: &mut Testcase<I>,
    ) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::illegal_state(
            "PassedFeedback append_metadata called without pending metadata",
        ))?;

        testcase.add_metadata(pending);
        Ok(())
    }
}
