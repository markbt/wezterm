use crate::escape::{Action, DeviceControlMode, Esc, OperatingSystemCommand, CSI};
use log::error;
use num;
use std::cell::RefCell;
use vte;

/// The `Parser` struct holds the state machine that is used to decode
/// a sequence of bytes.  The byte sequence can be streaming into the
/// state machine.
/// You can either have the parser trigger a callback as `Action`s are
/// decoded, or have it return a `Vec<Action>` holding zero-or-more
/// decoded actions.
pub struct Parser {
    state_machine: vte::Parser,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state_machine: vte::Parser::new(),
        }
    }

    pub fn parse<F: FnMut(Action)>(&mut self, bytes: &[u8], mut callback: F) {
        let mut perform = Performer {
            callback: &mut callback,
        };
        for b in bytes {
            self.state_machine.advance(&mut perform, *b);
        }
    }

    /// A specialized version of the parser that halts after recognizing the
    /// first action from the stream of bytes.  The return value is the action
    /// that was recognized and the length of the byte stream that was fed in
    /// to the parser to yield it.
    pub fn parse_first(&mut self, bytes: &[u8]) -> Option<(Action, usize)> {
        // holds the first action.  We need to use RefCell to deal with
        // the Performer holding a reference to this via the closure we set up.
        let first = RefCell::new(None);
        // will hold the iterator index when we emit an action
        let mut first_idx = None;
        {
            let mut perform = Performer {
                callback: &mut |action| {
                    // capture the action, but only if it is the first one
                    // we've seen.  Preserve an existing one if any.
                    if first.borrow().is_some() {
                        return;
                    }
                    *first.borrow_mut() = Some(action);
                },
            };
            for (idx, b) in bytes.iter().enumerate() {
                self.state_machine.advance(&mut perform, *b);
                if first.borrow().is_some() {
                    // if we recognized an action, record the iterator index
                    first_idx = Some(idx);
                    break;
                }
            }
        }

        match (first.into_inner(), first_idx) {
            // if we matched an action, transform the iterator index to
            // the length of the string that was consumed (+1)
            (Some(action), Some(idx)) => Some((action, idx + 1)),
            _ => None,
        }
    }

    pub fn parse_as_vec(&mut self, bytes: &[u8]) -> Vec<Action> {
        let mut result = Vec::new();
        self.parse(bytes, |action| result.push(action));
        result
    }

    /// Similar to `parse_first` but collects all actions from the first sequence.
    pub fn parse_first_as_vec(&mut self, bytes: &[u8]) -> Option<(Vec<Action>, usize)> {
        let mut actions = Vec::new();
        let mut first_idx = None;
        for (idx, b) in bytes.iter().enumerate() {
            self.state_machine.advance(
                &mut Performer {
                    callback: &mut |action| actions.push(action),
                },
                *b,
            );
            if !actions.is_empty() {
                // if we recognized any actions, record the iterator index
                first_idx = Some(idx);
                break;
            }
        }
        first_idx.map(|idx| (actions, idx + 1))
    }
}

struct Performer<'a, F: FnMut(Action) + 'a> {
    callback: &'a mut F,
}

impl<'a, F: FnMut(Action)> vte::Perform for Performer<'a, F> {
    fn print(&mut self, c: char) {
        (self.callback)(Action::Print(c));
    }

    fn execute(&mut self, byte: u8) {
        match num::FromPrimitive::from_u8(byte) {
            Some(code) => (self.callback)(Action::Control(code)),
            None => error!("impossible C0/C1 control code {:?} was dropped", byte),
        }
    }

    fn hook(&mut self, params: &[i64], intermediates: &[u8], ignored_extra_intermediates: bool) {
        (self.callback)(Action::DeviceControl(Box::new(DeviceControlMode::Enter {
            params: params.to_vec(),
            intermediates: intermediates.to_vec(),
            ignored_extra_intermediates,
        })));
    }

    fn put(&mut self, data: u8) {
        (self.callback)(Action::DeviceControl(Box::new(DeviceControlMode::Data(
            data,
        ))));
    }

    fn unhook(&mut self) {
        (self.callback)(Action::DeviceControl(Box::new(DeviceControlMode::Exit)));
    }

