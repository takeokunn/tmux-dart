use anyhow::{Context, Result};

use crate::{
    config::JumpConfig,
    jump::{bounded_subset_bounds, labels_for, positions_for},
    overlay::{draw_overlay, with_recovered_screen},
    tmux::{PaneState, TmuxBackend},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpState {
    ResolvePane,
    CaptureScreen,
    SelectTarget,
    JumpToTarget,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpReport {
    pub state: JumpState,
    pub transitions: Vec<JumpState>,
}

#[derive(Debug, Default)]
struct JumpMachine {
    transitions: Vec<JumpState>,
}

impl JumpMachine {
    fn enter(&mut self, state: JumpState) {
        self.transitions.push(state);
    }

    fn finish(mut self, state: JumpState) -> JumpReport {
        self.transitions.push(state.clone());
        JumpReport {
            state,
            transitions: self.transitions,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JumpRequest {
    pub initial_char: char,
    pub pane_id: Option<String>,
    pub config: JumpConfig,
}

pub fn run_jump<B: TmuxBackend>(backend: &B, request: JumpRequest) -> Result<JumpState> {
    Ok(run_jump_report(backend, request)?.state)
}

pub fn run_jump_report<B: TmuxBackend>(backend: &B, request: JumpRequest) -> Result<JumpReport> {
    let mut machine = JumpMachine::default();

    machine.enter(JumpState::ResolvePane);
    let pane_id = if let Some(pane_id) = request.pane_id {
        pane_id
    } else {
        backend.current_pane_id()?
    };
    let pane = backend.pane_state(&pane_id)?;
    backend.cancel_copy_mode(&pane)?;

    machine.enter(JumpState::CaptureScreen);
    let screen = backend.capture_visible_pane(&pane)?;
    let positions = positions_for(
        request.initial_char,
        &screen,
        request.config.options.match_mode,
        request.config.options.case_sensitive,
    );
    if positions.is_empty() {
        backend.display_message(&format!(
            "tmux-dart: no matches for '{}'",
            request.initial_char
        ))?;
        return Ok(machine.finish(JumpState::Cancelled));
    }
    if request.config.options.auto_jump && positions.len() == 1 {
        machine.enter(JumpState::JumpToTarget);
        backend.jump_to_position(&pane, positions[0])?;
        return Ok(machine.finish(JumpState::Done));
    }

    machine.enter(JumpState::SelectTarget);
    let selected_index = with_recovered_screen(backend, &pane, || {
        select_position_index(backend, &pane, &screen, &positions, &request.config)
    })?;
    let Some(selected_index) = selected_index else {
        backend.display_message("tmux-dart: jump cancelled")?;
        return Ok(machine.finish(JumpState::Cancelled));
    };

    machine.enter(JumpState::JumpToTarget);
    let target = positions
        .get(selected_index)
        .copied()
        .context("selected index out of bounds")?;
    backend.jump_to_position(&pane, target)?;
    Ok(machine.finish(JumpState::Done))
}

pub fn select_position_index<B: TmuxBackend>(
    backend: &B,
    pane: &PaneState,
    screen: &str,
    positions: &[usize],
    config: &JumpConfig,
) -> Result<Option<usize>> {
    if positions.is_empty() {
        return Ok(None);
    }
    if config.options.auto_jump && positions.len() == 1 {
        return Ok(Some(0));
    }

    let labels = labels_for(positions.len(), &config.options.label_keys);
    let label_len = labels[0].chars().count();
    draw_overlay(
        backend,
        pane,
        screen,
        positions,
        &labels[..positions.len()],
        &config.overlay_style,
    )?;
    let Some(input) = backend.prompt_for_label_char("char:")? else {
        return Ok(None);
    };

    let Some(key_index) = config
        .options
        .label_keys
        .iter()
        .position(|key| *key == input)
    else {
        return Ok(None);
    };

    if label_len == 1 {
        return Ok((key_index < positions.len()).then_some(key_index));
    }

    let Some(subset) = bounded_subset_bounds(
        key_index,
        label_len,
        positions.len(),
        config.options.label_keys.len(),
    ) else {
        return Ok(None);
    };
    let remaining = &positions[subset.clone()];
    let lower_index = select_position_index(backend, pane, screen, remaining, config)?;
    Ok(lower_index.map(|value| subset.start + value))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use anyhow::Result;

    use super::{JumpRequest, JumpState, run_jump_report};
    use crate::{
        config::{EnvValues, JumpConfig},
        tmux::{PaneState, TmuxBackend},
    };

    #[derive(Default)]
    struct FakeBackend {
        screen: String,
        prompts: RefCell<VecDeque<Option<char>>>,
        writes: RefCell<Vec<String>>,
        jumped_to: RefCell<Option<usize>>,
        messages: RefCell<Vec<String>>,
    }

    impl FakeBackend {
        fn with_prompts(prompts: impl IntoIterator<Item = char>) -> Self {
            Self {
                screen: String::from("qone qtwo qthree qfour"),
                prompts: RefCell::new(prompts.into_iter().map(Some).collect()),
                writes: RefCell::default(),
                jumped_to: RefCell::default(),
                messages: RefCell::default(),
            }
        }

        fn with_prompt_results(prompts: impl IntoIterator<Item = Option<char>>) -> Self {
            Self {
                screen: String::from("qone qtwo qthree qfour"),
                prompts: RefCell::new(prompts.into_iter().collect()),
                writes: RefCell::default(),
                jumped_to: RefCell::default(),
                messages: RefCell::default(),
            }
        }

        fn with_screen(mut self, screen: &str) -> Self {
            self.screen = screen.to_owned();
            self
        }
    }

    impl TmuxBackend for FakeBackend {
        fn current_pane_id(&self) -> Result<String> {
            Ok(String::from("%0"))
        }

        fn pane_state(&self, pane_id: &str) -> Result<PaneState> {
            Ok(PaneState {
                pane_id: pane_id.to_owned(),
                tty_path: String::from("/dev/null"),
                in_copy_mode: false,
                cursor_y: 0,
                cursor_x: 0,
                alternate_on: false,
                scroll_position: 0,
                pane_height: 24,
            })
        }

        fn cancel_copy_mode(&self, _pane: &PaneState) -> Result<()> {
            Ok(())
        }

        fn capture_visible_pane(&self, _pane: &PaneState) -> Result<String> {
            Ok(self.screen.clone())
        }

        fn capture_pane_with_escapes(&self, _pane_id: &str) -> Result<String> {
            Ok(String::new())
        }

        fn write_to_tty(&self, _pane: &PaneState, content: &str) -> Result<()> {
            self.writes.borrow_mut().push(content.to_owned());
            Ok(())
        }

        fn prompt_for_label_char(&self, _prompt: &str) -> Result<Option<char>> {
            Ok(self.prompts.borrow_mut().pop_front().flatten())
        }

        fn display_message(&self, message: &str) -> Result<()> {
            self.messages.borrow_mut().push(message.to_owned());
            Ok(())
        }

        fn jump_to_position(&self, _pane: &PaneState, jump_to: usize) -> Result<()> {
            *self.jumped_to.borrow_mut() = Some(jump_to);
            Ok(())
        }
    }

    #[test]
    fn jump_flow_selects_second_target_from_label_prompt() -> Result<()> {
        let backend = FakeBackend::with_prompts(['f']);
        let config = JumpConfig::from_values(EnvValues::default());
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: 'q',
                pane_id: None,
                config,
            },
        )?;

        assert_eq!(report.state, JumpState::Done);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::SelectTarget,
                JumpState::JumpToTarget,
                JumpState::Done,
            ]
        );
        assert_eq!(*backend.jumped_to.borrow(), Some(5));
        assert_eq!(backend.writes.borrow().len(), 3);
        assert!(
            backend
                .writes
                .borrow()
                .iter()
                .any(|write| write.contains('f'))
        );
        assert!(backend.messages.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn jump_flow_auto_jumps_single_target_without_overlay_or_message() -> Result<()> {
        let backend = FakeBackend::default().with_screen("only");
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: 'o',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Done);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::JumpToTarget,
                JumpState::Done,
            ]
        );
        assert_eq!(*backend.jumped_to.borrow(), Some(0));
        assert!(backend.writes.borrow().is_empty());
        assert!(backend.messages.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn jump_flow_auto_jumps_to_non_word_target_in_default_word_mode() -> Result<()> {
        // Regression guard for the reported bug: a non-word target such as '/'
        // must be jumpable in the default word mode, not reported as "no match".
        let backend = FakeBackend::default().with_screen("a/b");
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: '/',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Done);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::JumpToTarget,
                JumpState::Done,
            ]
        );
        assert_eq!(*backend.jumped_to.borrow(), Some(1));
        assert!(backend.writes.borrow().is_empty());
        assert!(backend.messages.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn jump_flow_selects_non_word_target_through_overlay() -> Result<()> {
        // Many-match non-word target must drive the overlay/label path, not just
        // the single-match auto-jump path.
        let backend = FakeBackend::with_prompts(['f']).with_screen("a/b/c");
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: '/',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Done);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::SelectTarget,
                JumpState::JumpToTarget,
                JumpState::Done,
            ]
        );
        // Slashes sit at indices 1 and 3; label 'f' is the second label key.
        assert_eq!(*backend.jumped_to.borrow(), Some(3));
        assert!(!backend.writes.borrow().is_empty());
        assert!(backend.messages.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn jump_flow_cancels_when_label_prompt_is_invalid() -> Result<()> {
        let backend = FakeBackend::with_prompts(['x']);
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: 'q',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Cancelled);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::SelectTarget,
                JumpState::Cancelled,
            ]
        );
        assert_eq!(*backend.jumped_to.borrow(), None);
        assert_eq!(backend.writes.borrow().len(), 3);
        assert_eq!(
            backend.messages.borrow().as_slice(),
            ["tmux-dart: jump cancelled"]
        );
        Ok(())
    }

    #[test]
    fn jump_flow_emits_cancellation_message_when_label_prompt_is_dismissed() -> Result<()> {
        let backend = FakeBackend::with_prompt_results([None]);
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: 'q',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Cancelled);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::SelectTarget,
                JumpState::Cancelled,
            ]
        );
        assert_eq!(*backend.jumped_to.borrow(), None);
        assert_eq!(backend.writes.borrow().len(), 3);
        assert_eq!(
            backend.messages.borrow().as_slice(),
            ["tmux-dart: jump cancelled"]
        );
        Ok(())
    }

    #[test]
    fn jump_flow_emits_no_match_message_when_no_targets_exist() -> Result<()> {
        let backend = FakeBackend::default().with_screen("only");
        let report = run_jump_report(
            &backend,
            JumpRequest {
                initial_char: 'z',
                pane_id: None,
                config: JumpConfig::from_values(EnvValues::default()),
            },
        )?;

        assert_eq!(report.state, JumpState::Cancelled);
        assert_eq!(
            report.transitions,
            vec![
                JumpState::ResolvePane,
                JumpState::CaptureScreen,
                JumpState::Cancelled,
            ]
        );
        assert!(backend.writes.borrow().is_empty());
        assert_eq!(*backend.jumped_to.borrow(), None);
        assert_eq!(
            backend.messages.borrow().as_slice(),
            ["tmux-dart: no matches for 'z'"]
        );
        Ok(())
    }
}
