#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::io::{Read, Write};
use std::cell::RefCell;
use std::process::{Command, Stdio, ChildStdin};
use std::time::Duration;
use std::sync::OnceLock;

use rand::{distributions::Alphanumeric, Rng};
use bstr::ByteSlice;
use nu_pretty_hex::pretty_hex;

use shlex::bytes;

#[derive(PartialEq, Debug)]
enum CompatMode {
    Bash,
    Zsh,
    Dash,
    BusyboxAsh,
    Fish,
    Mksh,
    Other
}

fn env_var_or(var: &str, default: &str) -> String {
    match std::env::var(var) {
        Ok(s) => s,
        Err(std::env::VarError::NotPresent) => default.into(),
        Err(std::env::VarError::NotUnicode(_)) => panic!("unicode"),
    }
}

fn env_bool(var: &str, default: bool) -> bool {
    match &*env_var_or(var, "") {
        "" => default,
        "0" => false,
        "1" => true,
        _ => panic!("{} should be 0 or 1", var),
    }
}

fn env_u64(var: &str, default: u64) -> u64 {
    match &*env_var_or(var, "") {
        "" => default,
        x => x.parse().unwrap(),
    }
}

struct Config {
    fuzz_shell: String,
    debug: bool,
    use_docker: bool,
    use_pty: bool,
    cooked_pty: bool, // just for experimentation; this is expected to fail
    compat_mode: CompatMode,
    shell_is_interactive: bool,
    fuzz_timeout: u64,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
impl Config {
    fn get() -> &'static Config {
        CONFIG.get_or_init(|| {
            let fuzz_shell = env_var_or("FUZZ_SHELL", "zsh --no-rcs");
            let use_pty = env_bool("FUZZ_USE_PTY", true);
            let shell_is_interactive = env_bool("FUZZ_SHELL_IS_INTERACTIVE", {
                // default: guess -i/+i from the string (very crude)
                if fuzz_shell.contains(" -i") {
                    true
                } else if fuzz_shell.contains(" +i") {
                    false
                } else {
                    use_pty
                }
            });
            let compat_mode = match &*env_var_or("FUZZ_COMPAT_MODE", "") {
                "bash" => CompatMode::Bash,
                "zsh" => CompatMode::Zsh,
                "dash" => CompatMode::Dash,
                "busybox ash" => CompatMode::BusyboxAsh,
                "fish" => CompatMode::Fish,
                "mksh" => CompatMode::Mksh,
                "other" => CompatMode::Other,
                "" => {
                    // default: guess the shell from the string (somewhat dumbly)
                    if fuzz_shell.contains("bash") {
                        CompatMode::Bash
                    } else if fuzz_shell.contains("zsh") {
                        CompatMode::Zsh
                    } else if fuzz_shell.contains("dash") {
                        CompatMode::Dash
                    } else if fuzz_shell.contains("ash") {
                        CompatMode::BusyboxAsh
                    } else if fuzz_shell.contains("fish") {
                        CompatMode::Fish
                    } else if fuzz_shell.contains("mksh") {
                        CompatMode::Mksh
                    } else {
                        CompatMode::Other
                    }
                },
                _ => panic!("invalid FUZZ_COMPAT_MODE")
            };
            Config {
                debug: env_bool("FUZZ_DEBUG", false),
                use_docker: env_bool("FUZZ_USE_DOCKER", true),
                use_pty,
                cooked_pty: env_bool("FUZZ_COOKED_PTY", false),
                compat_mode,
                fuzz_shell,
                shell_is_interactive,
                fuzz_timeout: env_u64("FUZZ_TIMEOUT", 120),
            }
        })
    }
}


