// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows (Win32) plugin editor host: same line protocol as the Linux X11 host.

use au_plugin_api::{EditorParent, PluginHost, PluginInstance};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    IsWindow, PeekMessageW, RegisterClassW, SetWindowPos, TranslateMessage, CW_USEDEFAULT, MSG,
    PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_CLOSE, WM_QUIT,
    WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::editor::{escape_field, make_state_line, parse_editor_args};

static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

#[allow(unused_unsafe)]
unsafe extern "system" fn editor_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLOSE {
        CLOSE_REQUESTED.store(true, Ordering::SeqCst);
        0
    } else {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

pub(crate) fn editor_main_windows(args: &[String]) -> Result<(), String> {
    CLOSE_REQUESTED.store(false, Ordering::SeqCst);

    let parsed_args = parse_editor_args(args)?;
    let format = parsed_args.format.clone();
    let path = parsed_args.path.clone();
    let requested_id = parsed_args.id.clone();

    let (descriptor, is_vst3) = match format.to_ascii_lowercase().as_str() {
        "clap" => {
            let descriptors =
                crate::clap::scan_file(&path).map_err(|error| format!("{error:?}"))?;
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.id == requested_id)
                .or_else(|| descriptors.first())
                .cloned()
                .ok_or_else(|| "no plugin descriptors found in module".to_owned())?;
            (descriptor, false)
        }
        "vst3" => {
            let descriptors =
                crate::vst3::scan_file(&path).map_err(|error| format!("{error:?}"))?;
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.id == requested_id)
                .or_else(|| descriptors.first())
                .cloned()
                .ok_or_else(|| "no plugin descriptors found in module".to_owned())?;
            (descriptor, true)
        }
        other => return Err(format!("unsupported plugin format: {other}")),
    };

    let mut plugin: Box<dyn PluginInstance> = if is_vst3 {
        crate::vst3::Vst3Host::new()
            .instantiate(&descriptor)
            .map_err(|error| format!("{error:?}"))?
    } else {
        crate::clap::ClapHost::new()
            .instantiate(&descriptor)
            .map_err(|error| format!("{error:?}"))?
    };

    if !plugin.has_editor() {
        return Err("plugin has no editor".to_owned());
    }

    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class_name_wide: Vec<u16> = "AudacityEditorWindowClass\0".encode_utf16().collect();

    let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
    wc.lpfnWndProc = Some(editor_wndproc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name_wide.as_ptr();

    unsafe {
        RegisterClassW(&wc);
    }

    let (parent_hwnd, embedded) = if let Some(h) = parsed_args.parent_xid {
        let candidate = (h as usize) as HWND;
        if unsafe { IsWindow(candidate) } != 0 {
            (candidate, true)
        } else {
            (std::ptr::null_mut(), false)
        }
    } else {
        (std::ptr::null_mut(), false)
    };

    let title_str = format!("{} - Audacity-RS\0", descriptor.name.replace('\0', " "));
    let title_wide: Vec<u16> = title_str.encode_utf16().collect();

    let (style, parent, win_x, win_y, win_w, win_h, title_ptr) = if embedded {
        (
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            parent_hwnd,
            parsed_args.x,
            parsed_args.y,
            640,
            480,
            std::ptr::null(),
        )
    } else {
        (
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            std::ptr::null_mut(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            title_wide.as_ptr(),
        )
    };

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name_wide.as_ptr(),
            title_ptr,
            style,
            win_x,
            win_y,
            win_w,
            win_h,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        return Err("could not create Windows editor window".to_owned());
    }

    let (width, height) = match plugin.open_editor(EditorParent::Win32(hwnd as usize)) {
        Ok(size) => size,
        Err(error) => {
            unsafe {
                DestroyWindow(hwnd);
            }
            return Err(format!("failed to open plugin editor: {error:?}"));
        }
    };

    if embedded {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                width as i32,
                height as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    } else {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        unsafe {
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
    }

    let (stdin_sender, stdin_receiver) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if stdin_sender.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = stdin_sender.send(None);
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "EMBED\t{}", u8::from(embedded))
        .map_err(|error| format!("failed to report embedding: {error}"))?;
    writeln!(output, "READY\t{width}\t{height}\t{}", hwnd as usize)
        .and_then(|()| output.flush())
        .map_err(|error| format!("failed to report editor readiness: {error}"))?;

    let crash_after = std::env::var("AU_EDITOR_TEST_CRASH_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis);
    let ready_at = Instant::now();
    let mut state_pairs = Vec::<(String, String)>::new();
    let mut last_sent = HashMap::<u32, f32>::new();
    for parameter in &descriptor.params {
        if let Some(value) = plugin.get_param(parameter.id) {
            last_sent.insert(parameter.id, value);
        }
    }

    let mut quitting = false;
    let mut last_param_check = Instant::now();

    while !quitting {
        if is_vst3 {
            crate::vst3::run_loop_idle();
        } else {
            crate::clap::run_loop_idle();
        }

        let mut msg = unsafe { std::mem::zeroed::<MSG>() };
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                quitting = true;
                break;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if CLOSE_REQUESTED.load(Ordering::SeqCst) {
                quitting = true;
                break;
            }
        }

        loop {
            match stdin_receiver.try_recv() {
                Ok(Some(line)) => {
                    let mut parts = line.splitn(3, '\t');
                    match parts.next().unwrap_or_default() {
                        "STATE" => {
                            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                                state_pairs.push((key.to_owned(), value.to_owned()));
                            }
                        }
                        "LOAD" => {
                            if let Err(error) = plugin.load_state(&state_pairs) {
                                writeln!(output, "ERROR\t{}", escape_field(&format!("{error:?}")))
                                    .and_then(|()| output.flush())
                                    .ok();
                            }
                            state_pairs.clear();
                        }
                        "GETSTATE" => match plugin.save_state() {
                            Ok(pairs) => {
                                for (key, value) in pairs {
                                    writeln!(output, "{}", make_state_line(&key, &value)).ok();
                                }
                                writeln!(output, "STATEEND").ok();
                                output.flush().ok();
                            }
                            Err(error) => {
                                writeln!(output, "ERROR\t{}", escape_field(&format!("{error:?}")))
                                    .and_then(|()| output.flush())
                                    .ok();
                            }
                        },
                        "SETPARAM" => {
                            if let (Some(id), Some(value)) = (parts.next(), parts.next()) {
                                if let (Ok(id), Ok(value)) =
                                    (id.parse::<u32>(), value.parse::<f32>())
                                {
                                    if let Err(error) = plugin.set_param(id, value) {
                                        writeln!(
                                            output,
                                            "ERROR\t{}",
                                            escape_field(&format!("{error:?}"))
                                        )
                                        .and_then(|()| output.flush())
                                        .ok();
                                    }
                                }
                            }
                        }
                        "MOVE" => {
                            if let (Some(x), Some(y)) = (parts.next(), parts.next()) {
                                if let (Ok(x), Ok(y)) =
                                    (x.parse::<i32>(), y.trim().parse::<i32>())
                                {
                                    unsafe {
                                        SetWindowPos(
                                            hwnd,
                                            std::ptr::null_mut(),
                                            x,
                                            y,
                                            0,
                                            0,
                                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                                        );
                                    }
                                }
                            }
                        }
                        "CLOSE" => {
                            quitting = true;
                            break;
                        }
                        _ => {}
                    }
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    quitting = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if let Some(delay) = crash_after {
            if ready_at.elapsed() >= delay {
                unsafe {
                    (16usize as *mut i32).write_volatile(1);
                }
            }
        }

        if last_param_check.elapsed() >= Duration::from_millis(100) {
            for parameter in &descriptor.params {
                if let Some(value) = plugin.get_param(parameter.id) {
                    let changed = last_sent
                        .get(&parameter.id)
                        .map(|last| last.to_bits() != value.to_bits())
                        .unwrap_or(true);
                    if changed {
                        writeln!(output, "PARAM\t{}\t{}", parameter.id, value).ok();
                        output.flush().ok();
                        last_sent.insert(parameter.id, value);
                    }
                }
            }
            last_param_check = Instant::now();
        }

        if !quitting {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    plugin.close_editor();
    unsafe {
        DestroyWindow(hwnd);
    }
    writeln!(output, "CLOSED")
        .map_err(|error| format!("failed to report editor closure: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("failed to flush editor closure: {error}"))?;
    Ok(())
}