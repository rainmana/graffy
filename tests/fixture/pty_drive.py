#!/usr/bin/env python3
"""Drive a command inside a pseudo-terminal, the way a human at a keyboard would.

graffy only reads approvals from stdin when stdin is a terminal, so a pipe
cannot reach that path. This runs the command with a real (pseudo) terminal on
stdin/stdout/stderr and plays a script against it.

    pty_drive.py <timeout-seconds> <command> [args...] --- <step>...

Steps, in order:
    expect=TEXT   wait until TEXT appears in the output after the last match
    send=TEXT     type TEXT (include the newline yourself)
    interrupt     press Ctrl-C

Exit status: 0 when every step ran and the command then exited; 1 when a step
timed out (the missing text is named on stderr). The terminal transcript is
written to stdout either way. Standard library only; Unix only.
"""

import os
import pty
import select
import signal
import sys
import time


def main() -> int:
    timeout = float(sys.argv[1])
    split = sys.argv.index("---")
    command, steps = sys.argv[2:split], sys.argv[split + 1 :]

    pid, terminal = pty.fork()
    if pid == 0:
        os.execvp(command[0], command)

    seen = b""
    cursor = 0  # output before this was already matched by an earlier expect

    def read_some(deadline: float) -> bool:
        """Append available output to `seen`. False at timeout or hang-up."""
        nonlocal seen
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False
        ready, _, _ = select.select([terminal], [], [], remaining)
        if not ready:
            return False
        try:
            chunk = os.read(terminal, 4096)
        except OSError:  # Linux reports the child's exit as EIO
            return False
        seen += chunk
        return bool(chunk)

    failure = None
    for step in steps:
        kind, _, text = step.partition("=")
        if kind == "send":
            os.write(terminal, text.encode())
        elif kind == "interrupt":
            os.write(terminal, b"\x03")
        elif kind == "expect":
            wanted = text.encode()
            deadline = time.monotonic() + timeout
            while wanted not in seen[cursor:]:
                if not read_some(deadline):
                    failure = f"timed out after {timeout:g}s waiting for {text!r}"
                    break
            if failure:
                break
            cursor = seen.index(wanted, cursor) + len(wanted)
        else:
            failure = f"unknown step {step!r}"
            break

    if failure is None:
        deadline = time.monotonic() + timeout
        while read_some(deadline):
            pass
        # The terminal hangs up a moment before the exit status is ready.
        finished, _ = os.waitpid(pid, os.WNOHANG)
        while finished == 0 and time.monotonic() < deadline:
            time.sleep(0.01)
            finished, _ = os.waitpid(pid, os.WNOHANG)
        if finished == 0:
            failure = f"command still running {timeout:g}s after the last step"
    if failure is not None:
        try:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass

    sys.stdout.write(seen.decode(errors="replace"))
    sys.stdout.flush()
    if failure is not None:
        sys.stderr.write(f"pty_drive: {failure}\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