struct Shell {
    stdout_receiver: mpsc::Receiver<Vec<u8>>,
    stdout_buf: Vec<u8>,
    stdin: ChildStdin,
}
impl Shell {
    fn new() -> Shell {
        let config = Config::get();
        let mut real_shell = config.fuzz_shell.clone();
        real_shell = format!("{} 2>&1", real_shell);
        #[cfg(target_os = "macos")]
        if !config.use_docker {
            // Provide some protection for native macOS execution.  Not actually secure (it doesn't
            // block IPC) but should be good enough against _accidental_ bad commands.  Probably.
            let sandbox_profile = r#"""
                (version 1)
                (allow default)
                (deny file-write*)
                (allow file-write-data (literal "/dev/null"))
            """#;
            real_shell = format!("sandbox-exec -p {} sh -c {}",
                                 shlex::try_quote(sandbox_profile).unwrap(),
                                 shlex::try_quote(&real_shell).unwrap());
        }
        if config.use_pty {
            // Use python3 to set up a pty.  Don't do it locally because then we're validating the
            // pty relay layer of Docker for Mac and I've had issues with it.
            real_shell = format!("exec python3 -c 'import sys, pty; exit(pty.spawn(sys.argv[1:]))' sh -c 'stty sane {} -echo; exec '{}",
                if config.cooked_pty { "cooked" } else { "raw" },
                shlex::try_quote(&real_shell).unwrap());
            //real_shell = format!(r#"CMD={} socat -b1 - 'EXEC:sh -c "\"eval \\\"$CMD\\\"\"",pty,sane,raw,echo=0,nonblock'"#, shlex::quote(&real_shell));
        }
        if config.use_docker {
            // By default, run in a Docker container so that we don't cause random commands to be
            // run on the host (if quoting is buggy), or clutter up the shell history file for
            // interactive shells.
            real_shell = format!("docker run --rm --log-opt max-size=1m -i {} $(docker build -q - < {}/Dockerfile) sh -c {}",
                                 env_var_or("FUZZ_DOCKER_ARGS", ""),
                                 shlex::try_quote(env!("CARGO_MANIFEST_DIR")).unwrap(),
                                 shlex::try_quote(&real_shell).unwrap());
        }
        if config.debug {
            println!("=> {}", real_shell);
        }
        let cmd = Command::new("/bin/sh")
            .arg("-c")
            .arg(real_shell)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to execute shell");
        let mut stdout = cmd.stdout.unwrap();
        let stdin = cmd.stdin.unwrap();
        let (sender, receiver) = mpsc::channel();

        // Read stdout on a separate thread to avoid deadlocking on pipe buffers.
        thread::spawn(move || {
            loop {
                let mut buf: Vec<u8> = Vec::new();
                buf.resize(128, 0u8);
                let size = stdout.read(&mut buf).expect("failed to read stdout");
                if size == 0 {
                    break;
                }
                buf.truncate(size);
                if sender.send(buf).is_err() { break; }
            }
        });

        let mut this = Shell { stdout_receiver: receiver, stdout_buf: Vec::new(), stdin };

        this.wait_until_responsive();
        this
    }

    // Keep reading until we find `delim`; return the output without `delim`.
    fn read_until_delim(&mut self, delim: &[u8], timeout: Duration) -> Result<Vec<u8>, RecvTimeoutError> {
        let mut pos = 0;
        loop {
            if Config::get().debug {
                println!("READ: {}", pretty_hex(&self.stdout_buf));
                //println!(">> wanted: {}", pretty_hex(&delim));
                //if self.stdout_buf.find(b"zsh: no such event").is_some() { panic!("xxx"); }
            }
            if let Some(delim_pos) = self.stdout_buf[pos..].find(delim) {
                let ret = self.stdout_buf[..pos + delim_pos].to_owned();
                self.stdout_buf.drain(0..(pos + delim_pos + delim.len()));
                return Ok(ret);
            }
            pos = self.stdout_buf.len().saturating_sub(delim.len() - 1);
            let new_data = self.stdout_receiver.recv_timeout(timeout)?;
            self.stdout_buf.extend_from_slice(&new_data);
        }
    }

    // Write something.
    fn write(&mut self, text: &[u8]) {
        if Config::get().debug {
            println!("WROTE: {}", pretty_hex(&text));
        }
        self.stdin.write_all(text).expect("failed to write to shell stdin");
        self.stdin.flush().expect("failed to flush shell stdin"); // shouldn't be necessary
    }

    // Wait until the shell listens to us.  Also disable history logging in case this is an
    // interactive shell.
    fn wait_until_responsive(&mut self) {
        let unset_histfile: &[u8] = if let CompatMode::Fish = Config::get().compat_mode {
            b""
        } else {
            b"; unset HISTFILE"
        };
        for _ in 0..60 {
            let delimiter = random_alphanum();
            self.write(&[
                b"echo ",
                &delimiter[..1],
                b"''",
                &delimiter[1..],
                unset_histfile,
                b"\n",
            ].concat());
            match self.read_until_delim(&delimiter, Duration::from_millis(500)) {
                Ok(_) => return,
                Err(RecvTimeoutError::Timeout) => (),
                Err(RecvTimeoutError::Disconnected) => panic!("shell exited"),
            }
        };
        panic!("timeout waiting for shell to be responsive");
    }
}

/// Return a byte string of 10 random alphanumeric characters.
///
/// Used as delimiters around the stuff we actually want to quote.
///
/// Using `rand` makes the fuzzer slightly less reproducible, but the specific string chosen
/// shouldn't make a difference, and having it be different every time reduces the chance of false
/// positive matches with interactive shells, in case the delimiter gets into shell history and
/// then the shell prints it as part of some autocompletion routine.
///
/// (Though in theory, unsetting HISTFILE as done above should be enough to prevent it from getting
/// into shell history in the first place.)
fn random_alphanum() -> Vec<u8> {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .collect()
}

thread_local! {
    static SHELL: RefCell<Shell> = RefCell::new(Shell::new());
}

fuzz_target!(|unquoted: &[u8]| {
    let mut unquoted: Vec<u8> = unquoted.into();
    {
        // Strip nul characters.
        for byte in unquoted.iter_mut() {
            if *byte == 0 {
                *byte = b'x';
            }
        }
    }
    let config = Config::get();

    /*
    TODO:
    let length_limit = match config.compat_mode {
        // zsh in interactive mode gets very slow for long inputs.
        CompatMode::Zsh if config.shell_is_interactive => Some(1024),
        // busybox ash has a line length limit when reading from a pty (and we need to be
        // conservative since this length is pre-quoting).
        CompatMode::BusyboxAsh  if config.use_pty => Some(256),
        // Otherwise no length limit.
        _ => None
    };
    */
    let length_limit = Some(256);

    if let Some(limit) = length_limit {
        unquoted.truncate(limit);
    }

    // Disable certain types of input for shells that can't handle them.
    // This is perhaps unnecessarily tightly dialed in to the quirks of specific shells, but I've
    // found this helpful as a way understand those shells' behavior better.

    // Strip control characters in pty mode because they are special there and we cannot quote them
    // properly while being POSIX-compatible (see crate documentation).
    // And bash tries to interpret them even without a pty in interactive mode.
    let strip_controls = config.use_pty ||
        (config.compat_mode == CompatMode::Bash && config.shell_is_interactive);

    // Strip \r in cases where shells turns it into \n.
    // - bash: happens in interactive mode, using a pty, or both
    // - zsh: happens if using a pty (can't test interactive mode without pty)
    // - busybox ash: happens if using a pty (not in interactive mode)
    // - fish: actually turns \n into \r\n, but we need to strip it from input
    // In all cases, I verified using strace that this is happening in the shell rather than in the
    // kernel's tty layer.  The tty layer can be configured to do things like that, but apparently
    // it's not the default.
    let strip_crs = match config.compat_mode {
        CompatMode::Bash => config.use_pty || config.shell_is_interactive,
        CompatMode::Zsh | CompatMode::BusyboxAsh => config.use_pty,
        CompatMode::Fish => config.use_pty,
        CompatMode::Mksh => config.use_pty,
        _ => false
    };

    // Ignore \r added by the shell.  This assumes strip_crs is also on.
    let ignore_added_crs = match config.compat_mode {
        CompatMode::Fish => config.use_pty,
        _ => false
    };

    // Strip characters with the high bit set only if the string as a whole is invalid UTF-8,
    // because:
    // - bash: sometimes strips bytes at the end that could be the beginning of a UTF-8
    //   sequence, again if in interactive mode and/or using a pty
    //   XXX and also valid UTF-8?
    // - zsh: goes through multibyte routines and will replace invalid characters with
    //   question marks, only if interactive
    // - busybox ash: something similar, only if using a pty
    // - fish: ditto
    // Again, can't deal with this properly while being POSIX-compatible.  (In theory we could make
    // them safer by quoting, so the question marks wouldn't be treated as glob characters, but the
    // string still wouldn't round-trip properly, so don't bother.)
    let is_invalid_utf8 = std::str::from_utf8(&unquoted).is_err();
    let strip_8bit = match config.compat_mode {
        CompatMode::Bash => config.use_pty || config.shell_is_interactive,
        CompatMode::Zsh => config.shell_is_interactive && is_invalid_utf8,
        CompatMode::BusyboxAsh |
        CompatMode::Fish |
        CompatMode::Mksh => config.use_pty && is_invalid_utf8,
        CompatMode::Dash |
        CompatMode::Other => false,
    };

    for byte in unquoted.iter_mut() {
        if (strip_controls && byte.is_ascii_control() && *byte != b'\r' && *byte != b'\n') ||
           (*byte == b'\0') ||
           (strip_crs && *byte == b'\r') ||
           (strip_8bit && *byte >= 0x80) {
            *byte = b'a' + (*byte % 26);
        }
    }

    //println!("len={}", unquoted.len());

    // We already filtered out nul bytes so this should be successful.
    let quoted = bytes::try_quote(&unquoted).unwrap();

    SHELL.with(|ref_shell| {
        let mut shell = ref_shell.borrow_mut();
        // Add a random prefix and suffix to ensure we can identify the output while ignoring the shell
        // prompt.  The prefix and suffix are alphanumeric so they don't need to be quoted.  They are
        // placed outside the double quotes just in case any shell cares about something being the
        // first or last character in a double-quoted string (though it shouldn't).
        // Also break up the prefix and suffix so that we don't get them back from shell echo.
        let mut alphanum_prefix = random_alphanum();
        let mut alphanum_suffix = random_alphanum();
        // Add the literal string PREFIX to the end of the prefix, and SUFFIX to the start of the
        // suffix, to make them more recognizable.
        alphanum_prefix.extend_from_slice(b"PREFIX");
        alphanum_suffix.splice(0..0, *b"SUFFIX");
        // Write the command:
        //    printf %s "AAAPREFIX***SUFFIXBBB"
        //               ^^^---------------------random prefix
        //                        ^^^------------quoted string
        //                                 ^^^---random suffix
        let full_command = [
            b"printf %s ",
            &alphanum_prefix[..1],
            b"\"\"",
            &alphanum_prefix[1..],
            &quoted,
            &alphanum_suffix[..1],
            b"\"\"",
            &alphanum_suffix[1..],
            b"\n"
        ].concat();
        shell.write(&full_command);
        let read_data = shell.read_until_delim(&alphanum_suffix, Duration::from_secs(config.fuzz_timeout)).unwrap();
        let prefix_pos = read_data.find(&alphanum_prefix).expect("did not find prefix");
        let mut read_data = &read_data[prefix_pos + alphanum_prefix.len() ..];
        let buf: Vec<u8>;
        //println!("read back {} bytes", read_data.len());
        if ignore_added_crs {
            buf = read_data.iter().cloned().filter(|&c| c != b'\r').collect();
            read_data = &buf[..];
        }
        if read_data != unquoted {
            panic!("original:\n{}\nread from shell:\n{}\nquoted:\n{}",
                   pretty_hex(&unquoted), pretty_hex(&read_data), pretty_hex(&quoted));
        }
    })
});
