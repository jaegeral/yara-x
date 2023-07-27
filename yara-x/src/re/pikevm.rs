use std::mem;

use crate::re::instr::{
    decode_instr, epsilon_closure, CodeLoc, EpsilonClosureState, Instr,
};

pub(crate) enum Match {
    Continue,
    Stop,
}

/// Represents a [Pike's VM](https://swtch.com/~rsc/regexp/regexp2.html) that
/// executes VM code produced by the [compiler][`crate::re::compiler::Compiler`].
pub(crate) struct PikeVM<'r> {
    code: &'r [u8],
    /// The list of currently active threads. Each item in this list is a
    /// position within the VM code, pointing to some VM instruction. Each item
    /// in the list is unique, the VM guarantees that there aren't two active
    /// threads at the same VM instruction.
    threads: Vec<usize>,
    /// The list of threads that will become the active threads when the next
    /// byte is read from the input.
    next_threads: Vec<usize>,
    cache: EpsilonClosureState,
}

impl<'r> PikeVM<'r> {
    /// Creates a new [`PikeVM`].
    pub fn new(code: &'r [u8]) -> Self {
        Self {
            code,
            threads: Vec::new(),
            next_threads: Vec::new(),
            cache: EpsilonClosureState::new(),
        }
    }

    /// Executes VM code starting at the `start` location and returns the
    /// number of bytes from `fwd_input` that matched. The number of bytes
    /// returned can be zero if the VM matches a zero-length string. Returns
    /// `None` if the data doesn't match the regexp.
    ///
    /// `bck_input` is an iterator that returns the bytes that are before
    /// the starting point of `fwd_input`, in reverse order. For instance,
    /// suppose we have the string `a b c e f g h i`, and `fwd_input` starts
    /// at the `f` character and returns `f`, `g`, `h` and `i` in that order.
    /// In such case `bck_input` will return `e`, `c`, `b` and `a`.
    ///
    /// ```text
    ///       a  b  c  e  f   g   h   i
    ///                   |  
    ///      <- bck_input | fwd_input ->
    /// ```
    ///
    /// The purpose of `bck_input` is allowing the function to access the bytes
    /// that appear right before the start of `fwd_input` for matching some
    /// look-around assertions that need information about the surrounding
    /// bytes.
    pub(crate) fn try_match<'a, C, F, B>(
        &mut self,
        start: C,
        mut fwd_input: F,
        mut bck_input: B,
        mut f: impl FnMut(usize) -> Match,
    ) where
        C: CodeLoc,
        F: Iterator<Item = &'a u8>,
        B: Iterator<Item = &'a u8>,
    {
        let step = 1;
        let mut current_pos = 0;
        let mut curr_byte = fwd_input.next();

        epsilon_closure(
            self.code,
            start,
            curr_byte,
            bck_input.next(),
            &mut self.cache,
            &mut self.threads,
        );

        while !self.threads.is_empty() {
            let next_byte = fwd_input.next();

            for ip in self.threads.iter() {
                let (instr, size) = decode_instr(&self.code[*ip..]);

                let is_match = match instr {
                    Instr::AnyByte => {
                        matches!(curr_byte, Some(_))
                    }
                    Instr::Byte(byte) => {
                        matches!(curr_byte, Some(b) if *b == byte)
                    }
                    Instr::MaskedByte { byte, mask } => {
                        matches!(curr_byte, Some(b) if *b & mask == byte)
                    }
                    Instr::CaseInsensitiveChar(byte) => {
                        matches!(curr_byte, Some(b) if b.to_ascii_lowercase() == byte)
                    }
                    Instr::ClassBitmap(class) => {
                        matches!(curr_byte, Some(b) if class.contains(*b))
                    }
                    Instr::ClassRanges(class) => {
                        matches!(curr_byte, Some(b) if class.contains(*b))
                    }
                    Instr::Match => match f(current_pos) {
                        Match::Stop => break,
                        Match::Continue => false,
                    },
                    Instr::Eoi => {
                        // TODO: is this correct?
                        break;
                    }
                    _ => unreachable!(),
                };

                if is_match {
                    epsilon_closure(
                        self.code,
                        C::from(*ip + size),
                        next_byte,
                        curr_byte,
                        &mut self.cache,
                        &mut self.next_threads,
                    );
                }
            }

            curr_byte = next_byte;
            current_pos += step;
            mem::swap(&mut self.threads, &mut self.next_threads);
            self.next_threads.clear();
        }
    }
}
