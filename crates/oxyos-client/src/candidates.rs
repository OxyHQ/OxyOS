//! The candidates an input method hands the system (ADR 0041), as the session's shell receives
//! them: each candidate's text and the auxiliary text in chunks, collected into one list that the
//! closing `InputCandidates` completes; and the shell's requests on the window that shows them.

use crate::Client;
use oxyos_protocol::{
    Event, Request, Size, MAX_CANDIDATES_AUX_BYTES, MAX_CANDIDATE_BYTES, MAX_INPUT_CANDIDATES,
    NO_CANDIDATE,
};
use std::io;

/// One page of candidates to draw at the caret, in the order the input method gave them (the
/// shell numbers them 1 to 9, then 0). No candidates: hide the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidates {
    pub candidates: Vec<String>,
    /// A reading (pinyin's "ni'hao"), a mode; empty for none.
    pub aux: String,
    /// The highlighted candidate's index, if any.
    pub highlighted: Option<u8>,
    /// CANDIDATES_PREVIOUS, CANDIDATES_NEXT, CANDIDATES_VERTICAL bits.
    pub flags: u8,
}

#[derive(Debug, Default)]
pub(crate) struct CandidateLists {
    /// The list being received, and the one the last event completed.
    building: Candidates,
    pub(crate) done: Option<Candidates>,
}

/// `text` with `chunk` added, unless that would pass `limit` bytes (a compositor never sends
/// more; one that did is cut at the last whole chunk).
fn extend(text: &mut String, chunk: &str, limit: usize) {
    if text.len() + chunk.len() <= limit {
        text.push_str(chunk);
    }
}

impl Client {
    /// The candidates the event last returned completed (an InputCandidates), if any. Dropped
    /// with the next event.
    pub fn candidates(&self) -> Option<&Candidates> {
        self.candidate_lists.done.as_ref()
    }

    /// The session's shell (ADR 0041): its candidates surface, one per connection, which the
    /// compositor places at the caret of the field an input method serves while it hands this
    /// shell a list, and hides otherwise. It never takes focus.
    pub fn create_input_candidates(&self, surface: u32, size: Size) -> io::Result<()> {
        self.send(&Request::CreateInputCandidates { surface, size })
    }

    /// Its candidates surface at a new size: what it showed stays until a frame of that size.
    /// Protocol version 21: refused (`Unsupported`) with an older compositor, where the surface is
    /// made anew instead.
    pub fn resize_input_candidates(&self, surface: u32, size: Size) -> io::Result<()> {
        self.send(&Request::ResizeInputCandidates { surface, size })
    }

    /// The candidate of the list it draws now at `index` pressed: the input method chooses it.
    pub fn choose_input_candidate(&self, index: u8) -> io::Result<()> {
        self.send(&Request::ChooseInputCandidate { index })
    }

    /// A chevron pressed: the input method turns to the page after (`forward`) or before.
    pub fn turn_input_candidates_page(&self, forward: bool) -> io::Result<()> {
        self.send(&Request::TurnInputCandidatesPage { forward })
    }

    /// Collects a list's parts; its InputCandidates completes it.
    pub(crate) fn observe_candidates(&mut self, event: &Event) {
        let lists = &mut self.candidate_lists;
        lists.done = None;
        match *event {
            Event::InputCandidate { index, text } => {
                let index = usize::from(index);
                let candidates = &mut lists.building.candidates;
                // One repeated adds its next chunk; the next begins; a gap is filled with empty
                // candidates, so the indices stay the input method's.
                if index >= MAX_INPUT_CANDIDATES {
                    return;
                }
                if index >= candidates.len() {
                    candidates.resize(index + 1, String::new());
                }
                extend(&mut candidates[index], text.as_str(), MAX_CANDIDATE_BYTES);
            }
            Event::InputCandidatesAux { text } => {
                extend(
                    &mut lists.building.aux,
                    text.as_str(),
                    MAX_CANDIDATES_AUX_BYTES,
                );
            }
            Event::InputCandidates {
                count,
                highlighted,
                flags,
            } => {
                let mut list = std::mem::take(&mut lists.building);
                list.candidates.resize(usize::from(count), String::new());
                list.highlighted =
                    (highlighted != NO_CANDIDATE && highlighted < count).then_some(highlighted);
                list.flags = flags;
                if count == 0 {
                    list.aux.clear();
                }
                lists.done = Some(list);
            }
            _ => {}
        }
    }
}
