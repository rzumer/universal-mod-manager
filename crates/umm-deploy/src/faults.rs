use crate::DeployError;

/// Counts the filesystem steps of an apply or revert and can simulate a crash before a given
/// step. A simulated crash returns immediately with no cleanup, as a real crash would, so tests
/// can check that recovery restores the game folder from every intermediate state.
#[derive(Debug, Default)]
pub struct Faults {
    crash_at: Option<usize>,
    step: usize,
}

impl Faults {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn crash_at(step: usize) -> Self {
        Self {
            crash_at: Some(step),
            step: 0,
        }
    }

    /// Number of steps reached so far.
    pub fn steps(&self) -> usize {
        self.step
    }

    pub(crate) fn tick(&mut self) -> Result<(), DeployError> {
        let s = self.step;
        self.step += 1;
        if self.crash_at == Some(s) {
            Err(DeployError::InjectedCrash(s))
        } else {
            Ok(())
        }
    }
}