    fn osc_dispatch(&mut self, osc: &[&[u8]]) {
        let osc = OperatingSystemCommand::parse(osc);
        (self.callback)(Action::OperatingSystemCommand(Box::new(osc)));
    }

    fn csi_dispatch(
        &mut self,
        params: &[i64],
        intermediates: &[u8],
        ignored_extra_intermediates: bool,
        control: char,
    ) {
        for action in CSI::parse(params, intermediates, ignored_extra_intermediates, control) {
            (self.callback)(Action::CSI(action));
        }
    }

    fn esc_dispatch(
        &mut self,
        _params: &[i64],
        intermediates: &[u8],
        _ignored_extra_intermediates: bool,
        control: u8,
    ) {
        // It doesn't appear to be possible for params.len() > 1 due to the way
        // that the state machine in vte functions.  As such, it also seems to
        // be impossible for ignored_extra_intermediates to be true too.
        (self.callback)(Action::Esc(Esc::parse(
            if intermediates.len() == 1 {
                Some(intermediates[0])
            } else {
                None
            },
            control,
        )));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::cell::Intensity;
    use crate::escape::csi::Sgr;
    use crate::escape::EscCode;
    use std::io::Write;

    fn encode(seq: &Vec<Action>) -> String {
        let mut res = Vec::new();
        for s in seq {
            write!(res, "{}", s).unwrap();
        }
        String::from_utf8(res).unwrap()
    }

    #[test]
    fn basic_parse() {
        let mut p = Parser::new();
        let actions = p.parse_as_vec(b"hello");
        assert_eq!(
            vec![
                Action::Print('h'),
                Action::Print('e'),
                Action::Print('l'),
                Action::Print('l'),
                Action::Print('o'),
            ],
            actions
        );
        assert_eq!(encode(&actions), "hello");
    }

    #[test]
    fn basic_bold() {
        let mut p = Parser::new();
        let actions = p.parse_as_vec(b"\x1b[1mb");
        assert_eq!(
            vec![
                Action::CSI(CSI::Sgr(Sgr::Intensity(Intensity::Bold))),
                Action::Print('b'),
            ],
            actions
        );
        assert_eq!(encode(&actions), "\x1b[1mb");
    }

    #[test]
    fn basic_bold_italic() {
        let mut p = Parser::new();
        let actions = p.parse_as_vec(b"\x1b[1;3mb");
        assert_eq!(
            vec![
                Action::CSI(CSI::Sgr(Sgr::Intensity(Intensity::Bold))),
                Action::CSI(CSI::Sgr(Sgr::Italic(true))),
                Action::Print('b'),
            ],
            actions
        );

        assert_eq!(encode(&actions), "\x1b[1m\x1b[3mb");
    }

    #[test]
    fn basic_osc() {
        let mut p = Parser::new();
        let actions = p.parse_as_vec(b"\x1b]0;hello\x07");
        assert_eq!(
            vec![Action::OperatingSystemCommand(Box::new(
                OperatingSystemCommand::SetIconNameAndWindowTitle("hello".to_owned()),
            ))],
            actions
        );
        assert_eq!(encode(&actions), "\x1b]0;hello\x07");

        let actions = p.parse_as_vec(b"\x1b]532534523;hello\x07");
        assert_eq!(
            vec![Action::OperatingSystemCommand(Box::new(
                OperatingSystemCommand::Unspecified(vec![b"532534523".to_vec(), b"hello".to_vec()]),
            ))],
            actions
        );
        assert_eq!(encode(&actions), "\x1b]532534523;hello\x07");
    }

    #[test]
    fn basic_esc() {
        let mut p = Parser::new();
        let actions = p.parse_as_vec(b"\x1bH");
        assert_eq!(
            vec![Action::Esc(Esc::Code(EscCode::HorizontalTabSet))],
            actions
        );
        assert_eq!(encode(&actions), "\x1bH");

        let actions = p.parse_as_vec(b"\x1b%H");
        assert_eq!(
            vec![Action::Esc(Esc::Unspecified {
                intermediate: Some(b'%'),
                control: b'H',
            })],
            actions
        );
        assert_eq!(encode(&actions), "\x1b%H");
    }
}
