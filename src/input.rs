//! The process's one stdin line reader (issue #35).
//!
//! A blocked `read_line` cannot be cancelled. When each prompt started its own
//! read, an approval abandoned by Ctrl-C or a deadline left a thread still
//! blocked on stdin, and that thread took — and threw away — the next line the
//! human typed at the chat prompt.
//!
//! Here one thread owns the reading, and reads only when someone is waiting.
//! A waiter that gives up is noticed when its line arrives: the line is kept
//! and handed to the next waiter instead of being dropped. Lines are never
//! reordered and never read ahead of a request.
//!
//! Plain `graffy chat` and the plain-terminal approval prompt read through
//! this. `graffy rate` and the setup interviews read stdin directly; they
//! never share a process with either.

use std::sync::{OnceLock, mpsc};

use tokio::sync::oneshot;

/// One read: `Ok(None)` is end of input. The line keeps its newline.
type Line = std::io::Result<Option<String>>;

#[derive(Clone)]
pub struct Lines {
    waiters: mpsc::Sender<oneshot::Sender<Line>>,
}

impl Lines {
    /// The reader over this process's stdin. Every call returns a handle to
    /// the same reader; its thread starts on first use.
    pub fn stdin() -> Self {
        static STDIN: OnceLock<Lines> = OnceLock::new();
        STDIN
            .get_or_init(|| Self::over(|line| std::io::stdin().read_line(line)))
            .clone()
    }

    /// `read_line` has the contract of [`std::io::BufRead::read_line`].
    fn over(
        mut read_line: impl FnMut(&mut String) -> std::io::Result<usize> + Send + 'static,
    ) -> Self {
        let (waiters, waiting) = mpsc::channel::<oneshot::Sender<Line>>();
        // A plain thread, not a runtime blocking task: the runtime would wait
        // for a blocked read at shutdown, and the process could not exit.
        std::thread::spawn(move || {
            let mut unclaimed: Option<Line> = None;
            // Ends when every handle is gone.
            while let Ok(waiter) = waiting.recv() {
                let line = unclaimed.take().unwrap_or_else(|| {
                    let mut line = String::new();
                    read_line(&mut line).map(|read| (read > 0).then_some(line))
                });
                // The waiter gave up while the read was blocked: what arrived
                // belongs to whoever asks next.
                if let Err(line) = waiter.send(line) {
                    unclaimed = Some(line);
                }
            }
        });
        Self { waiters }
    }

    /// Wait for the next line. Dropping this future gives the wait up without
    /// losing a line.
    pub async fn next(&self) -> Line {
        let (waiter, line) = oneshot::channel();
        let gone = || std::io::Error::other("the stdin reader thread has stopped");
        self.waiters.send(waiter).map_err(|_| gone())?;
        line.await.map_err(|_| gone())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::time::Duration;

    fn piped() -> (Lines, std::io::PipeWriter) {
        let (reader, writer) = std::io::pipe().unwrap();
        let mut reader = std::io::BufReader::new(reader);
        (Lines::over(move |line| reader.read_line(line)), writer)
    }

    #[tokio::test]
    async fn a_wait_that_is_given_up_does_not_swallow_the_next_line() {
        let (lines, mut typed) = piped();
        // An approval prompt starts waiting, then its turn is cancelled.
        let approval = tokio::time::timeout(Duration::from_millis(100), lines.next()).await;
        assert!(
            approval.is_err(),
            "nothing was typed, so the wait timed out"
        );

        // The human's next line belongs to the chat prompt that is now waiting.
        let chat = lines.next();
        typed.write_all(b"hello\nsecond\n").unwrap();
        assert_eq!(chat.await.unwrap().as_deref(), Some("hello\n"));
        assert_eq!(lines.next().await.unwrap().as_deref(), Some("second\n"));
    }

    #[tokio::test]
    async fn several_waits_given_up_in_a_row_lose_nothing() {
        let (lines, mut typed) = piped();
        for _ in 0..3 {
            let gave_up = tokio::time::timeout(Duration::from_millis(20), lines.next()).await;
            assert!(gave_up.is_err());
        }
        typed.write_all(b"one\ntwo\n").unwrap();
        assert_eq!(lines.next().await.unwrap().as_deref(), Some("one\n"));
        assert_eq!(lines.next().await.unwrap().as_deref(), Some("two\n"));
    }

    #[tokio::test]
    async fn end_of_input_is_reported_not_invented() {
        let (lines, mut typed) = piped();
        typed.write_all(b"last").unwrap();
        drop(typed);
        // A final line without a newline is still a line; then input is over.
        assert_eq!(lines.next().await.unwrap().as_deref(), Some("last"));
        assert_eq!(lines.next().await.unwrap(), None);
        assert_eq!(lines.next().await.unwrap(), None);
    }
}
