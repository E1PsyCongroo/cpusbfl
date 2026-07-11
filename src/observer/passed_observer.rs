use std::borrow::Cow;

use libafl::{executors::ExitKind, observers::Observer, prelude::Error};
use libafl_bolts::Named;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct PassedObserver {
    name: Cow<'static, str>,
    passed: Option<bool>,
}

impl PassedObserver {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            passed: None,
        }
    }
}

impl Named for PassedObserver {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for PassedObserver {
    fn pre_exec(&mut self, _state: &mut S, _input: &I) -> Result<(), Error> {
        self.passed = None;
        Ok(())
    }

    fn post_exec(&mut self, _state: &mut S, _input: &I, exit_kind: &ExitKind) -> Result<(), Error> {
        self.passed = Some(matches!(exit_kind, ExitKind::Ok));
        Ok(())
    }
}
