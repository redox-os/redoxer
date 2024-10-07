use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::process::{Child, Command, ExitStatus, Stdio};

use anyhow::{bail, Context, Result};
use event::{user_data, EventFlags, EventQueue};
use libredox::flag::{O_CLOEXEC, O_CREAT, O_NONBLOCK, O_RDWR};
use libredox::Fd;
use syscall::{Io, Pio};

const DEFAULT_COLS: u32 = 80;
const DEFAULT_LINES: u32 = 30;

pub fn handle(
    event_queue: EventQueue<EventSource>,
    master_fd: Fd,
    timeout_fd: Fd,
    process: &mut Child,
) -> Result<ExitStatus> {
    'outer: for event in event_queue {
        match event.context("failed to read from event queue")?.user_data {
            EventSource::Master => {
                let mut packet = [0; 4096];

                loop {
                    // Read data from PTY master
                    let count = match master_fd.read(&mut packet) {
                        Ok(0) => break 'outer,
                        Ok(count) => count,
                        Err(ref err) if err.is_wouldblock() => continue,
                        Err(err) => return Err(err.into()),
                    };

                    // Write data to stdout
                    libredox::call::write(1, &packet[1..count])?;

                    for i in 1..count {
                        // Write byte to QEMU debugcon (Bochs compatible)
                        Pio::<u8>::new(0xe9).write(packet[i]);
                    }
                }
            }
            EventSource::Timeout => {
                let mut timespec = syscall::TimeSpec::default();
                timeout_fd.read(&mut timespec)?;

                timespec.tv_sec += 1;
                timeout_fd.write(&mut timespec)?;
            }
        }
        match process.try_wait() {
            Ok(status_opt) => match status_opt {
                Some(status) => return Ok(status),
                None => (),
            },
            Err(err) => match err.kind() {
                io::ErrorKind::WouldBlock => (),
                _ => return Err(err.into()),
            },
        }
    }

    let _ = process.kill();
    Ok(process.wait()?)
}

pub fn getpty(columns: u32, lines: u32) -> Result<(Fd, String)> {
    let master = Fd::open("/scheme/pty", O_CLOEXEC | O_RDWR | O_CREAT | O_NONBLOCK, 0)
        .context("failed to open pty")?;

    if let Ok(winsize_fd) = master.dup(b"winsize") {
        let _ = syscall::write(
            winsize_fd,
            &redox_termios::Winsize {
                ws_row: lines as u16,
                ws_col: columns as u16,
            },
        );
    }

    let mut buf: [u8; 4096] = [0; 4096];
    let count = master.fpath(&mut buf)?;
    Ok((master, String::from_utf8(Vec::from(&buf[..count])).unwrap()))
}
user_data! {
    pub enum EventSource {
        Master,
        Timeout,
    }
}

fn inner() -> Result<()> {
    unsafe {
        syscall::iopl(3)?;
    }

    let config = fs::read_to_string("/etc/redoxerd").context("failed to read redoxerd config")?;
    let mut config_lines = config.lines();

    let (columns, lines) = (DEFAULT_COLS, DEFAULT_LINES);
    let (master_fd, pty) = getpty(columns, lines)?;

    let timeout_fd = Fd::open("/scheme/time/4", O_CLOEXEC | O_RDWR | O_NONBLOCK, 0)?;

    let event_queue = EventQueue::new()?;

    event_queue.subscribe(master_fd.raw(), EventSource::Master, EventFlags::READ)?;
    event_queue.subscribe(timeout_fd.raw(), EventSource::Timeout, EventFlags::READ)?;

    let slave_stdin = OpenOptions::new().read(true).open(&pty)?;
    let slave_stdout = OpenOptions::new().write(true).open(&pty)?;
    let slave_stderr = OpenOptions::new().write(true).open(&pty)?;

    let Some(name) = config_lines.next() else {
        bail!("/etc/redoxerd does not specify command");
    };
    let mut command = Command::new(name);
    println!("redoxer: starting `{name}`");
    for arg in config_lines {
        command.arg(arg);
    }
    unsafe {
        command
            .stdin(Stdio::from_raw_fd(slave_stdin.into_raw_fd()))
            .stdout(Stdio::from_raw_fd(slave_stdout.into_raw_fd()))
            .stderr(Stdio::from_raw_fd(slave_stderr.into_raw_fd()))
            .env("COLUMNS", format!("{}", columns))
            .env("LINES", format!("{}", lines))
            .env("TERM", "xterm-256color")
            .env("TTY", &pty);
    }

    let mut process = command.spawn().context("failed to spawn command")?;

    let status = handle(event_queue, master_fd, timeout_fd, &mut process)?;

    if status.success() {
        Ok(())
    } else {
        bail!("command failed: {status}");
    }
}

pub fn main() {
    match inner() {
        Ok(()) => {
            // Exit with success using qemu device
            Pio::<u16>::new(0x604).write(0x2000);
            Pio::<u8>::new(0x501).write(51 / 2);
        }
        Err(err) => {
            eprintln!("redoxerd: {err:?}");
            // Exit with error using qemu device
            Pio::<u8>::new(0x501).write(53 / 2);
        }
    }
}
