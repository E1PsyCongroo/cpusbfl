use libafl::prelude::*;
use libafl_bolts::rands::Rand;

use crate::similarity::*;

#[derive(Debug, Clone)]
pub(crate) struct WitScheduler {
    initial_id: Option<CorpusId>,
    initial_seed_rate: f64,
    cover_weight: f64,
}

impl WitScheduler {
    pub(crate) fn new(
        initial_seed_rate: f64,
        cover_weight: f64,
        initial_id: Option<CorpusId>,
    ) -> Self {
        Self {
            initial_id,
            initial_seed_rate,
            cover_weight,
        }
    }
}

impl<I, S> RemovableScheduler<I, S> for WitScheduler {}

impl<I, S> Scheduler<I, S> for WitScheduler
where
    S: HasCorpus<I> + HasRand,
{
    fn on_add(&mut self, state: &mut S, id: CorpusId) -> Result<(), Error> {
        let current_id = *state.corpus().current();
        state
            .corpus()
            .get(id)?
            .borrow_mut()
            .set_parent_id_optional(current_id);

        if self.initial_id.is_none() {
            self.initial_id = Some(id);
        }

        Ok(())
    }

    fn next(&mut self, state: &mut S) -> Result<CorpusId, Error> {
        let initial_id = self
            .initial_id
            .ok_or_else(|| Error::empty("WitScheduler has no initial corpus entry".to_string()))?;
        let passed_cases = ranked_passed_cases(state, initial_id, self.cover_weight)?;

        let choose_initial =
            passed_cases.is_empty() || state.rand_mut().next_float() < self.initial_seed_rate;
        let next_id = if choose_initial {
            initial_id
        } else {
            let total_fitness = passed_cases.iter().map(|case| case.fitness).sum::<f64>();
            if !total_fitness.is_finite() || total_fitness <= 0.0 {
                passed_cases[state.rand_mut().below_or_zero(passed_cases.len())].id
            } else {
                let mut ticket = state.rand_mut().next_float() * total_fitness;
                let mut selected = passed_cases.last().expect("passed_cases is not empty").id;
                for case in &passed_cases {
                    if ticket < case.fitness {
                        selected = case.id;
                        break;
                    }
                    ticket -= case.fitness;
                }
                selected
            }
        };

        Self::set_current_scheduled(self, state, Some(next_id))?;
        Ok(next_id)
    }

    fn set_current_scheduled(
        &mut self,
        state: &mut S,
        next_id: Option<CorpusId>,
    ) -> Result<(), Error> {
        *state.corpus_mut().current_mut() = next_id;
        Ok(())
    }
}
