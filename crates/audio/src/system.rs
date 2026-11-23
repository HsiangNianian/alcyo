//! The system's own speech recognition: SpeechAnalyzer or, before macOS and iPadOS 26 or where it can't,
//! SFSpeechRecognizer, in apple/speech.swift. On a Mac that is a helper process, `alcyo-speech`; on the iPad,
//! which lets an app start no other program, the same code is built into the app (see build.rs) and runs on a
//! thread of its own, talking over two pipes. Either way each sentence goes to it as a little-endian sample
//! count and 16 kHz float32 samples, and its text comes back as a line of JSON.

use crate::Recognizer;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Child;
#[cfg(not(target_os = "ios"))]
use std::process::{Command, Stdio};

/// What runs the recognizer: the helper process, or the thread running it inside the app. Each build makes only
/// one of them, and the thread is held without being joined (stopping never waits on a sentence), hence the allow.
#[allow(dead_code)]
enum Runner {
    Process(Child),
    Thread(std::thread::JoinHandle<()>),
}

pub struct SystemSpeech {
    runner: Option<Runner>,
    input: Box<dyn Write + Send>,
    output: BufReader<Box<dyn Read + Send>>,
    /// Which of the system's recognizers answered, such as "analyzer" or "legacy".
    pub engine: String,
}

#[cfg(target_os = "ios")]
extern "C" {
    /// In apple/speech.swift: serves `locale` reading sentences from `input` and answering on `output`, until
    /// the input ends; closes both.
    fn alcyo_speech_serve(input: i32, output: i32, locale: *const std::ffi::c_char);
}

impl SystemSpeech {
    /// Start the recognizer for `locale` ("auto" for the system's language) and wait until it can recognize,
    /// which the first time can include downloading the language's model. `helper` is the program on a Mac;
    /// the iPad has it built in.
    pub fn start(helper: &Path, locale: &str) -> Result<Self> {
        #[cfg(target_os = "ios")]
        let speech = {
            let _ = helper;
            Self::in_app(locale)?
        };
        #[cfg(not(target_os = "ios"))]
        let speech = {
            let mut child = Command::new(helper)
                .args(["serve", locale])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .with_context(|| format!("starting {}", helper.display()))?;
            let input = child.stdin.take().context("no input to the speech helper")?;
            let output = child.stdout.take().context("no output from the speech helper")?;
            Self { runner: Some(Runner::Process(child)), input: Box::new(input), output: BufReader::new(Box::new(output)), engine: String::new() }
        };
        speech.wait_ready()
    }

    /// Apple's recognizer inside the app, on its own thread, over two pipes: what it reads, what it writes.
    #[cfg(target_os = "ios")]
    fn in_app(locale: &str) -> Result<Self> {
        use std::fs::File;
        use std::os::fd::FromRawFd;
        let pipe = || -> Result<(i32, i32)> {
            let mut fds = [0; 2];
            // SAFETY: `fds` has room for the two descriptors pipe() writes.
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error()).context("making a pipe for speech recognition");
            }
            Ok((fds[0], fds[1]))
        };
        let (their_input, our_input) = pipe()?;
        let (our_output, their_output) = pipe()?;
        let locale = std::ffi::CString::new(locale).context("a locale with a NUL in it")?;
        let thread = std::thread::Builder::new()
            .name("apple speech".into())
            // SAFETY: the descriptors are this thread's to hand over; the Swift side closes them when it ends.
            .spawn(move || unsafe { alcyo_speech_serve(their_input, their_output, locale.as_ptr()) })
            .context("starting Apple's speech recognition")?;
        // SAFETY: both ends were just made and are owned by nothing else.
        let (input, output) = unsafe { (File::from_raw_fd(our_input), File::from_raw_fd(our_output)) };
        Ok(Self { runner: Some(Runner::Thread(thread)), input: Box::new(input), output: BufReader::new(Box::new(output)), engine: String::new() })
    }

    fn wait_ready(mut self) -> Result<Self> {
        let speech = &mut self;
        loop {
            let event = speech.next_event()?;
            match event["type"].as_str() {
                Some("ready") => {
                    speech.engine = event["engine"].as_str().unwrap_or_default().to_string();
                    return Ok(self);
                }
                Some("error") => bail!("{}", event["message"].as_str().unwrap_or("the system's speech recognition did not start")),
                Some("fallback") => eprintln!("SpeechAnalyzer could not be used, so the older recognizer is: {}", event["message"].as_str().unwrap_or_default()),
                _ => {}
            }
        }
    }

    fn next_event(&mut self) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.output.read_line(&mut line)? == 0 {
                bail!("the speech helper stopped");
            }
            if let Ok(event) = serde_json::from_str::<Value>(line.trim()) {
                return Ok(event);
            }
        }
    }
}

