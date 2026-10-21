//! Capture on Windows. The hooks do not touch the socket.
//! A swallowed key never arrives as raw input, so the keyboard hook keeps it.

use crate::ballistics::Ballistics;
use crate::inbox::Store;
use crate::desk::{Desk, Edge, Mon};
use crate::frame::{Frame, VERSION};
use crate::hid;
use crate::machine::{Action, Event, Machine};
use crate::net::{self, Link};
use crate::scale::Scaler;
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, PeekMessageW,
    RegisterClassW, SetCursorPos, SetWindowsHookExW, ShowCursor, SystemParametersInfoW,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, HMENU, HWND_MESSAGE, MONITORINFOF_PRIMARY, MSG,
    PM_REMOVE, QS_ALLINPUT, SPI_GETMOUSESPEED, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE,
    WNDCLASSW, WM_INPUT, WM_QUIT,
};
use windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjects;

const RI_KEY_BREAK: u16 = 0x0001;
const RI_KEY_E0: u16 = 0x0002;
const MOUSE_MOVE_ABSOLUTE: u16 = 0x0001;
const LEFT_DOWN: u16 = 0x0001;
const LEFT_UP: u16 = 0x0002;
const RIGHT_DOWN: u16 = 0x0004;
const RIGHT_UP: u16 = 0x0008;
const MIDDLE_DOWN: u16 = 0x0010;
const MIDDLE_UP: u16 = 0x0020;
const WHEEL: u16 = 0x0400;
const HWHEEL: u16 = 0x0800;
const BIT_LCTRL: u8 = 1 << 0;
const BIT_RCTRL: u8 = 1 << 1;
const BIT_LALT: u8 = 1 << 2;
const BIT_RALT: u8 = 1 << 3;
const BIT_LSHIFT: u8 = 1 << 4;
const BIT_RSHIFT: u8 = 1 << 5;
const BIT_LWIN: u8 = 1 << 6;
const BIT_RWIN: u8 = 1 << 7;
const LLKHF_UP: u32 = 0x80;

static SWALLOW: AtomicBool = AtomicBool::new(false);
static FORCE_LOCAL: AtomicBool = AtomicBool::new(false);
static MODS: AtomicU8 = AtomicU8::new(0);

struct QueuedKey {
    hid: u16,
    down: bool,
}

static KEY_QUEUE: Mutex<Vec<QueuedKey>> = Mutex::new(Vec::new());
static HOOKED: AtomicU64 = AtomicU64::new(0);

struct Dial {
    gen: u64,
    result: io::Result<std::net::TcpStream>,
}

struct Session {
    desk: Desk,
    machine: Machine,
    ballistics: Ballistics,
    scaler: Scaler,
    link: Option<Link>,
    greeted: bool,
    hello_at: Option<Instant>,
    hidden: bool,
    host: String,
    port: u16,
    gen: u64,
    dialing: bool,
    next_dial: Instant,
    tx: Sender<Dial>,
    rx: Receiver<Dial>,
    started: Instant,
    announced: bool,
    settings_path: PathBuf,
    settings_mtime: Option<SystemTime>,
    settings_checked: Instant,
    accepts_config: bool,
    accepts_inbox: bool,
    inbox: Store,
}

