//! Helper for running short-lived child processes without flashing a console window.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Builds a command that never opens a console window.
pub fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Runs `program` hidden and returns its stdout, or `None` when it cannot be
/// started, exits with a failure code or does not finish within `timeout`.
pub fn run_hidden(program: &str, args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
    let mut child = hidden_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = reader.join().ok()?;
                return status.success().then_some(output);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// Decodes console output that may be UTF-16LE (as `wsl.exe` emits) or UTF-8.
pub fn decode_console_output(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && (bytes.starts_with(&[0xFF, 0xFE]) || bytes.iter().skip(1).step_by(2).any(|b| *b == 0));
    let text = if looks_utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').replace('\0', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_le() {
        let bytes: Vec<u8> = "Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(decode_console_output(&bytes), "Ubuntu\r\nDebian\r\n");
    }

    #[test]
    fn decodes_utf8() {
        assert_eq!(decode_console_output(b"hello\n"), "hello\n");
    }

    #[test]
    fn missing_program_returns_none() {
        assert!(run_hidden(
            "definitely-not-a-real-program-aium",
            &[],
            Duration::from_secs(1)
        )
        .is_none());
    }
}
