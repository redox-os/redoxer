use anyhow::Context;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::process::{Child, Command, ExitStatus, Stdio};
use syscall::{Io, Pio, ProcSchemeVerb};

const DEFAULT_COLS: u32 = 80;
const DEFAULT_LINES: u32 = 30;

fn syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno)
}

extern "C" {
    fn redox_cur_thrfd_v0() -> usize;

    fn redox_sys_call_v0(
        fd: usize,
        payload: *mut u8,
        payload_len: usize,
        flags: usize,
        metadata: *const u64,
        metadata_len: usize,
    ) -> usize;
}

unsafe fn sys_call<T>(
    fd: usize,
    payload: &mut T,
    flags: usize,
    metadata: &[u64],
) -> libredox::error::Result<usize> {
    libredox::error::Error::demux(redox_sys_call_v0(
        fd,
        payload as *mut T as *mut u8,
        std::mem::size_of::<T>(),
        flags,
        metadata.as_ptr(),
        metadata.len(),
    ))
}

// TODO: Copied from drivers repo, so:
//   a. this function should be moved to libredox
//   b. move the qemu test driver part to drivers repo and let redoxerd communicate with that driver instead
fn acquire_port_io_rights() -> io::Result<()> {
    let kernel_fd =
        syscall::dup(unsafe { redox_cur_thrfd_v0() }, b"open_via_dup").map_err(syscall_error)?;
    let res = unsafe { sys_call::<[u8; 0]>(kernel_fd, &mut [], 0, &[ProcSchemeVerb::Iopl as u64]) };
    let _ = syscall::close(kernel_fd);
    res?;
    Ok(())
}

event::user_data! {
    enum EventData {
        Pty,
        Timer,
    }
}

fn handle(
    event_queue: event::EventQueue<EventData>,
    master_fd: RawFd,
    timeout_fd: RawFd,
    process: &mut Child,
) -> io::Result<ExitStatus> {
    let handle_event = |event: EventData| -> io::Result<bool> {
        match event {
            EventData::Pty => {
                let mut packet = [0; 4096];
                loop {
                    // Read data from PTY master
                    let count = match syscall::read(master_fd as usize, &mut packet) {
                        Ok(0) => return Ok(false),
                        Ok(count) => count,
                        Err(ref err) if err.errno == syscall::EAGAIN => return Ok(true),
                        Err(err) => return Err(syscall_error(err)),
                    };

                    // Write data to stdout
                    syscall::write(1, &packet[1..count]).map_err(syscall_error)?;

                    for i in 1..count {
                        // Write byte to QEMU debugcon (Bochs compatible)
                        Pio::<u8>::new(0xe9).write(packet[i]);
                    }
                }
            }
            EventData::Timer => {
                let mut timespec = syscall::TimeSpec::default();
                syscall::read(timeout_fd as usize, &mut timespec).map_err(syscall_error)?;

                timespec.tv_sec += 1;
                syscall::write(timeout_fd as usize, &mut timespec).map_err(syscall_error)?;

                Ok(true)
            }
        }
    };

    if handle_event(EventData::Pty)? && handle_event(EventData::Timer)? {
        'events: loop {
            match process.try_wait() {
                Ok(status_opt) => match status_opt {
                    Some(status) => return Ok(status),
                    None => (),
                },
                Err(err) => match err.kind() {
                    io::ErrorKind::WouldBlock => (),
                    _ => return Err(err),
                },
            }

            let event = event_queue.next_event()?;
            if !handle_event(event.user_data)? {
                break 'events;
            }
        }
    }

    let _ = process.kill();
    process.wait()
}

fn getpty(columns: u32, lines: u32) -> io::Result<(RawFd, String)> {
    let master = syscall::open(
        "/scheme/pty",
        syscall::O_CLOEXEC | syscall::O_RDWR | syscall::O_CREAT | syscall::O_NONBLOCK,
    )
    .map_err(syscall_error)?;

    if let Ok(winsize_fd) = syscall::dup(master, b"winsize") {
        let _ = syscall::write(
            winsize_fd,
            &redox_termios::Winsize {
                ws_row: lines as u16,
                ws_col: columns as u16,
            },
        );
        let _ = syscall::close(winsize_fd);
    }

    let mut buf: [u8; 4096] = [0; 4096];
    let count = syscall::fpath(master, &mut buf).map_err(syscall_error)?;
    Ok((master as RawFd, unsafe {
        String::from_utf8_unchecked(Vec::from(&buf[..count]))
    }))
}

fn inner() -> anyhow::Result<()> {
    acquire_port_io_rights()?;

    let config = fs::read_to_string("/etc/redoxerd").context("Failed to read /etc/redoxerd")?;
    let mut config_lines = config.lines();

    let (columns, lines) = (DEFAULT_COLS, DEFAULT_LINES);
    let (master_fd, pty) = getpty(columns, lines)?;

    let timeout_fd = syscall::open(
        "/scheme/time/4",
        syscall::O_CLOEXEC | syscall::O_RDWR | syscall::O_NONBLOCK,
    )
    .map_err(syscall_error)? as RawFd;

    let event_queue = event::EventQueue::new()?;
    event_queue.subscribe(master_fd as usize, EventData::Pty, event::EventFlags::READ)?;
    event_queue.subscribe(
        timeout_fd as usize,
        EventData::Timer,
        event::EventFlags::READ,
    )?;

    let slave_stdin = OpenOptions::new().read(true).open(&pty)?;
    let slave_stdout = OpenOptions::new().write(true).open(&pty)?;
    let slave_stderr = OpenOptions::new().write(true).open(&pty)?;

    let Some(name) = config_lines.next() else {
        anyhow::bail!("/etc/redoxerd does not specify command");
    };
    let mut command = Command::new(name);
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

    let mut process = command
        .spawn()
        .with_context(|| format!("Failed to spawn {command:?}"))?;
    let status = handle(event_queue, master_fd, timeout_fd, &mut process)
        .with_context(|| format!("Failed to run {name}"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("{name} failed with {}", status);
    }
}

fn main() {
    match inner() {
        Ok(()) => {
            // Exit with success using qemu device
            Pio::<u16>::new(0x604).write(0x2000);
            Pio::<u8>::new(0x501).write(51 / 2);
        }
        Err(err) => {
            eprintln!("redoxerd: {:#}", err);

            // Wait a bit for the error message to get flushed through the tty subsystem before exiting.
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Exit with error using qemu device
            Pio::<u8>::new(0x501).write(53 / 2);
        }
    }
}