pub fn run(
    host: &str,
    port: u16,
    scale: f64,
    edge: Edge,
    settings_path: &Path,
    upgrade: bool,
) -> io::Result<()> {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let desk = monitors().unwrap_or_else(|_| crate::desk::darwin_desk());
    let (tx, rx) = mpsc::channel();
    let settings_mtime = std::fs::metadata(settings_path)
        .and_then(|meta| meta.modified())
        .ok();
    let mut session = Session {
        desk,
        machine: Machine::new(edge),
        ballistics: Ballistics::new(mouse_speed()),
        scaler: Scaler::new(scale),
        link: None,
        greeted: false,
        hello_at: None,
        hidden: false,
        host: host.to_string(),
        port,
        gen: 0,
        dialing: false,
        next_dial: Instant::now(),
        tx,
        rx,
        started: Instant::now(),
        announced: false,
        settings_path: settings_path.to_path_buf(),
        settings_mtime,
        settings_checked: Instant::now(),
        accepts_config: false,
        accepts_inbox: false,
        inbox: Store::attach(crate::inbox::path_for(settings_path)),
    };
    let window = message_window()?;
    register_raw(window)?;
    if upgrade {
        crate::upgrade::watch();
    }
    let mouse_hook = unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), HINSTANCE::default(), 0)
    }
    .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))?;
    let key_hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), HINSTANCE::default(), 0) }
            .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))?;
    let result = event_loop(&mut session);
    unsafe {
        let _ = UnhookWindowsHookEx(mouse_hook);
        let _ = UnhookWindowsHookEx(key_hook);
    }
    show_cursor(&mut session.hidden, false);
    result
}

fn event_loop(session: &mut Session) -> io::Result<()> {
    let mut msg = MSG::default();
    loop {
        unsafe {
            MsgWaitForMultipleObjects(None, false, 4, QS_ALLINPUT);
        }
        loop {
            let pending = unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) };
            if !pending.as_bool() {
                break;
            }
            if msg.message == WM_QUIT {
                return Ok(());
            }
            if msg.message == WM_INPUT {
                take_local(session);
                handle_raw(session, msg.lParam);
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        take_local(session);
        refresh_settings(session);
        poll_dial(session);
        pump(session);
        tick(session);
        if let Some((x, y)) = session.machine.park() {
            unsafe {
                let _ = SetCursorPos(x, y);
            }
        }
    }
}

fn take_local(session: &mut Session) {
    crate::trace::flush();
    if FORCE_LOCAL.swap(false, Ordering::SeqCst) {
        let now = now_ms(session.started);
        enact(session, Event::ForceLocal { now });
    }
    drain_keys(session);
}

fn drain_keys(session: &mut Session) {
    let queued = match KEY_QUEUE.lock() {
        Ok(mut queue) => std::mem::take(&mut *queue),
        Err(err) => std::mem::take(&mut *err.into_inner()),
    };
    if queued.is_empty() {
        return;
    }
    let now = now_ms(session.started);
    for key in queued {
        if crate::trace::on() {
            crate::trace::key(&format!(
                "{} {} hook",
                hid::label(key.hid),
                crate::trace::dir(key.down)
            ));
        }
        enact(
            session,
            Event::Key {
                hid: key.hid,
                down: key.down,
                now,
            },
        );
    }
}

fn tick(session: &mut Session) {
    let handshake_late = !session.greeted
        && session
            .hello_at
            .map(|at| at.elapsed() > Duration::from_secs(2))
            .unwrap_or(false);
    if handshake_late {
        crate::report::submit(
            "handshake-timeout",
            "Hello got no HelloAck within 2 seconds.",
        );
        fail_link(session);
        return;
    }
    let now = now_ms(session.started);
    enact(session, Event::Tick { now });
}

fn pump(session: &mut Session) {
    let incoming = match session.link.as_mut() {
        Some(link) => link.recv(),
        None => return,
    };
    match incoming {
        Ok(frames) => {
            for frame in frames {
                dispatch(session, frame);
            }
        }
        Err(err) => {
            crate::report::submit(
                "session-dropped",
                &format!("The connection closed ({err})."),
            );
            fail_link(session);
        }
    }
}

