use crate::actors::Message;

/// Trait for actors that maintain internal state and can transition between states
pub trait StateSystem {
    type State: Clone + std::fmt::Debug + PartialEq;
    
    /// Get the current state of the actor
    fn current_state(&self) -> &Self::State;
    
    /// Attempt to transition state based on a message
    /// Returns Some(new_state) if transition occurred, None if no change
    fn transition(&mut self, message: &Message) -> Option<Self::State>;
}

#[cfg(test)]
pub mod test_utils {
    use super::*;
    
    /// Test utility to assert a state transition occurs
    pub fn assert_state_transition<T: StateSystem>(
        actor: &mut T,
        message: Message,
        expected_state: T::State,
    ) {
        let old_state = actor.current_state().clone();
        let transition_result = actor.transition(&message);
        
        assert_eq!(
            actor.current_state(),
            &expected_state,
            "Expected state transition from {:?} to {:?}, but actor is in state {:?}",
            old_state,
            expected_state,
            actor.current_state()
        );
        
        assert_eq!(
            transition_result,
            Some(expected_state.clone()),
            "transition() should return Some({:?}) when state changes",
            expected_state
        );
    }
    
    /// Test utility to assert no state transition occurs
    pub fn assert_no_state_transition<T: StateSystem>(
        actor: &mut T,
        message: Message,
    ) {
        let old_state = actor.current_state().clone();
        let transition_result = actor.transition(&message);
        
        assert_eq!(
            actor.current_state(),
            &old_state,
            "Expected no state change, but state changed from {:?} to {:?}",
            old_state,
            actor.current_state()
        );
        
        assert_eq!(
            transition_result,
            None,
            "transition() should return None when no state change occurs"
        );
    }
}