impl Recognizer for SystemSpeech {
    fn recognize(&mut self, samples: &[f32]) -> Result<String> {
        let count = u32::try_from(samples.len()).map_err(|_| anyhow!("sentence too long"))?;
        let mut bytes = Vec::with_capacity(4 + samples.len() * 4);
        bytes.extend_from_slice(&count.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.input.write_all(&bytes).and_then(|()| self.input.flush()).context("sending a sentence to the speech helper")?;
        loop {
            let event = self.next_event()?;
            match event["type"].as_str() {
                Some("text") => return Ok(event["text"].as_str().unwrap_or_default().trim().to_string()),
                Some("error") => bail!("{}", event["message"].as_str().unwrap_or("recognition failed")),
                _ => {}
            }
        }
    }
}

impl Drop for SystemSpeech {
    fn drop(&mut self) {
        // A count of 0 ends it; the helper also leaves once this process is gone.
        let _ = self.input.write_all(&0_u32.to_le_bytes());
        let _ = self.input.flush();
        match self.runner.take() {
            Some(Runner::Process(mut child)) => {
                if child.try_wait().ok().flatten().is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            // It ends after the sentence it is on; not waited for, so stopping never hangs on one.
            Some(Runner::Thread(_)) | None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Its own: the module above imports Command only where it starts the helper, which is not on iOS.
    #[cfg(target_os = "ios")]
    use std::process::Command;

    /// A stand-in helper that speaks the same protocol and says how many samples it got.
    const FAKE: &str = r#"
import json, struct, sys
print(json.dumps({"type": "fallback", "message": "no analyzer here"}), flush=True)
print(json.dumps({"type": "ready", "engine": "fake", "locale": sys.argv[2]}), flush=True)
while True:
    head = sys.stdin.buffer.read(4)
    if len(head) < 4: break
    (n,) = struct.unpack("<I", head)
    if n == 0: break
    body = sys.stdin.buffer.read(n * 4)
    total = sum(struct.unpack("<%df" % n, body))
    print(json.dumps({"type": "text", "text": " %d samples, sum %.1f " % (n, total)}), flush=True)
"#;

    fn fake_helper() -> Option<std::path::PathBuf> {
        Command::new("python3").arg("--version").output().ok()?;
        let dir = std::env::temp_dir().join(format!("alcyo-speech-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let script = dir.join("fake.py");
        std::fs::write(&script, FAKE).ok()?;
        let helper = dir.join("alcyo-speech");
        std::fs::write(&helper, format!("#!/bin/sh\nexec python3 -I {} \"$@\"\n", script.display())).ok()?;
        #[cfg(not(unix))]
        return None;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).ok()?;
        }
        Some(helper)
    }

    #[test]
    fn sentences_go_to_the_helper_and_text_comes_back() {
        let Some(helper) = fake_helper() else { return };
        let mut speech = SystemSpeech::start(&helper, "ja-JP").unwrap();
        assert_eq!(speech.engine, "fake");
        assert_eq!(speech.recognize(&[0.5; 1600]).unwrap(), "1600 samples, sum 800.0");
        assert_eq!(speech.recognize(&[0.25; 8]).unwrap(), "8 samples, sum 2.0");
    }

    #[test]
    fn a_missing_helper_is_an_error() {
        assert!(SystemSpeech::start(Path::new("/nowhere/alcyo-speech"), "auto").is_err());
    }
}