fn dispatch(session: &mut Session, frame: Frame) {
    let now = now_ms(session.started);
    match frame {
        Frame::HelloAck {
            logical_w,
            logical_h,
            scale_milli,
            accepts_config,
            accepts_inbox,
        } => {
            eprintln!(
                "bezel: mac {logical_w}x{logical_h} scale {:.2}",
                f64::from(scale_milli) / 1000.0
            );
            session.accepts_config = accepts_config;
            session.accepts_inbox = accepts_inbox;
            session.greeted = true;
            let edge = session.machine.edge();
            eprintln!("bezel: edge {}", edge.as_str());
            if accepts_config {
                if !try_send(session, &Frame::Config { edge }) {
                    fail_link(session);
                    return;
                }
            } else {
                eprintln!("bezel: mac cannot take the edge yet");
            }
            if accepts_inbox {
                let hello = session.inbox.greet();
                if !try_send(session, &hello) {
                    fail_link(session);
                    return;
                }
            }
            enact(session, Event::PeerUp { now });
        }
        Frame::InboxHello { .. } | Frame::Note { .. } => {
            if !session.accepts_inbox {
                return;
            }
            let replies = match session.inbox.observe(frame) {
                Ok(frames) => frames,
                Err(_) => return,
            };
            for reply in replies {
                if !try_send(session, &reply) {
                    fail_link(session);
                    return;
                }
            }
        }
        Frame::EnterAck { seq } => enact(session, Event::EnterAck { seq, now }),
        Frame::Leave => enact(session, Event::Leave { now }),
        Frame::Heartbeat => enact(session, Event::Heartbeat { now }),
        _ => {}
    }
}

fn enact(session: &mut Session, event: Event) {
    let actions = session.machine.handle(event, &session.desk);
    let mut dead = false;
    for action in actions {
        match action {
            Action::Send(frame) => {
                let sent = try_send(session, &frame);
                if let Frame::Key { hid, down } = &frame {
                    if crate::trace::on() {
                        let why = if sent { "sent" } else { "not-sent link" };
                        crate::trace::key(&format!(
                            "{} {} {why}",
                            hid::label(*hid),
                            crate::trace::dir(*down)
                        ));
                    }
                }
                if !sent {
                    dead = true;
                }
            }
            Action::HideCursor { x, y } => {
                SWALLOW.store(true, Ordering::SeqCst);
                unsafe {
                    let _ = SetCursorPos(x, y);
                }
                show_cursor(&mut session.hidden, true);
            }
            Action::ShowCursor { x, y } => {
                SWALLOW.store(false, Ordering::SeqCst);
                unsafe {
                    let _ = SetCursorPos(x, y);
                }
                show_cursor(&mut session.hidden, false);
            }
            Action::Note(word) => {
                eprintln!("bezel: {word}");
                if word == "enter-timeout" {
                    crate::report::submit(
                        "enter-timeout",
                        "Enter was sent and no EnterAck arrived within 150ms. The pointer stayed here.",
                    );
                }
            }
        }
    }
    SWALLOW.store(session.machine.swallow(), Ordering::SeqCst);
    if dead {
        fail_link(session);
    }
}

fn try_send(session: &mut Session, frame: &Frame) -> bool {
    let Some(link) = session.link.as_mut() else {
        return false;
    };
    match link.send(frame) {
        Ok(()) => true,
        Err(_) => {
            session.link = None;
            false
        }
    }
}

fn fail_link(session: &mut Session) {
    session.accepts_inbox = false;
    session.inbox.pause();
    let was_up = session.greeted;
    if session.link.is_none() && !was_up && session.hello_at.is_none() {
        return;
    }
    session.gen = session.gen.wrapping_add(1);
    session.link = None;
    session.greeted = false;
    session.accepts_config = false;
    session.accepts_inbox = false;
    session.hello_at = None;
    SWALLOW.store(false, Ordering::SeqCst);
    if !was_up {
        return;
    }
    crate::report::submit("session-dropped", "The connection closed while sending.");
    let now = now_ms(session.started);
    let actions = session.machine.handle(Event::PeerDown { now }, &session.desk);
    for action in actions {
        match action {
            Action::ShowCursor { x, y } => {
                unsafe {
                    let _ = SetCursorPos(x, y);
                }
                show_cursor(&mut session.hidden, false);
            }
            Action::HideCursor { .. } => show_cursor(&mut session.hidden, false),
            Action::Note(word) => eprintln!("bezel: {word}"),
            Action::Send(_) => {}
        }
    }
}

