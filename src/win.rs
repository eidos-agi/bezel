//! Capture on Windows. The low-level hooks only decide whether to swallow.
//! They do not touch the socket. Raw input is read on this thread.

use crate::ballistics::Ballistics;
use crate::desk::{Desk, Mon};
use crate::frame::{Frame, VERSION};
use crate::hid;
use crate::machine::{Action, Event, Machine};
use crate::net::{self, Link};
use crate::scale::Scaler;
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY};
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
const VK_SHIFT: VIRTUAL_KEY = VIRTUAL_KEY(0x10);
const VK_CONTROL: VIRTUAL_KEY = VIRTUAL_KEY(0x11);
const VK_MENU: VIRTUAL_KEY = VIRTUAL_KEY(0x12);
const VK_LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5B);
const VK_RWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5C);

static SWALLOW: AtomicBool = AtomicBool::new(false);
static FORCE_LOCAL: AtomicBool = AtomicBool::new(false);

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
}

pub fn run(host: &str, port: u16, scale: f64) -> io::Result<()> {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let desk = monitors().unwrap_or_else(|_| crate::desk::darwin_desk());
    let (tx, rx) = mpsc::channel();
    let mut session = Session {
        desk,
        machine: Machine::new(),
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
    };
    let window = message_window()?;
    register_raw(window)?;
    let mouse_hook = unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), HINSTANCE::default(), 0)
    }
    .map_err(|err| io::Error::new(ErrorKind::Other, err.to_string()))?;
    let key_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), HINSTANCE::default(), 0) }
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
    if FORCE_LOCAL.swap(false, Ordering::SeqCst) {
        let now = now_ms(session.started);
        enact(session, Event::ForceLocal { now });
    }
}

fn tick(session: &mut Session) {
    let handshake_late = !session.greeted
        && session
            .hello_at
            .map(|at| at.elapsed() > Duration::from_secs(2))
            .unwrap_or(false);
    if handshake_late {
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
        Err(_) => fail_link(session),
    }
}

fn dispatch(session: &mut Session, frame: Frame) {
    let now = now_ms(session.started);
    match frame {
        Frame::HelloAck {
            logical_w,
            logical_h,
            scale_milli,
        } => {
            eprintln!(
                "bezel: mac {logical_w}x{logical_h} scale {:.2}",
                f64::from(scale_milli) / 1000.0
            );
            session.greeted = true;
            enact(session, Event::PeerUp { now });
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
                if !try_send(session, &frame) {
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
            Action::Note(word) => eprintln!("bezel: {word}"),
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
    let was_up = session.greeted;
    if session.link.is_none() && !was_up && session.hello_at.is_none() {
        return;
    }
    session.gen = session.gen.wrapping_add(1);
    session.link = None;
    session.greeted = false;
    session.hello_at = None;
    SWALLOW.store(false, Ordering::SeqCst);
    if !was_up {
        return;
    }
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
        let extended = board.Flags & RI_KEY_E0 != 0;
        let down = board.Flags & RI_KEY_BREAK == 0;
        let Some(hid) = hid::from_set1(board.MakeCode, extended) else {
            return;
        };
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
        let win = key_down(VK_LWIN) || key_down(VK_RWIN);
        let chord = key_down(VK_CONTROL) && key_down(VK_MENU) && key_down(VK_SHIFT);
        if win || chord {
            FORCE_LOCAL.store(true, Ordering::SeqCst);
            SWALLOW.store(false, Ordering::SeqCst);
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        }
        if SWALLOW.load(Ordering::SeqCst) {
            return LRESULT(1);
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk.0)) as u16 & 0x8000 != 0 }
}