fn poll_dial(session: &mut Session) {
    while let Ok(dial) = session.rx.try_recv() {
        session.dialing = false;
        if dial.gen != session.gen || session.link.is_some() {
            continue;
        }
        if let Ok(stream) = dial.result {
            attach(session, stream);
        }
    }
    if session.link.is_some() || session.dialing || Instant::now() < session.next_dial {
        return;
    }
    if !session.announced {
        eprintln!("bezel: waiting for {}", session.host);
        session.announced = true;
    }
    session.dialing = true;
    session.next_dial = Instant::now() + Duration::from_secs(1);
    let tx = session.tx.clone();
    let host = session.host.clone();
    let port = session.port;
    let gen = session.gen;
    thread::spawn(move || {
        let result = dial(&host, port);
        let _ = tx.send(Dial { gen, result });
    });
}

fn refresh_settings(session: &mut Session) {
    if session.settings_checked.elapsed() < Duration::from_millis(500) {
        return;
    }
    session.settings_checked = Instant::now();
    sync_inbox(session);
    let modified = match std::fs::metadata(&session.settings_path).and_then(|meta| meta.modified()) {
        Ok(modified) => modified,
        Err(_) => return,
    };
    // The file at startup already lost to the flags. A later save wins.
    let Some(seen) = session.settings_mtime else {
        session.settings_mtime = Some(modified);
        return;
    };
    if modified <= seen {
        return;
    }
    let file = match crate::settings::load(&session.settings_path) {
        Ok(file) => file,
        Err(_) => return,
    };
    session.settings_mtime = Some(modified);
    if file.edge != session.machine.edge() {
        session.machine.set_edge(file.edge);
        eprintln!("bezel: edge {}", file.edge.as_str());
        if session.greeted && session.accepts_config {
            let edge = file.edge;
            if !try_send(session, &Frame::Config { edge }) {
                fail_link(session);
            }
        } else if session.greeted {
            eprintln!("bezel: mac cannot take the edge yet");
        }
    }
    let previous = session.scaler.scale();
    session.scaler.set_scale(file.scale);
    if session.scaler.scale() != previous {
        eprintln!("bezel: scale {}", format_scale(session.scaler.scale()));
    }
}

fn sync_inbox(session: &mut Session) {
    let frames = match session.inbox.poll() {
        Ok(frames) => frames,
        Err(_) => return,
    };
    if !session.greeted || !session.accepts_inbox {
        return;
    }
    for frame in frames {
        if !try_send(session, &frame) {
            fail_link(session);
            return;
        }
    }
}

fn format_scale(scale: f64) -> String {
    let text = format!("{scale}");
    if text.contains('.') {
        text
    } else {
        format!("{scale:.1}")
    }
}

fn attach(session: &mut Session, stream: std::net::TcpStream) {
    let mut link = match Link::new(stream) {
        Ok(link) => link,
        Err(_) => return,
    };
    if link.send(&Frame::Hello { version: VERSION }).is_err() {
        return;
    }
    session.link = Some(link);
    session.hello_at = Some(Instant::now());
    session.greeted = false;
}

fn dial(host: &str, port: u16) -> io::Result<std::net::TcpStream> {
    let addrs = net::resolve(host, port)?;
    let addr = net::prefer_tailscale(&addrs).ok_or_else(|| {
        io::Error::new(ErrorKind::NotFound, "no address")
    })?;
    let stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn handle_raw(session: &mut Session, lparam: LPARAM) {
    let Some(input) = read_raw(lparam) else {
        return;
    };
    let now = now_ms(session.started);
    if input.header.dwType == RIM_TYPEMOUSE.0 {
        let mouse = unsafe { input.data.mouse };
        let button_bits = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
        let button_data = unsafe { mouse.Anonymous.Anonymous.usButtonData } as i16;
        if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE == 0 && (mouse.lLastX != 0 || mouse.lLastY != 0) {
            let mut point = POINT::default();
            if unsafe { GetCursorPos(&mut point) }.is_ok() {
                let (dx, dy) = session.ballistics.pixels(mouse.lLastX, mouse.lLastY);
                let (dx, dy) = session.scaler.apply(dx, dy);
                enact(
                    session,
                    Event::Pointer {
                        x: point.x,
                        y: point.y,
                        dx,
                        dy,
                        now,
                    },
                );
            }
        }
        for (bit, button, down) in [
            (LEFT_DOWN, 0u8, true),
            (LEFT_UP, 0, false),
            (RIGHT_DOWN, 1, true),
            (RIGHT_UP, 1, false),
            (MIDDLE_DOWN, 2, true),
            (MIDDLE_UP, 2, false),
        ] {
            if button_bits & bit != 0 {
                enact(
                    session,
                    Event::Button {
                        button,
                        down,
                        now,
                    },
                );
            }
        }
        if button_bits & WHEEL != 0 {
            enact(
                session,
                Event::Wheel {
                    dx: 0,
                    dy: i32::from(button_data) / 120,
                    now,
                },
            );
        }
        if button_bits & HWHEEL != 0 {
            enact(
                session,
                Event::Wheel {
                    dx: i32::from(button_data) / 120,
                    dy: 0,
                    now,
                },
            );
        }
        return;
    }
    if input.header.dwType == RIM_TYPEKEYBOARD.0 {
        let board = unsafe { input.data.keyboard };
        let mut flags = 0u32;
        if board.Flags & RI_KEY_E0 != 0 {
            flags |= 1;
        }
        if board.Flags & RI_KEY_BREAK != 0 {
            flags |= 0x80;
        }
        let Some((hid, down)) = hid::from_hook(u32::from(board.MakeCode), flags, u32::from(board.VKey))
        else {
            if crate::trace::on() {
                crate::trace::key(&format!(
                    "scan {:02X} vk {:02X} raw unmapped",
                    board.MakeCode, board.VKey
                ));
            }
            return;
        };
        if hook_echo(hid, down) {
            return;
        }
        if crate::trace::on() {
            crate::trace::key(&format!(
                "{} {} raw",
                hid::label(hid),
                crate::trace::dir(down)
            ));
        }
        enact(session, Event::Key { hid, down, now });
    }
}

fn read_raw(lparam: LPARAM) -> Option<RAWINPUT> {
    let handle = HRAWINPUT(lparam.0 as *mut std::ffi::c_void);
    let mut size = 0u32;
    let header = size_of::<RAWINPUTHEADER>() as u32;
    unsafe {
        GetRawInputData(handle, RID_INPUT, None, &mut size, header);
    }
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u64; (size as usize + 7) / 8];
    let wrote = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some(buf.as_mut_ptr() as *mut _),
            &mut size,
            header,
        )
    };
    if wrote == 0 || wrote == u32::MAX {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const RAWINPUT) })
}

fn monitors() -> io::Result<Desk> {
    let mut found = Vec::new();
    let data = LPARAM(&mut found as *mut Vec<Mon> as isize);
    let ok = unsafe {
        EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(enum_monitor),
            data,
        )
    };
    if !ok.as_bool() || found.is_empty() {
        return Err(io::Error::new(ErrorKind::NotFound, "no monitors"));
    }
    Ok(Desk::new(found))
}

unsafe extern "system" fn enum_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut windows::Win32::Foundation::RECT,
    data: LPARAM,
) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Mon>);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    if GetMonitorInfoW(monitor, &mut info).as_bool() {
        let rect = info.rcMonitor;
        list.push(Mon {
            x: rect.left,
            y: rect.top,
            w: rect.right - rect.left,
            h: rect.bottom - rect.top,
            primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    BOOL(1)
}

fn message_window() -> io::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null())
            .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("bezel-msg"),
            ..WNDCLASSW::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::new(ErrorKind::Other, "register class"));
        }
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("bezel-msg"),
            w!("bezel"),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            HMENU::default(),
            instance,
            None,
        )
        .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_raw(window: HWND) -> io::Result<()> {
    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window,
        },
    ];
    unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) }
        .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))
}

fn mouse_speed() -> i32 {
    let mut speed = 10i32;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETMOUSESPEED,
            0,
            Some(&mut speed as *mut i32 as *mut _),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() && (1..=20).contains(&speed) {
        speed
    } else {
        10
    }
}

fn show_cursor(hidden: &mut bool, hide: bool) {
    if *hidden == hide {
        return;
    }
    unsafe {
        ShowCursor(!hide);
    }
    *hidden = hide;
}

fn now_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && SWALLOW.load(Ordering::SeqCst) {
        return LRESULT(1);
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let packet = &*(lparam.0 as *const LlKey);
        let down = packet.flags & LLKHF_UP == 0;
        note_mod(packet.vk_code, down);
        let mods = MODS.load(Ordering::SeqCst);
        let win = mods & (BIT_LWIN | BIT_RWIN) != 0;
        let chord = mods & (BIT_LCTRL | BIT_RCTRL) != 0
            && mods & (BIT_LALT | BIT_RALT) != 0
            && mods & (BIT_LSHIFT | BIT_RSHIFT) != 0;
        if win || chord {
            FORCE_LOCAL.store(true, Ordering::SeqCst);
            SWALLOW.store(false, Ordering::SeqCst);
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        }
        if SWALLOW.load(Ordering::SeqCst) && remember_hook_key(packet) {
            return LRESULT(1);
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

fn note_mod(vk: u32, down: bool) {
    let Some(bit) = (match vk {
        0xA2 | 0x11 => Some(BIT_LCTRL),
        0xA3 => Some(BIT_RCTRL),
        0xA4 | 0x12 => Some(BIT_LALT),
        0xA5 => Some(BIT_RALT),
        0xA0 | 0x10 => Some(BIT_LSHIFT),
        0xA1 => Some(BIT_RSHIFT),
        0x5B => Some(BIT_LWIN),
        0x5C => Some(BIT_RWIN),
        _ => None,
    }) else {
        return;
    };
    if down {
        MODS.fetch_or(bit, Ordering::SeqCst);
    } else {
        MODS.fetch_and(!bit, Ordering::SeqCst);
    }
}

fn remember_hook_key(packet: &LlKey) -> bool {
    let Some((hid, down)) = hid::from_hook(packet.scan_code, packet.flags, packet.vk_code) else {
        if crate::trace::on() {
            crate::trace::later(format!(
                "scan {:02X} vk {:02X} {} unmapped",
                packet.scan_code,
                packet.vk_code,
                crate::trace::dir(packet.flags & LLKHF_UP == 0)
            ));
        }
        return false;
    };
    if hid::is_gui(hid) {
        return false;
    }
    let Ok(mut queue) = KEY_QUEUE.lock() else {
        if crate::trace::on() {
            crate::trace::later(format!(
                "{} {} not-sent queue",
                hid::label(hid),
                crate::trace::dir(down)
            ));
        }
        return true;
    };
    if queue.len() >= 128 && down {
        if crate::trace::on() {
            crate::trace::later(format!(
                "{} {} not-sent queue",
                hid::label(hid),
                crate::trace::dir(down)
            ));
        }
        return true;
    }
    queue.push(QueuedKey { hid, down });
    mark_hooked(hid, down);
    true
}

fn mark_hooked(hid: u16, down: bool) {
    let now = unix_ms();
    let packed = (now << 17) | (u64::from(hid) << 1) | u64::from(down);
    HOOKED.store(packed, Ordering::Relaxed);
}

fn hook_echo(hid: u16, down: bool) -> bool {
    if !SWALLOW.load(Ordering::Relaxed) {
        return false;
    }
    let packed = HOOKED.load(Ordering::Relaxed);
    let seen = (packed >> 1) as u16;
    let was_down = packed & 1 == 1;
    let at = packed >> 17;
    seen == hid && was_down == down && unix_ms().saturating_sub(at) < 50
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[repr(C)]
struct LlKey {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra: usize,
}
